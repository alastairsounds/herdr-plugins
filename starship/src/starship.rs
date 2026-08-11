use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum AdapterError {
    Failed(String),
    Timeout,
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
    let mut child = cmd
        .spawn()
        .map_err(|e| AdapterError::Failed(e.to_string()))?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < TIMEOUT => {
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
    let output = child
        .wait_with_output()
        .map_err(|e| AdapterError::Failed(e.to_string()))?;

    // Check stderr, not exit status, to find an error.
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(AdapterError::Failed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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

    /// **Starship**: Malformed config file writes an error to stderr, but still produces valid output.
    /// 
    /// We want a bad config file to push no tokens, so this function must return Err even when
    /// starship succeeds with a fallback.
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

    /// **Starship**: Worktree path with shell metacharacters is passed as argv, not interpolated by a shell.
    #[test]
    fn invoke_module_worktree_path_with_shell_metacharacters_is_not_interpreted() {
        let _guard = ENV_LOCK.lock().unwrap();
        let canary = std::env::temp_dir().join("herdr-starship-injection-canary");
        let _ = fs::remove_file(&canary);
        // Canary acts as a sentinel for shell injection. If the test fails, the canary file will be created.
        let dir_name = format!("herdr-starship-test-injection; touch {} #", canary.display());
        let dir = init_temp_repo(&dir_name);
        let _ = invoke_module("directory", &dir, None);

        assert!(!canary.exists(), "shell metacharacters in worktree_path were interpreted");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Creates a disposable, unfocused herdr workspace for tests of report_metadata
    /// against the real herdr CLI. Closes the workspace on drop.
    struct ScratchWorkspace {
        id: String,
    }

    impl ScratchWorkspace {
        fn create(label: &str) -> Self {
            let output = Command::new("herdr")
                .args(["workspace", "create", "--no-focus", "--label", label])
                .output()
                .expect("herdr workspace create failed to run");
            assert!(output.status.success(), "herdr workspace create failed: {}", String::from_utf8_lossy(&output.stderr));
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Minimal text search, not full JSON parse. e.g. `{"...,"workspace_id":"w3V",...}`
            let key = "\"workspace_id\":\"";
            let start = stdout.find(key).expect("no workspace_id in create output") + key.len();
            let end = stdout[start..].find('"').unwrap() + start;
            ScratchWorkspace { id: stdout[start..end].to_string() }
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
            let _ = Command::new("herdr").args(["workspace", "close", &self.id]).output();
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

    /// **Herdr**: `report-metadata` with multiple tokens in one call writes all tokens to workspace metadata.
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
}
