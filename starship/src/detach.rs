use crate::config::RefreshMode;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const MARKER: &str = "HerdrStarshipPollDetached";

/// Returns the plugin's poll PID path when `$HOME` is set.
pub fn pid_file_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local/state/herdr/plugins/herdr-starship")
            .join("poll.pid")
    })
}

fn mode_str(mode: RefreshMode) -> &'static str {
    match mode {
        RefreshMode::Basic => "basic",
        RefreshMode::Poll => "poll",
        RefreshMode::Watch => "watch",
    }
}

fn parse_mode_str(s: &str) -> Option<RefreshMode> {
    match s {
        "poll" => Some(RefreshMode::Poll),
        "watch" => Some(RefreshMode::Watch),
        _ => None,
    }
}

pub fn render_pid_file(pid: u32, binary_path: &Path, mode: RefreshMode) -> String {
    format!(
        "pid={pid}\nmarker={MARKER}\nbinary={}\nmode={}\n",
        binary_path.display(),
        mode_str(mode)
    )
}

pub fn is_our_pid_file(contents: &str) -> bool {
    let marker_line = format!("marker={MARKER}");
    contents.lines().any(|line| line == marker_line)
}

fn extract_field<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    contents.lines().find_map(|line| line.strip_prefix(prefix.as_str()))
}

fn extract_recorded_pid(contents: &str) -> Option<u32> {
    extract_field(contents, "pid").and_then(|s| s.parse().ok())
}

fn extract_recorded_binary(contents: &str) -> Option<String> {
    extract_field(contents, "binary").map(str::to_string)
}

/// An older pid file lacks this field and reads as `None`. `decide` treats that as a mismatch.
fn extract_recorded_mode(contents: &str) -> Option<RefreshMode> {
    extract_field(contents, "mode").and_then(parse_mode_str)
}

pub fn installed_binary_path() -> Option<PathBuf> {
    let path = pid_file_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    if !is_our_pid_file(&contents) {
        return None;
    }
    extract_recorded_binary(&contents).map(PathBuf::from)
}

/// Verifies that the PID belongs to the expected binary.
fn is_alive_and_ours(pid: u32, expected_binary: &Path) -> bool {
    let output = match Command::new("ps").args(["-p", &pid.to_string(), "-o", "args="]).output() {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    let args = String::from_utf8_lossy(&output.stdout);
    let args = args.trim();
    !args.is_empty() && args.contains(&expected_binary.display().to_string())
}

fn spawn_and_record(
    path: &Path,
    binary_path: &Path,
    refresh: RefreshMode,
    interval_seconds: u64,
    config_dir: Option<&Path>,
) {
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            eprintln!("detach: could not create {}", parent.display());
            return;
        }
    }

    let mut cmd = Command::new(binary_path);
    match refresh {
        RefreshMode::Poll => {
            cmd.arg("--poll-loop").arg(interval_seconds.to_string());
        }
        RefreshMode::Watch => {
            cmd.arg("--watch-loop");
        }
        RefreshMode::Basic => {
            eprintln!("detach: spawn_and_record called with refresh=Basic, nothing to run");
            return;
        }
    }
    cmd
        // New process group, in case a hook timeout ever signals by process group.
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = config_dir {
        cmd.env("HERDR_PLUGIN_CONFIG_DIR", dir);
    }

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            eprintln!("detach: could not spawn {} loop: {e}", mode_str(refresh));
            return;
        }
    };
    if std::fs::write(path, render_pid_file(child.id(), binary_path, refresh)).is_err() {
        eprintln!("detach: could not write {}", path.display());
    }
    // Not waited on, so it can outlive this process. `Child::drop` does not kill it.
}

/// Re-checks `is_our_pid_file` here, before it signals or deletes anything.
fn kill_and_delete(path: &Path) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    if !is_our_pid_file(&contents) {
        return;
    }
    if let Some(pid) = extract_recorded_pid(&contents) {
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).output();
    }
    let _ = std::fs::remove_file(path);
}

