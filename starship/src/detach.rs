use crate::config::RefreshMode;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

pub fn render_pid_file(pid: u32, binary_path: &Path) -> String {
    format!("pid={pid}\nmarker={MARKER}\nbinary={}\n", binary_path.display())
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

fn spawn_and_record(path: &Path, binary_path: &Path, interval_seconds: u64, config_dir: Option<&Path>) {
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            eprintln!("detach: could not create {}", parent.display());
            return;
        }
    }

    let mut cmd = Command::new(binary_path);
    cmd.arg("--poll-loop")
        .arg(interval_seconds.to_string())
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
            eprintln!("detach: could not spawn poll loop: {e}");
            return;
        }
    };
    if std::fs::write(path, render_pid_file(child.id(), binary_path)).is_err() {
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
    Noop,
}

pub struct ExistingProcess {
    pub is_ours: bool,
    pub alive: bool,
    pub binary_exists: bool,
}

/// Reconciles the detached poll process with the requested refresh mode.
pub fn decide(refresh: RefreshMode, existing: Option<&ExistingProcess>) -> Action {
    match existing {
        Some(p) if !p.is_ours => Action::Noop,
        Some(p) if !p.binary_exists => Action::Remove,
        Some(p) if p.alive && refresh != RefreshMode::Poll => Action::Remove,
        Some(p) if !p.alive && refresh == RefreshMode::Poll => Action::Install,
        None if refresh == RefreshMode::Poll => Action::Install,
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
        }
    });

    match decide(refresh, existing.as_ref()) {
        Action::Install => {
            let config_dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR").map(PathBuf::from);
            spawn_and_record(&path, binary_path, interval_seconds, config_dir.as_deref());
        }
        Action::Remove => kill_and_delete(&path),
        Action::Noop => {}
    }

    if let RefreshMode::Hook = refresh {
        eprintln!(r#"refresh: "hook" not implemented yet, falling back to basic"#);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_pid_file_includes_pid_marker_and_binary() {
        let result = render_pid_file(1234, Path::new("/usr/local/bin/herdr-starship"));

        assert!(result.contains("pid=1234"));
        assert!(result.contains("marker=HerdrStarshipPollDetached"));
        assert!(result.contains("binary=/usr/local/bin/herdr-starship"));
    }

    #[test]
    fn is_our_pid_file_true_for_our_own_rendered_output() {
        let contents = render_pid_file(1234, Path::new("/bin/herdr-starship"));

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
        let contents = render_pid_file(4321, Path::new("/opt/herdr-starship"));

        assert_eq!(extract_recorded_pid(&contents), Some(4321));
    }

    #[test]
    fn extract_recorded_pid_none_when_absent() {
        assert_eq!(extract_recorded_pid("marker=HerdrStarshipPollDetached\n"), None);
    }

    #[test]
    fn extract_recorded_binary_finds_value() {
        let contents = render_pid_file(4321, Path::new("/opt/herdr-starship"));

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
        write_fake_pid_file(&home, &render_pid_file(1, Path::new("/opt/herdr-starship")));

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
        };

        assert_eq!(decide(RefreshMode::Poll, Some(&existing)), Action::Noop);
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
        };

        assert_eq!(decide(RefreshMode::Poll, Some(&existing)), Action::Install);
    }

    #[test]
    fn decide_dead_process_in_non_poll_mode_is_noop() {
        let existing = ExistingProcess {
            is_ours: true,
            alive: false,
            binary_exists: true,
        };

        assert_eq!(decide(RefreshMode::Basic, Some(&existing)), Action::Noop);
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
