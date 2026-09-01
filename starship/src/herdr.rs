use crate::starship::AdapterError;
use std::path::PathBuf;
use std::process::Command;

const SOURCE: &str = "herdr-starship";

/// An open workspace and the directory whose state renders for it.
#[derive(Debug, PartialEq, Clone)]
pub struct WorkspaceTarget {
    pub workspace_id: String,
    pub repo: PathBuf,
}

/// The `herdr` CLI boundary, so a refresh sweep is testable without a live session.
pub trait Herdr {
    fn workspace_targets(&self) -> Vec<WorkspaceTarget>;
    fn report_metadata(
        &self,
        workspace_id: &str,
        tokens: &[(&str, &str)],
    ) -> Result<(), AdapterError>;
}

/// Shells out to the real `herdr` binary.
pub struct Cli;

/// Resolves a workspace's active-pane directory, falling back to the first pane.
fn resolve_pane_cwd(workspace_id: &str, active_tab_id: Option<&str>) -> Option<PathBuf> {
    let output = Command::new("herdr")
        .args(["pane", "list", "--workspace", workspace_id])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let panes = value.get("result")?.get("panes")?.as_array()?;
    let pane = active_tab_id
        .and_then(|tab_id| {
            panes
                .iter()
                .find(|p| p.get("tab_id").and_then(|v| v.as_str()) == Some(tab_id))
        })
        .or_else(|| panes.first())?;
    pane.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from)
}

impl Herdr for Cli {
    /// Lists workspace IDs with pane directories, falling back to checkout paths.
    fn workspace_targets(&self) -> Vec<WorkspaceTarget> {
        let output = match Command::new("herdr").args(["workspace", "list"]).output() {
            Ok(o) if o.status.success() => o,
            Ok(o) => {
                eprintln!(
                    "herdr: workspace list failed: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
                return Vec::new();
            }
            Err(e) => {
                eprintln!("herdr: could not run workspace list: {e}");
                return Vec::new();
            }
        };
        let value: serde_json::Value = match serde_json::from_slice(&output.stdout) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("herdr: could not parse workspace list output: {e}");
                return Vec::new();
            }
        };
        let workspaces = value
            .get("result")
            .and_then(|v| v.get("workspaces"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        workspaces
            .iter()
            .filter_map(|ws| {
                let workspace_id = ws.get("workspace_id")?.as_str()?.to_string();
                let active_tab_id = ws.get("active_tab_id").and_then(|v| v.as_str());
                let repo = resolve_pane_cwd(&workspace_id, active_tab_id).or_else(|| {
                    ws.get("worktree")
                        .and_then(|w| w.get("checkout_path"))
                        .and_then(|v| v.as_str())
                        .map(PathBuf::from)
                })?;
                Some(WorkspaceTarget { workspace_id, repo })
            })
            .collect()
    }

    fn report_metadata(
        &self,
        workspace_id: &str,
        tokens: &[(&str, &str)],
    ) -> Result<(), AdapterError> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::starship::ENV_LOCK;

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
        Cli.report_metadata(&ws.id, &[("git_state", "REBASING")])
            .unwrap();

        assert!(ws.tokens_json().contains(r#""git_state":"REBASING""#));
    }

    /// **Herdr**: `report-metadata` writes every token from one call to workspace metadata.
    #[test]
    fn report_metadata_multiple_tokens_in_one_call() {
        let _guard = ENV_LOCK.lock().unwrap();
        let ws = ScratchWorkspace::create("herdr-starship-test-multi-token");

        Cli.report_metadata(&ws.id, &[("git_branch", "main"), ("git_state", "MERGING")])
            .unwrap();
        let tokens = ws.tokens_json();
        assert!(tokens.contains(r#""git_branch":"main""#));
        assert!(tokens.contains(r#""git_state":"MERGING""#));
    }

    /// **Herdr**: `report-metadata` with a nonexistent workspace returns error, not crash.
    #[test]
    fn report_metadata_nonexistent_workspace_returns_error_not_crash() {
        let _guard = ENV_LOCK.lock().unwrap();
        let result = Cli.report_metadata("w-does-not-exist", &[("git_state", "REBASING")]);

        assert!(matches!(result, Err(AdapterError::Failed(_))));
    }

    /// **Herdr**: a real session reports at least the workspace this test runs in.
    #[test]
    fn workspace_targets_lists_the_live_session_workspaces() {
        let _guard = ENV_LOCK.lock().unwrap();
        let ws = ScratchWorkspace::create("herdr-starship-test-workspace-targets");

        let result = Cli.workspace_targets();

        assert!(
            result.iter().any(|t| t.workspace_id == ws.id),
            "expected the scratch workspace in {result:?}"
        );
    }
}