/// No signal, only delete: the loop calls this on itself right before it exits.
fn remove_if_ours(path: &Path) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    if !is_our_pid_file(&contents) {
        return;
    }
    let _ = std::fs::remove_file(path);
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Remove,
    /// If alive in the other mode (`poll` or `watch`), kill and respawn in one call.
    Switch,
    Noop,
}

pub struct ExistingProcess {
    pub is_ours: bool,
    pub alive: bool,
    pub binary_exists: bool,
    pub mode: Option<RefreshMode>,
}

fn wants_background_process(refresh: RefreshMode) -> bool {
    matches!(refresh, RefreshMode::Poll | RefreshMode::Watch)
}

/// Reconciles the background process with the requested `poll` or `watch` mode.
pub fn decide(refresh: RefreshMode, existing: Option<&ExistingProcess>) -> Action {
    match existing {
        Some(p) if !p.is_ours => Action::Noop,
        Some(p) if !p.binary_exists => Action::Remove,
        Some(p) if p.alive && !wants_background_process(refresh) => Action::Remove,
        Some(p) if p.alive && p.mode != Some(refresh) => Action::Switch,
        Some(p) if !p.alive && wants_background_process(refresh) => Action::Install,
        None if wants_background_process(refresh) => Action::Install,
        _ => Action::Noop,
    }
}

pub fn reconcile(refresh: RefreshMode, binary_path: &Path, interval_seconds: u64) {
    let Some(path) = pid_file_path() else {
        eprintln!("detach: $HOME not set, skipping reconcile");
        return;
    };
    let existing = std::fs::read_to_string(&path).ok().map(|contents| {
        let is_ours = is_our_pid_file(&contents);
        let binary_exists = extract_recorded_binary(&contents).is_some_and(|p| Path::new(&p).exists());
        let alive = is_ours
            && extract_recorded_pid(&contents)
                .zip(extract_recorded_binary(&contents))
                .is_some_and(|(pid, binary)| is_alive_and_ours(pid, Path::new(&binary)));
        ExistingProcess {
            is_ours,
            alive,
            binary_exists,
            mode: extract_recorded_mode(&contents),
        }
    });

    let config_dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR").map(PathBuf::from);
    match decide(refresh, existing.as_ref()) {
        Action::Install => {
            spawn_and_record(&path, binary_path, refresh, interval_seconds, config_dir.as_deref());
        }
        Action::Remove => kill_and_delete(&path),
        Action::Switch => {
            kill_and_delete(&path);
            spawn_and_record(&path, binary_path, refresh, interval_seconds, config_dir.as_deref());
        }
        Action::Noop => {}
    }
}

/// Herdr has no plugin-uninstall hook; this is the only way an uninstall is detected.
pub fn self_check_and_teardown(binary_path: &Path) -> bool {
    if binary_path.exists() {
        return false;
    }
    if let Some(path) = pid_file_path() {
        remove_if_ours(&path);
    }
    true
}

/// Returns the plugin's refresh-lock path when `$HOME` is set.
pub fn refresh_lock_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local/state/herdr/plugins/herdr-starship")
            .join("refresh.lock")
    })
}

fn pid_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|o| o.status.success())
}

const LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

/// Serializes refreshes across ticks and pushes, and reclaims locks left by crashed processes.
pub fn with_refresh_lock<T>(f: impl FnOnce() -> T) -> T {
    let Some(path) = refresh_lock_path() else {
        return f();
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "pid={}", std::process::id());
                break;
            }
            Err(_) => {
                let holder_dead = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|c| extract_field(&c, "pid").and_then(|s| s.parse::<u32>().ok()))
                    .is_some_and(|pid| !pid_is_alive(pid));
                if holder_dead {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                std::thread::sleep(LOCK_RETRY_DELAY);
            }
        }
    }

    let result = f();
    let _ = std::fs::remove_file(&path);
    result
}

