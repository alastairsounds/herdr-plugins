use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(3);
// A login shell with oh-my-zsh, nvm, or p10k starts slower than a single module render.
const SHELL_RESOLVE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug)]
pub enum AdapterError {
    Failed(String),
    Timeout,
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::Failed(msg) => write!(f, "{msg}"),
            AdapterError::Timeout => write!(f, "timed out"),
        }
    }
}

pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, AdapterError> {
    let mut child = cmd
        .spawn()
        .map_err(|e| AdapterError::Failed(e.to_string()))?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AdapterError::Timeout);
            }
            Err(e) => return Err(AdapterError::Failed(e.to_string())),
        }
    }

    // Large output can exceed the pipe buffer and cause a deadlock.
    child
        .wait_with_output()
        .map_err(|e| AdapterError::Failed(e.to_string()))
}

pub fn invoke_module(
    name: &str,
    worktree_path: &Path,
    config_file: Option<&Path>,
) -> Result<String, AdapterError> {
    let mut cmd = Command::new("starship");
    cmd.arg("module")
        .arg(name)
        .current_dir(worktree_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Starship reads the inherited PWD value, not the real directory set by current_dir.
    cmd.env("PWD", worktree_path);
    if let Some(config) = config_file {
        cmd.env("STARSHIP_CONFIG", config);
    }

    let output = run_with_timeout(cmd, TIMEOUT)?;

    // Check stderr, not exit status, to find an error.
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(AdapterError::Failed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn rc_file_for_shell(shell: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let rc_name = match Path::new(shell).file_name()?.to_str()? {
        "zsh" => ".zshrc",
        "bash" => ".bash_profile",
        _ => return None,
    };
    Some(PathBuf::from(home).join(rc_name))
}

fn rc_mtime_secs(rc_file: &Path) -> Option<u64> {
    std::fs::metadata(rc_file)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn read_cached_path(cache_file: &Path, rc_file: &Path) -> Option<String> {
    let rc_secs = rc_mtime_secs(rc_file)?;
    let cached = std::fs::read_to_string(cache_file).ok()?;
    let mut lines = cached.lines();
    let cached_secs: u64 = lines.next()?.parse().ok()?;
    let cached_path = lines.next()?;
    (cached_secs == rc_secs).then(|| cached_path.to_string())
}

fn write_cached_path(cache_file: &Path, rc_file: &Path, resolved: &str) {
    if let Some(rc_secs) = rc_mtime_secs(rc_file) {
        let _ = std::fs::write(cache_file, format!("{rc_secs}\n{resolved}\n"));
    }
}

fn login_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// Uses `-lic`, not `-lc`, so `.zshrc` PATH exports load. Returns `None` on failure.
fn resolve_login_shell_path_with_cache(cache_file: &Path) -> Option<String> {
    let shell = login_shell();
    let rc_file = rc_file_for_shell(&shell);

    if let Some(rc_file) = &rc_file
        && let Some(cached) = read_cached_path(cache_file, rc_file)
    {
        return Some(cached);
    }

    let mut cmd = Command::new(&shell);
    cmd.arg("-lic")
        // Use `printf` not `echo -n` to avoid printing newlines from PATH entries
        .arg("printf %s \"$PATH\"")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = match run_with_timeout(cmd, SHELL_RESOLVE_TIMEOUT) {
        Ok(output) => output,
        Err(e) => {
            eprintln!("path-resolve: adapter error: {e}");
            return None;
        }
    };
    if !output.status.success() {
        eprintln!("path-resolve: login shell exited with a failure");
        return None;
    }
    let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if resolved.is_empty() {
        eprintln!("path-resolve: login shell produced an empty PATH");
        return None;
    }

    if let Some(rc_file) = &rc_file {
        write_cached_path(cache_file, rc_file, &resolved);
    }
    Some(resolved)
}

fn path_cache_file() -> PathBuf {
    std::env::temp_dir().join("herdr-starship-path-cache")
}

pub fn resolve_login_shell_path() -> Option<String> {
    resolve_login_shell_path_with_cache(&path_cache_file())
}

const SOURCE: &str = "herdr-starship";

pub fn report_metadata(workspace_id: &str, tokens: &[(&str, &str)]) -> Result<(), AdapterError> {
    let mut cmd = Command::new("herdr");
    cmd.arg("workspace")
        .arg("report-metadata")
        .arg(workspace_id)
        .arg("--source")
        .arg(SOURCE);
    for (key, val) in tokens {
        cmd.arg("--token").arg(format!("{key}={val}"));
    }
    let output = cmd
        .output()
        .map_err(|e| AdapterError::Failed(e.to_string()))?;
    if !output.status.success() {
        return Err(AdapterError::Failed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    Ok(())
}

// Stops races between env-mutating tests and tests that need an intact `PATH`.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn init_temp_repo(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&dir)
            .output()
            .unwrap();
        dir
    }

    /// **Starship**: Directory module returns the directory name.
    #[test]
    fn invoke_module_happy_path_returns_directory_name() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = init_temp_repo("herdr-starship-test-happy-path");
        let result = invoke_module("directory", &dir, None).unwrap();

        assert!(
            result.contains("herdr-starship-test-happy-path"),
            "expected output to contain the directory name, got: {result:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// **Starship**: Git-context module exits 0 and empty outside a git repo.
    #[test]
    fn invoke_module_non_git_repo_returns_empty_not_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = init_temp_repo("herdr-starship-test-non-git");
        fs::remove_dir_all(dir.join(".git")).unwrap();
        let result = invoke_module("git_branch", &dir, None).unwrap();

        assert_eq!(result, "");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// **Starship**: Unknown module name exits 0 and writes an error to stderr.
    #[test]
    fn invoke_module_unknown_module_name_returns_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = init_temp_repo("herdr-starship-test-unknown-module");
        let result = invoke_module("totally_fake_module_xyz", &dir, None);

        assert!(matches!(result, Err(AdapterError::Failed(_))));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Missing binary returns an error, not a panic.
    #[test]
    fn invoke_module_missing_binary_returns_error_not_panic() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = init_temp_repo("herdr-starship-test-missing-binary");
        let original_path = std::env::var("PATH").ok();

        // Serialize this test against other tests that change PATH to avoid races / failures
        unsafe { std::env::set_var("PATH", "") };
        let result = invoke_module("directory", &dir, None);
        unsafe {
            match &original_path {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }

        assert!(matches!(result, Err(AdapterError::Failed(_))));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// **Starship**: a malformed config still succeeds via fallback, so this must return `Err` anyway.
    #[test]
    fn invoke_module_malformed_config_returns_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = init_temp_repo("herdr-starship-test-bad-config");
        let config_path = dir.join("bad-starship.toml");
        fs::write(&config_path, "this is not valid toml [[[").unwrap();
        let result = invoke_module("directory", &dir, Some(&config_path));

        assert!(matches!(result, Err(AdapterError::Failed(_))));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Hung subprocesses are killed after a timeout, and the adapter returns an error.
    #[test]
    fn invoke_module_hung_subprocess_times_out() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = init_temp_repo("herdr-starship-test-hang");

        let fake_bin_dir = std::env::temp_dir().join("herdr-starship-test-hang-bin");
        let _ = fs::remove_dir_all(&fake_bin_dir);
        fs::create_dir_all(&fake_bin_dir).unwrap();
        let fake_starship = fake_bin_dir.join("starship");
        fs::write(&fake_starship, "#!/bin/sh\n/bin/sleep 30\n").unwrap();
        let mut perms = fs::metadata(&fake_starship).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755); // executable
        fs::set_permissions(&fake_starship, perms).unwrap();

        let original_path = std::env::var("PATH").ok();
        unsafe { std::env::set_var("PATH", &fake_bin_dir) };
        let started = std::time::Instant::now();
        let result = invoke_module("directory", &dir, None);
        let elapsed = started.elapsed();
        unsafe {
            match &original_path {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }

        assert!(matches!(result, Err(AdapterError::Timeout)));
        assert!(
            elapsed < Duration::from_secs(10),
            "expected timeout well under the 30s sleep, took {elapsed:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&fake_bin_dir).unwrap();
    }

    /// **Starship**: a worktree path with shell metacharacters is passed as argv, not by a shell.
    #[test]
    fn invoke_module_worktree_path_with_shell_metacharacters_is_not_interpreted() {
        let _guard = ENV_LOCK.lock().unwrap();
        let canary = std::env::temp_dir().join("herdr-starship-injection-canary");
        let _ = fs::remove_file(&canary);
        // The canary file appears only if shell injection succeeds.
        let dir_name = format!(
            "herdr-starship-test-injection; touch {} #",
            canary.display()
        );
        let dir = init_temp_repo(&dir_name);
        let _ = invoke_module("directory", &dir, None);

        assert!(
            !canary.exists(),
            "shell metacharacters in worktree_path were interpreted"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A disposable workspace for `report_metadata` tests against the real CLI. Closes on drop.
    struct ScratchWorkspace {
        id: String,
    }

    impl ScratchWorkspace {
        fn create(label: &str) -> Self {
            let output = Command::new("herdr")
                .args(["workspace", "create", "--no-focus", "--label", label])
                .output()
                .expect("herdr workspace create failed to run");
            assert!(
                output.status.success(),
                "herdr workspace create failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Minimal text search, not a full JSON parse: `{...,"workspace_id":"w3V",...}`
            let key = "\"workspace_id\":\"";
            let start = stdout.find(key).expect("no workspace_id in create output") + key.len();
            let end = stdout[start..].find('"').unwrap() + start;
            ScratchWorkspace {
                id: stdout[start..end].to_string(),
            }
        }
        fn tokens_json(&self) -> String {
            let output = Command::new("herdr")
                .args(["workspace", "get", &self.id])
                .output()
                .expect("herdr workspace get failed to run");
            String::from_utf8_lossy(&output.stdout).to_string()
        }
    }

    impl Drop for ScratchWorkspace {
        fn drop(&mut self) {
            let _ = Command::new("herdr")
                .args(["workspace", "close", &self.id])
                .output();
        }
    }

    /// **Herdr**: `report-metadata` with one token writes token to workspace metadata.
    #[test]
    fn report_metadata_happy_path_one_token() {
        let _guard = ENV_LOCK.lock().unwrap();
        let ws = ScratchWorkspace::create("herdr-starship-test-report-metadata");
        report_metadata(&ws.id, &[("git_state", "REBASING")]).unwrap();

        assert!(ws.tokens_json().contains(r#""git_state":"REBASING""#));
    }

    /// **Herdr**: `report-metadata` writes every token from one call to workspace metadata.
    #[test]
    fn report_metadata_multiple_tokens_in_one_call() {
        let _guard = ENV_LOCK.lock().unwrap();
        let ws = ScratchWorkspace::create("herdr-starship-test-multi-token");

        report_metadata(&ws.id, &[("git_branch", "main"), ("git_state", "MERGING")]).unwrap();
        let tokens = ws.tokens_json();
        assert!(tokens.contains(r#""git_branch":"main""#));
        assert!(tokens.contains(r#""git_state":"MERGING""#));
    }

    /// **Herdr**: `report-metadata` with a nonexistent workspace returns error, not crash.
    #[test]
    fn report_metadata_nonexistent_workspace_returns_error_not_crash() {
        let _guard = ENV_LOCK.lock().unwrap();
        let result = report_metadata("w-does-not-exist", &[("git_state", "REBASING")]);

        assert!(matches!(result, Err(AdapterError::Failed(_))));
    }

    #[test]
    fn login_shell_falls_back_to_bin_sh_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        let original = std::env::var("SHELL").ok();
        unsafe { std::env::remove_var("SHELL") };
        let result = login_shell();

        unsafe {
            match &original {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
        assert_eq!(result, "/bin/sh");
    }

    #[test]
    fn login_shell_uses_shell_env_var_when_set() {
        let _guard = ENV_LOCK.lock().unwrap();
        let original = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", "/usr/bin/zsh") };
        let result = login_shell();

        unsafe {
            match &original {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
        assert_eq!(result, "/usr/bin/zsh");
    }

    fn write_fake_shell(dir: &Path, name: &str, body: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn with_shell<T>(shell_path: &Path, f: impl FnOnce() -> T) -> T {
        let original = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", shell_path) };
        let result = f();
        unsafe {
            match &original {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
        result
    }

    #[test]
    fn resolve_login_shell_path_happy_path_returns_resolved_path() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("herdr-starship-test-resolve-happy-bin");
        let _ = fs::remove_dir_all(&dir);
        let shell = write_fake_shell(&dir, "fake-shell", "printf %s \"/fake/resolved/path\"");
        let cache_file = std::env::temp_dir().join("herdr-starship-test-resolve-happy-cache");
        let _ = fs::remove_file(&cache_file);
        let result = with_shell(&shell, || resolve_login_shell_path_with_cache(&cache_file));

        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result, Some("/fake/resolved/path".to_string()));
    }

    #[test]
    fn resolve_login_shell_path_spawn_failure_returns_none() {
        let _guard = ENV_LOCK.lock().unwrap();
        let missing_shell = std::env::temp_dir().join("herdr-starship-test-resolve-no-such-shell");
        let cache_file = std::env::temp_dir().join("herdr-starship-test-resolve-spawn-fail-cache");
        let _ = fs::remove_file(&cache_file);

        let result = with_shell(&missing_shell, || {
            resolve_login_shell_path_with_cache(&cache_file)
        });

        assert_eq!(result, None);
    }

    #[test]
    fn resolve_login_shell_path_empty_output_returns_none() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("herdr-starship-test-resolve-empty-bin");
        let _ = fs::remove_dir_all(&dir);
        let shell = write_fake_shell(&dir, "fake-shell", "printf %s \"   \"");
        let cache_file = std::env::temp_dir().join("herdr-starship-test-resolve-empty-cache");
        let _ = fs::remove_file(&cache_file);

        let result = with_shell(&shell, || resolve_login_shell_path_with_cache(&cache_file));

        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_login_shell_path_hung_shell_times_out() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("herdr-starship-test-resolve-hang-bin");
        let _ = fs::remove_dir_all(&dir);
        let shell = write_fake_shell(&dir, "fake-shell", "/bin/sleep 30");
        let cache_file = std::env::temp_dir().join("herdr-starship-test-resolve-hang-cache");
        let _ = fs::remove_file(&cache_file);

        let started = Instant::now();
        let result = with_shell(&shell, || resolve_login_shell_path_with_cache(&cache_file));
        let elapsed = started.elapsed();

        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result, None);
        assert!(
            elapsed < Duration::from_secs(15),
            "expected timeout well under the 30s sleep, took {elapsed:?}"
        );
    }

    fn init_zsh_cache_fixture(label: &str, shell_body: &str) -> (PathBuf, PathBuf, PathBuf) {
        let home = std::env::temp_dir().join(format!("herdr-starship-test-{label}-home"));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let rc_file = home.join(".zshrc");
        fs::write(&rc_file, "# fake zshrc\n").unwrap();

        let bin_dir = std::env::temp_dir().join(format!("herdr-starship-test-{label}-bin"));
        let _ = fs::remove_dir_all(&bin_dir);
        let shell = write_fake_shell(&bin_dir, "zsh", shell_body);
        (home, rc_file, shell)
    }

    #[test]
    fn resolve_login_shell_path_cache_hit_skips_shell_spawn() {
        let _guard = ENV_LOCK.lock().unwrap();
        let canary = std::env::temp_dir().join("herdr-starship-test-cache-hit-canary");
        let _ = fs::remove_file(&canary);
        let (home, rc_file, shell) = init_zsh_cache_fixture(
            "cache-hit",
            &format!(
                "touch {} && printf %s \"/should/not/be/used\"",
                canary.display()
            ),
        );
        let rc_secs = rc_mtime_secs(&rc_file).unwrap();
        let cache_file = std::env::temp_dir().join("herdr-starship-test-cache-hit-cache");
        fs::write(&cache_file, format!("{rc_secs}\n/cached/path\n")).unwrap();

        let original_home = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", &home) };
        let result = with_shell(&shell, || resolve_login_shell_path_with_cache(&cache_file));
        unsafe {
            match &original_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }

        fs::remove_dir_all(&home).unwrap();
        fs::remove_dir_all(shell.parent().unwrap()).unwrap();
        assert_eq!(result, Some("/cached/path".to_string()));
        assert!(!canary.exists(), "cache hit should not spawn the shell");
    }

    #[test]
    fn resolve_login_shell_path_cache_miss_when_rc_mtime_changed_re_resolves_and_rewrites_cache() {
        let _guard = ENV_LOCK.lock().unwrap();
        let (home, rc_file, shell) =
            init_zsh_cache_fixture("cache-miss", "printf %s \"/freshly/resolved/path\"");
        let stale_secs = rc_mtime_secs(&rc_file).unwrap().wrapping_sub(1);
        let cache_file = std::env::temp_dir().join("herdr-starship-test-cache-miss-cache");
        fs::write(&cache_file, format!("{stale_secs}\n/stale/cached/path\n")).unwrap();

        let original_home = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", &home) };
        let result = with_shell(&shell, || resolve_login_shell_path_with_cache(&cache_file));
        unsafe {
            match &original_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }

        let rewritten = fs::read_to_string(&cache_file).unwrap();
        fs::remove_dir_all(&home).unwrap();
        fs::remove_dir_all(shell.parent().unwrap()).unwrap();
        assert_eq!(result, Some("/freshly/resolved/path".to_string()));
        assert!(rewritten.contains("/freshly/resolved/path"));
    }

    #[test]
    fn resolve_login_shell_path_makes_a_path_only_reachable_binary_runnable() {
        let _guard = ENV_LOCK.lock().unwrap();
        let extra_bin_dir = std::env::temp_dir().join("herdr-starship-test-e2e-extra-bin");
        let _ = fs::remove_dir_all(&extra_bin_dir);
        fs::create_dir_all(&extra_bin_dir).unwrap();
        let custom_tool = extra_bin_dir.join("herdr-starship-e2e-custom-tool");
        fs::write(&custom_tool, "#!/bin/sh\nprintf %s \"ok\"\n").unwrap();
        let mut perms = fs::metadata(&custom_tool).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&custom_tool, perms).unwrap();

        let minimal_path = "/usr/bin:/bin:/usr/sbin:/sbin";
        let fake_shell_dir = std::env::temp_dir().join("herdr-starship-test-e2e-shell-bin");
        let _ = fs::remove_dir_all(&fake_shell_dir);
        let shell = write_fake_shell(
            &fake_shell_dir,
            "fake-shell",
            &format!("printf %s \"{}:{minimal_path}\"", extra_bin_dir.display()),
        );
        let cache_file = std::env::temp_dir().join("herdr-starship-test-e2e-cache");
        let _ = fs::remove_file(&cache_file);

        let original_path = std::env::var("PATH").ok();
        unsafe { std::env::set_var("PATH", minimal_path) };

        let before = Command::new("herdr-starship-e2e-custom-tool").output();

        let resolved =
            with_shell(&shell, || resolve_login_shell_path_with_cache(&cache_file)).unwrap();
        unsafe { std::env::set_var("PATH", &resolved) };
        let after = Command::new("herdr-starship-e2e-custom-tool").output();

        unsafe {
            match &original_path {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
        fs::remove_dir_all(&extra_bin_dir).unwrap();
        fs::remove_dir_all(&fake_shell_dir).unwrap();

        assert!(
            before.is_err(),
            "custom tool should be unreachable under the minimal PATH before resolution"
        );
        let after = after.expect("custom tool should be runnable after PATH resolution");
        assert_eq!(String::from_utf8_lossy(&after.stdout), "ok");
    }
}