/// Runs one long-lived polling loop.
pub fn run_loop(interval_seconds: u64) {
    loop {
        std::thread::sleep(Duration::from_secs(interval_seconds));
        let self_path = installed_binary_path().or_else(|| std::env::current_exe().ok());
        if let Some(path) = &self_path {
            if self_check_and_teardown(path) {
                return;
            }
        }
        crate::tick();
    }
}

/// The paths `watch` mode watches for a single repo.
#[derive(Debug, PartialEq)]
pub struct GitPaths {
    pub head: PathBuf,
    pub index: PathBuf,
    pub refs: PathBuf,
}

fn rev_parse(repo: &Path, arg: &str) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("rev-parse")
        .arg(arg)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let path = Path::new(text.trim());
    if path.as_os_str().is_empty() {
        return None;
    }
    Some(if path.is_absolute() { path.to_path_buf() } else { repo.join(path) })
}

/// A linked worktree's `.git` is a file. Its `HEAD`/`index` use `--git-dir`. `refs/` is shared.
pub fn resolve_git_paths(repo: &Path) -> Option<GitPaths> {
    let git_dir = rev_parse(repo, "--git-dir")?;
    let common_dir = rev_parse(repo, "--git-common-dir")?;
    Some(GitPaths {
        head: git_dir.join("HEAD"),
        index: git_dir.join("index"),
        refs: common_dir.join("refs"),
    })
}

/// Non-ignored directories under `repo`, skipping trees like `target/` so filtering runs once.
fn working_tree_dirs(repo: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(repo)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|ft| ft.is_dir()))
        .map(ignore::DirEntry::into_path)
        .collect()
}

/// Paths `watch` mode watches. Only `refs/` is `Recursive`, so nested ignored dirs stay hidden.
fn desired_watch_paths(targets: &[(String, PathBuf)]) -> HashMap<PathBuf, RecursiveMode> {
    let mut paths = HashMap::new();
    for (_, repo) in targets {
        let Some(git_paths) = resolve_git_paths(repo) else {
            continue;
        };
        for (path, mode) in [
            (git_paths.head, RecursiveMode::NonRecursive),
            (git_paths.index, RecursiveMode::NonRecursive),
            (git_paths.refs, RecursiveMode::Recursive),
        ] {
            if path.exists() {
                paths.insert(path, mode);
            }
        }
        for dir in working_tree_dirs(repo) {
            paths.insert(dir, RecursiveMode::NonRecursive);
        }
    }
    paths
}

/// Pure diff so the resync logic is testable without a real `notify::Watcher`.
fn diff_watch_paths(
    current: &HashSet<PathBuf>,
    desired: &HashMap<PathBuf, RecursiveMode>,
) -> (Vec<(PathBuf, RecursiveMode)>, Vec<PathBuf>) {
    let to_add = desired
        .iter()
        .filter(|(path, _)| !current.contains(*path))
        .map(|(path, mode)| (path.clone(), *mode))
        .collect();
    let to_remove = current.iter().filter(|path| !desired.contains_key(*path)).cloned().collect();
    (to_add, to_remove)
}

/// Rebuilds the watch set from open workspaces and applies the delta. `tick()` already makes this call.
fn sync_watcher(watcher: &mut RecommendedWatcher, watched: &mut HashSet<PathBuf>, targets: &[(String, PathBuf)]) {
    let desired = desired_watch_paths(targets);
    let (to_add, to_remove) = diff_watch_paths(watched, &desired);

    for (path, mode) in to_add {
        if let Err(e) = watcher.watch(&path, mode) {
            eprintln!("watch: could not watch {}: {e}", path.display());
            continue;
        }
        watched.insert(path);
    }
    for path in to_remove {
        let _ = watcher.unwatch(&path);
        watched.remove(&path);
    }
}

const WATCH_RESYNC_INTERVAL: Duration = Duration::from_secs(30);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(200);

/// One long fs-event loop. Resyncs on `WATCH_RESYNC_INTERVAL`, so new workspaces need no restart.
pub fn run_watch() {
    let (tx, rx) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("watch: could not create fs watcher: {e}");
            return;
        }
    };
    let mut watched = HashSet::new();

    loop {
        sync_watcher(&mut watcher, &mut watched, &crate::list_workspace_targets());

        match rx.recv_timeout(WATCH_RESYNC_INTERVAL) {
            Ok(_) => {
                // A `git commit` touches HEAD/index/refs together. Drain the burst into one tick.
                while rx.recv_timeout(WATCH_DEBOUNCE).is_ok() {}

                let self_path = installed_binary_path().or_else(|| std::env::current_exe().ok());
                if let Some(path) = &self_path {
                    if self_check_and_teardown(path) {
                        return;
                    }
                }
                crate::tick();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        let run = |args: &[&str]| {
            let status = Command::new("git").arg("-C").arg(dir).args(args).status().unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "test"]);
        run(&["commit", "--allow-empty", "-q", "-m", "init"]);
    }

    #[test]
    fn resolve_git_paths_plain_repo_uses_dot_git_for_everything() {
        let repo = std::env::temp_dir().join("herdr-starship-test-gitpaths-plain");
        let _ = std::fs::remove_dir_all(&repo);
        init_repo(&repo);

        let result = resolve_git_paths(&repo).unwrap();

        std::fs::remove_dir_all(&repo).unwrap();
        assert_eq!(result.head, repo.join(".git/HEAD"));
        assert_eq!(result.index, repo.join(".git/index"));
        assert_eq!(result.refs, repo.join(".git/refs"));
    }

    #[test]
    fn resolve_git_paths_linked_worktree_splits_head_from_shared_refs() {
        let base = std::env::temp_dir().join("herdr-starship-test-gitpaths-worktree");
        let main_repo = base.join("main");
        let worktree = base.join("wt");
        let _ = std::fs::remove_dir_all(&base);
        init_repo(&main_repo);
        let status = Command::new("git")
            .arg("-C")
            .arg(&main_repo)
            .args(["worktree", "add", "-q", "-b", "wt-branch"])
            .arg(&worktree)
            .status()
            .unwrap();
        assert!(status.success(), "git worktree add failed");

        let result = resolve_git_paths(&worktree).unwrap();
        // macOS's temp dir is a symlink (`/var` to `/private/var`) that git output already resolves.
        let main_repo = std::fs::canonicalize(&main_repo).unwrap();

        std::fs::remove_dir_all(&base).unwrap();
        assert!(
            result.head.starts_with(main_repo.join(".git/worktrees")),
            "expected a per-worktree HEAD, got {:?}",
            result.head
        );
        assert_eq!(result.refs, main_repo.join(".git/refs"));
    }

    #[test]
    fn resolve_git_paths_none_when_not_a_repo() {
        let dir = std::env::temp_dir().join("herdr-starship-test-gitpaths-not-a-repo");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let result = resolve_git_paths(&dir);

        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn working_tree_dirs_includes_repo_root_and_tracked_subdirs() {
        let repo = std::env::temp_dir().join("herdr-starship-test-working-tree-dirs-plain");
        let _ = std::fs::remove_dir_all(&repo);
        init_repo(&repo);
        std::fs::create_dir_all(repo.join("src/nested")).unwrap();

        let result = working_tree_dirs(&repo);

        std::fs::remove_dir_all(&repo).unwrap();
        assert!(result.contains(&repo), "expected repo root present");
        assert!(result.iter().any(|p| p.ends_with("src")));
        assert!(result.iter().any(|p| p.ends_with("src/nested")));
    }

    /// This test is the point: a huge ignored tree like `target/` must never be walked.
    #[test]
    fn working_tree_dirs_skips_gitignored_directories() {
        let repo = std::env::temp_dir().join("herdr-starship-test-working-tree-dirs-gitignore");
        let _ = std::fs::remove_dir_all(&repo);
        init_repo(&repo);
        std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(repo.join("target/debug/deps")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();

        let result = working_tree_dirs(&repo);

        std::fs::remove_dir_all(&repo).unwrap();
        assert!(result.iter().any(|p| p.ends_with("src")));
        assert!(!result.iter().any(|p| p.ends_with("target")));
        assert!(!result.iter().any(|p| p.ends_with("target/debug/deps")));
    }

    /// `.git` is watched separately, at finer granularity; it must not also show up here.
    #[test]
    fn working_tree_dirs_skips_dot_git() {
        let repo = std::env::temp_dir().join("herdr-starship-test-working-tree-dirs-dotgit");
        let _ = std::fs::remove_dir_all(&repo);
        init_repo(&repo);

        let result = working_tree_dirs(&repo);

        std::fs::remove_dir_all(&repo).unwrap();
        assert!(!result.iter().any(|p| p.ends_with(".git")));
        assert!(!result.iter().any(|p| p.to_string_lossy().contains(".git/")));
    }

    #[test]
    fn desired_watch_paths_includes_head_index_refs_for_a_real_repo() {
        let repo = std::env::temp_dir().join("herdr-starship-test-desired-watch-paths-repo");
        let _ = std::fs::remove_dir_all(&repo);
        init_repo(&repo);
        let targets = vec![("w1".to_string(), repo.clone())];

        let result = desired_watch_paths(&targets);

        std::fs::remove_dir_all(&repo).unwrap();
        assert_eq!(result.get(&repo.join(".git/HEAD")), Some(&RecursiveMode::NonRecursive));
        assert_eq!(result.get(&repo.join(".git/index")), Some(&RecursiveMode::NonRecursive));
        assert_eq!(result.get(&repo.join(".git/refs")), Some(&RecursiveMode::Recursive));
        assert_eq!(result.get(&repo), Some(&RecursiveMode::NonRecursive));
    }

    /// A workspace whose target isn't a git repo contributes nothing, silently.
    #[test]
    fn desired_watch_paths_skips_non_repo_targets() {
        let dir = std::env::temp_dir().join("herdr-starship-test-desired-watch-paths-not-a-repo");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let targets = vec![("w1".to_string(), dir.clone())];

        let result = desired_watch_paths(&targets);

        std::fs::remove_dir_all(&dir).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn diff_watch_paths_computes_additions_and_removals() {
        let keep = PathBuf::from("/repo/.git/HEAD");
        let stale = PathBuf::from("/repo/.git/index");
        let new = PathBuf::from("/repo/.git/refs");
        let current = HashSet::from([keep.clone(), stale.clone()]);
        let desired = HashMap::from([
            (keep, RecursiveMode::NonRecursive),
            (new.clone(), RecursiveMode::Recursive),
        ]);

        let (to_add, to_remove) = diff_watch_paths(&current, &desired);

        assert_eq!(to_add, vec![(new, RecursiveMode::Recursive)]);
        assert_eq!(to_remove, vec![stale]);
    }

    #[test]
    fn diff_watch_paths_no_change_when_sets_match() {
        let path = PathBuf::from("/repo/.git/HEAD");
        let current = HashSet::from([path.clone()]);
        let desired = HashMap::from([(path, RecursiveMode::NonRecursive)]);

        let (to_add, to_remove) = diff_watch_paths(&current, &desired);

        assert!(to_add.is_empty());
        assert!(to_remove.is_empty());
    }

    /// Uses a real `notify::Watcher` to check `sync_watcher` on new and closed workspaces.
    #[test]
    fn sync_watcher_adds_and_removes_real_watches_as_targets_change() {
        let repo = std::env::temp_dir().join("herdr-starship-test-sync-watcher-repo");
        let _ = std::fs::remove_dir_all(&repo);
        init_repo(&repo);
        let (tx, _rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .unwrap();
        let mut watched = HashSet::new();

        sync_watcher(&mut watcher, &mut watched, &[("w1".to_string(), repo.clone())]);
        assert!(watched.contains(&repo.join(".git/HEAD")));

        sync_watcher(&mut watcher, &mut watched, &[]);

        std::fs::remove_dir_all(&repo).unwrap();
        assert!(watched.is_empty());
    }

    /// End-to-end: a real `git checkout` rewrites `HEAD` and fires a real `notify` event.
    #[test]
    fn notify_watcher_fires_when_a_real_checkout_rewrites_head() {
        let repo = std::env::temp_dir().join("herdr-starship-test-watch-e2e-repo");
        let _ = std::fs::remove_dir_all(&repo);
        init_repo(&repo);
        let git_paths = resolve_git_paths(&repo).unwrap();

        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .unwrap();
        watcher.watch(&git_paths.head, RecursiveMode::NonRecursive).unwrap();

        let status = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["checkout", "-q", "-b", "other-branch"])
            .status()
            .unwrap();
        assert!(status.success(), "git checkout failed");

        let event = rx.recv_timeout(Duration::from_secs(5));

        std::fs::remove_dir_all(&repo).unwrap();
        assert!(event.is_ok(), "expected a fs event after checkout rewrote HEAD");
    }

    /// Confirms the gap plain `.git` watching can't cover: a brand new untracked file.
    #[test]
    fn sync_watcher_fires_when_an_untracked_file_is_created_in_the_working_tree() {
        let repo = std::env::temp_dir().join("herdr-starship-test-watch-e2e-untracked-repo");
        let _ = std::fs::remove_dir_all(&repo);
        init_repo(&repo);
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .unwrap();
        let mut watched = HashSet::new();
        sync_watcher(&mut watcher, &mut watched, &[("w1".to_string(), repo.clone())]);

        std::fs::write(repo.join("untracked.txt"), "hi").unwrap();
        let event = rx.recv_timeout(Duration::from_secs(5));

        std::fs::remove_dir_all(&repo).unwrap();
        assert!(event.is_ok(), "expected a fs event after creating an untracked file");
    }

    #[test]
    fn render_pid_file_includes_pid_marker_and_binary() {
        let result = render_pid_file(1234, Path::new("/usr/local/bin/herdr-starship"), RefreshMode::Poll);

        assert!(result.contains("pid=1234"));
        assert!(result.contains("marker=HerdrStarshipPollDetached"));
        assert!(result.contains("binary=/usr/local/bin/herdr-starship"));
    }

    #[test]
    fn is_our_pid_file_true_for_our_own_rendered_output() {
        let contents = render_pid_file(1234, Path::new("/bin/herdr-starship"), RefreshMode::Poll);

        assert!(is_our_pid_file(&contents));
    }

    /// Same fields, no marker: not ours.
    #[test]
    fn is_our_pid_file_false_when_marker_missing() {
        let contents = "pid=1234\nbinary=/some/other/tool\n";

        assert!(!is_our_pid_file(contents));
    }

    #[test]
    fn is_our_pid_file_false_for_unrelated_content() {
        assert!(!is_our_pid_file("not a pid file at all"));
    }

    #[test]
    fn extract_recorded_pid_finds_value() {
        let contents = render_pid_file(4321, Path::new("/opt/herdr-starship"), RefreshMode::Poll);

        assert_eq!(extract_recorded_pid(&contents), Some(4321));
    }

    #[test]
    fn extract_recorded_pid_none_when_absent() {
        assert_eq!(extract_recorded_pid("marker=HerdrStarshipPollDetached\n"), None);
    }

    #[test]
    fn extract_recorded_binary_finds_value() {
        let contents = render_pid_file(4321, Path::new("/opt/herdr-starship"), RefreshMode::Poll);

        assert_eq!(
            extract_recorded_binary(&contents),
            Some("/opt/herdr-starship".to_string())
        );
    }

    #[test]
    fn extract_recorded_binary_none_when_absent() {
        assert_eq!(extract_recorded_binary("pid=1\n"), None);
    }

    fn write_fake_pid_file(home: &Path, contents: &str) {
        let state_dir = home.join(".local/state/herdr/plugins/herdr-starship");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("poll.pid"), contents).unwrap();
    }

    fn with_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
        let original = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", home) };
        let result = f();
        unsafe {
            match &original {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        result
    }

    #[test]
    fn installed_binary_path_reads_recorded_path_from_our_own_pid_file() {
        let _guard = crate::starship::ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join("herdr-starship-test-detach-installed-path-home");
        let _ = std::fs::remove_dir_all(&home);
        write_fake_pid_file(&home, &render_pid_file(1, Path::new("/opt/herdr-starship"), RefreshMode::Poll));

        let result = with_home(&home, installed_binary_path);

        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(result, Some(PathBuf::from("/opt/herdr-starship")));
    }

    #[test]
    fn installed_binary_path_none_when_pid_file_not_ours() {
        let _guard = crate::starship::ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join("herdr-starship-test-detach-installed-path-not-ours-home");
        let _ = std::fs::remove_dir_all(&home);
        write_fake_pid_file(&home, "pid=1\nbinary=/opt/herdr-starship\n");

        let result = with_home(&home, installed_binary_path);

        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn installed_binary_path_none_when_no_pid_file_exists() {
        let _guard = crate::starship::ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join("herdr-starship-test-detach-installed-path-missing");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        let result = with_home(&home, installed_binary_path);

        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn decide_poll_and_nothing_installed_installs() {
        let result = decide(RefreshMode::Poll, None);

        assert_eq!(result, Action::Install);
    }

    #[test]
    fn decide_basic_and_our_process_alive_removes() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: true,
            binary_exists: true,
            mode: Some(RefreshMode::Poll),
        };
        let result = decide(RefreshMode::Basic, Some(&existing));

        assert_eq!(result, Action::Remove);
    }

    /// A file that fails `is_ours` must never be touched, whatever `refresh` is.
    #[test]
    fn decide_unrelated_pid_file_is_never_touched() {
        let existing = ExistingProcess {
            is_ours: false,
            alive: true,
            binary_exists: true,
            mode: None,
        };

        assert_eq!(decide(RefreshMode::Poll, Some(&existing)), Action::Noop);
        assert_eq!(decide(RefreshMode::Basic, Some(&existing)), Action::Noop);
    }

    #[test]
    fn decide_stale_binary_tears_down_regardless_of_refresh_mode() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: true,
            binary_exists: false,
            mode: Some(RefreshMode::Poll),
        };

        assert_eq!(decide(RefreshMode::Poll, Some(&existing)), Action::Remove);
        assert_eq!(decide(RefreshMode::Basic, Some(&existing)), Action::Remove);
    }

    #[test]
    fn decide_poll_already_alive_is_noop() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: true,
            binary_exists: true,
            mode: Some(RefreshMode::Poll),
        };

        assert_eq!(decide(RefreshMode::Poll, Some(&existing)), Action::Noop);
    }

    #[test]
    fn decide_watch_already_alive_is_noop() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: true,
            binary_exists: true,
            mode: Some(RefreshMode::Watch),
        };

        assert_eq!(decide(RefreshMode::Watch, Some(&existing)), Action::Noop);
    }

    #[test]
    fn decide_basic_and_nothing_installed_is_noop() {
        assert_eq!(decide(RefreshMode::Basic, None), Action::Noop);
    }

    /// Unlike `launchd`, nothing here restarts a crashed loop on its own.
    #[test]
    fn decide_dead_process_in_poll_mode_reinstalls() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: false,
            binary_exists: true,
            mode: Some(RefreshMode::Poll),
        };

        assert_eq!(decide(RefreshMode::Poll, Some(&existing)), Action::Install);
    }

    #[test]
    fn decide_dead_process_in_watch_mode_reinstalls() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: false,
            binary_exists: true,
            mode: Some(RefreshMode::Watch),
        };

        assert_eq!(decide(RefreshMode::Watch, Some(&existing)), Action::Install);
    }

    #[test]
    fn decide_dead_process_in_non_poll_mode_is_noop() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: false,
            binary_exists: true,
            mode: Some(RefreshMode::Poll),
        };

        assert_eq!(decide(RefreshMode::Basic, Some(&existing)), Action::Noop);
    }

    #[test]
    fn decide_watch_and_nothing_installed_installs() {
        assert_eq!(decide(RefreshMode::Watch, None), Action::Install);
    }

    /// A mode switch must finish in one `reconcile` call, not Remove-then-Install over two restarts.
    #[test]
    fn decide_switches_from_poll_to_watch_while_alive() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: true,
            binary_exists: true,
            mode: Some(RefreshMode::Poll),
        };

        assert_eq!(decide(RefreshMode::Watch, Some(&existing)), Action::Switch);
    }

    #[test]
    fn decide_switches_from_watch_to_poll_while_alive() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: true,
            binary_exists: true,
            mode: Some(RefreshMode::Watch),
        };

        assert_eq!(decide(RefreshMode::Poll, Some(&existing)), Action::Switch);
    }

    /// An older pid file without `mode` reads as a mismatch and self-heals on the next reconcile.
    #[test]
    fn decide_switches_when_recorded_mode_unknown() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: true,
            binary_exists: true,
            mode: None,
        };

        assert_eq!(decide(RefreshMode::Poll, Some(&existing)), Action::Switch);
    }

    #[test]
    fn refresh_lock_path_uses_home_state_dir() {
        let _guard = crate::starship::ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join("herdr-starship-test-detach-refresh-lock-path-home");

        let result = with_home(&home, refresh_lock_path);

        assert_eq!(
            result,
            Some(home.join(".local/state/herdr/plugins/herdr-starship/refresh.lock"))
        );
    }

    /// Ensures overlapping refreshes run serially.
    #[test]
    fn with_refresh_lock_serializes_overlapping_calls() {
        let _guard = crate::starship::ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join("herdr-starship-test-detach-refresh-lock-serial-home");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
        let log_thread = log.clone();

        with_home(&home, || {
            let handle = std::thread::spawn(move || {
                with_refresh_lock(|| {
                    log_thread.lock().unwrap().push("first-start");
                    std::thread::sleep(Duration::from_millis(150));
                    log_thread.lock().unwrap().push("first-end");
                });
            });

            // Gives the spawned thread time to grab the lock first.
            std::thread::sleep(Duration::from_millis(30));
            with_refresh_lock(|| {
                log.lock().unwrap().push("second-start");
            });
            handle.join().unwrap();
        });

        std::fs::remove_dir_all(&home).unwrap();
        let result = log.lock().unwrap().clone();
        assert_eq!(result, vec!["first-start", "first-end", "second-start"]);
    }

    /// Reclaims a refresh lock left by a dead process.
    #[test]
    fn with_refresh_lock_reclaims_lock_left_by_dead_pid() {
        let _guard = crate::starship::ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join("herdr-starship-test-detach-refresh-lock-stale-home");
        let _ = std::fs::remove_dir_all(&home);
        let state_dir = home.join(".local/state/herdr/plugins/herdr-starship");
        std::fs::create_dir_all(&state_dir).unwrap();

        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        std::fs::write(state_dir.join("refresh.lock"), format!("pid={dead_pid}\n")).unwrap();

        let ran = with_home(&home, || {
            let mut ran = false;
            with_refresh_lock(|| ran = true);
            ran
        });

        std::fs::remove_dir_all(&home).unwrap();
        assert!(ran, "a lock left by a dead pid must be reclaimed, not block forever");
    }
}
