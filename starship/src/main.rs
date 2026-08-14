mod config;
mod fitter;
mod starship;

use fitter::Module;
use std::path::{Path, PathBuf};
use std::process::Command;
use unicode_width::UnicodeWidthStr;

/// Used when `rows` has no `$module` tokens. Keeps the old behavior for
/// users who have not changed their Herdr config.
const DEFAULT_MODULES: [&str; 5] = ["directory", "git_branch", "git_status", "git_state", "rust"];

/// Reads `workspace_cwd` from `HERDR_PLUGIN_CONTEXT_JSON`, since Herdr runs hooks in the
/// plugin's own directory, not the workspace's. Uses the current directory otherwise.
fn target_repo() -> PathBuf {
    std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|json| extract_json_string(&json, "workspace_cwd"))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("no current directory"))
}

/// Finds a top-level `"key":"value"` field in text, without a full JSON parse.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = json.find(&needle)? + needle.len();
    let end = json[start..].find('"')? + start;
    Some(json[start..end].to_string())
}

/// A user's own `starship.toml` in Herdr's per-plugin config dir wins over the bundled
/// default, so customizing the prompt never requires editing this repo.
fn config_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR") {
        let user_config = PathBuf::from(dir).join("starship.toml");
        if user_config.is_file() {
            return user_config;
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/starship-herdr.toml")
}

/// Runs `starship prompt` to render the whole configured line in one call.
fn invoke_prompt(repo: &Path, config: &Path) -> String {
    let output = Command::new("starship")
        .arg("prompt")
        .arg("--path")
        .arg(repo)
        .arg("--terminal-width")
        .arg("200")
        .env("STARSHIP_CONFIG", config)
        // Removes `STARSHIP_SHELL` so output is plain ANSI, not zsh's `%{...%}` markers.
        .env_remove("STARSHIP_SHELL")
        .output()
        .expect("failed to run starship prompt");
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// `"starship"` is this plugin's composite key, not a real starship module.
/// The default install config includes it, so this exclusion is the common case.
fn resolve_modules(herdr_config: &config::HerdrConfig) -> Vec<String> {
    let discovered: Vec<String> = herdr_config
        .modules
        .iter()
        .filter(|name| name.as_str() != "starship")
        .cloned()
        .collect();
    if discovered.is_empty() {
        DEFAULT_MODULES.iter().map(|s| s.to_string()).collect()
    } else {
        discovered
    }
}

/// Composite `starship` entry goes first: `fit()` drops the last entry first, and most
/// sidebar configs render only `$starship`.
fn collect_modules(repo: &Path, config: &Path, modules: &[String]) -> Vec<Module> {
    let mut rendered = vec![Module::new("starship", invoke_prompt(repo, config))];
    rendered.extend(modules.iter().filter_map(|name| {
        let name = name.as_str();
        match starship::invoke_module(name, repo, Some(config)) {
            Ok(content) if name == "directory" => Some(Module::with_abbreviate(
                name,
                content,
                fitter::abbreviate_directory,
            )),
            Ok(content) => Some(Module::new(name, content)),
            Err(e) => {
                eprintln!("{name}: adapter error: {e}");
                None
            }
        }
    }));
    rendered
}

/// Cosmetic column width of `s`, ANSI codes stripped. Uses `unicode-width`'s raw
/// count so alignment matches how most terminals actually render.
fn column_width(s: &str) -> usize {
    UnicodeWidthStr::width(fitter::strip_ansi(s).as_str())
}

fn print_modules(modules: &[Module]) {
    let max_width = modules
        .iter()
        .map(|m| column_width(&m.content))
        .max()
        .unwrap_or(0);
    for m in modules {
        let pad = " ".repeat(max_width - column_width(&m.content) + 2);
        let debug = format!("{:?}", m.content);
        let raw = &debug[1..debug.len() - 1];
        println!("{:>12}: {}\x1b[0m{pad}(r#\"{raw}\")", m.name, m.content);
    }
}

/// A CLI arg, if present, always wins over config. A manual `cargo run 40`
/// still works, even when config sets `sidebar_width`.
fn parse_budget(args: &[String], config_width: Option<usize>) -> usize {
    args.get(1)
        .and_then(|a| a.parse().ok())
        .or(config_width)
        .unwrap_or(26)
}

fn wants_push(args: &[String]) -> bool {
    args.iter().any(|a| a == "--push")
}

/// Herdr's token store mangles raw ANSI, so this strips it before every push.
fn push_tokens(workspace_id: &str, modules: &[Module]) -> Result<(), starship::AdapterError> {
    let stripped: Vec<(String, String)> = modules
        .iter()
        .map(|m| (m.name.clone(), fitter::strip_ansi(&m.content)))
        .collect();
    let tokens: Vec<(&str, &str)> = stripped
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    starship::report_metadata(workspace_id, &tokens)
}

fn main() {
    let repo = target_repo();
    let config = config_path();
    let args: Vec<String> = std::env::args().collect();

    let herdr_config = match config::default_path() {
        Some(path) => config::load(&path),
        None => {
            eprintln!("config: $HOME not set, skipping ~/.config/herdr/config.toml");
            config::HerdrConfig::default()
        }
    };
    let budget = parse_budget(&args, herdr_config.sidebar_width);
    let modules = resolve_modules(&herdr_config);

    let rendered = collect_modules(&repo, &config, &modules);
    println!("--- output (raw output) ---");
    print_modules(&rendered);

    let fitted = fitter::fit(rendered, budget);
    println!("\n--- fitted to budget={budget} columns ---");
    print_modules(&fitted);

    if wants_push(&args) {
        let workspace_id = std::env::var("HERDR_WORKSPACE_ID")
            .expect("--push requires HERDR_WORKSPACE_ID (run inside a herdr session)");
        match push_tokens(&workspace_id, &fitted) {
            Ok(()) => println!("\npushed to workspace {workspace_id}"),
            Err(e) => eprintln!("\nreport_metadata error: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_string_finds_top_level_field() {
        let json = r#"{"workspace_id":"w1","workspace_cwd":"/tmp/repo","tab_id":"w1:t1"}"#;
        let result = extract_json_string(json, "workspace_cwd");

        assert_eq!(result, Some("/tmp/repo".to_string()));
    }

    #[test]
    fn extract_json_string_returns_none_when_key_missing() {
        let json = r#"{"workspace_id":"w1"}"#;
        let result = extract_json_string(json, "workspace_cwd");

        assert_eq!(result, None);
    }

    #[test]
    fn target_repo_falls_back_to_current_dir_when_context_json_absent() {
        let _guard = starship::ENV_LOCK.lock().unwrap();
        let original = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok();
        unsafe { std::env::remove_var("HERDR_PLUGIN_CONTEXT_JSON") };

        let result = target_repo();

        if let Some(value) = original {
            unsafe { std::env::set_var("HERDR_PLUGIN_CONTEXT_JSON", value) };
        }
        assert_eq!(result, std::env::current_dir().unwrap());
    }

    /// Falls back to the bundled config when `HERDR_PLUGIN_CONFIG_DIR` is unset.
    #[test]
    fn config_path_falls_back_to_bundled_default_when_env_unset() {
        let _guard = starship::ENV_LOCK.lock().unwrap();
        let original = std::env::var("HERDR_PLUGIN_CONFIG_DIR").ok();
        unsafe { std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR") };

        let result = config_path();

        if let Some(value) = original {
            unsafe { std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", value) };
        }
        assert_eq!(
            result,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/starship-herdr.toml")
        );
    }

    /// Falls back to the bundled config when the dir is set but has no `starship.toml`.
    #[test]
    fn config_path_falls_back_when_user_config_missing() {
        let _guard = starship::ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("herdr-starship-test-config-dir-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let original = std::env::var("HERDR_PLUGIN_CONFIG_DIR").ok();
        unsafe { std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", &dir) };

        let result = config_path();

        match original {
            Some(value) => unsafe { std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", value) },
            None => unsafe { std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR") },
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            result,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/starship-herdr.toml")
        );
    }

    /// A `starship.toml` present in `HERDR_PLUGIN_CONFIG_DIR` overrides the bundled default.
    #[test]
    fn config_path_prefers_user_config_when_present() {
        let _guard = starship::ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("herdr-starship-test-config-dir-override");
        std::fs::create_dir_all(&dir).unwrap();
        let user_config = dir.join("starship.toml");
        std::fs::write(&user_config, "add_newline = false\n").unwrap();
        let original = std::env::var("HERDR_PLUGIN_CONFIG_DIR").ok();
        unsafe { std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", &dir) };

        let result = config_path();

        match original {
            Some(value) => unsafe { std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", value) },
            None => unsafe { std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR") },
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result, user_config);
    }

    #[test]
    fn target_repo_reads_workspace_cwd_from_context_json_when_present() {
        let _guard = starship::ENV_LOCK.lock().unwrap();
        let original = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok();
        unsafe {
            std::env::set_var(
                "HERDR_PLUGIN_CONTEXT_JSON",
                r#"{"workspace_id":"w1","workspace_cwd":"/tmp/some-other-repo"}"#,
            );
        }

        let result = target_repo();

        match original {
            Some(value) => unsafe { std::env::set_var("HERDR_PLUGIN_CONTEXT_JSON", value) },
            None => unsafe { std::env::remove_var("HERDR_PLUGIN_CONTEXT_JSON") },
        }
        assert_eq!(result, PathBuf::from("/tmp/some-other-repo"));
    }

    #[test]
    fn parse_budget_defaults_to_26_when_arg_and_config_missing() {
        let args = vec!["herdr-starship".to_string()];
        let result = parse_budget(&args, None);

        assert_eq!(result, 26);
    }

    #[test]
    fn parse_budget_parses_provided_arg() {
        let args = vec!["herdr-starship".to_string(), "40".to_string()];
        let result = parse_budget(&args, None);

        assert_eq!(result, 40);
    }

    #[test]
    fn parse_budget_uses_config_when_arg_absent() {
        let args = vec!["herdr-starship".to_string()];
        let result = parse_budget(&args, Some(50));

        assert_eq!(result, 50);
    }

    #[test]
    fn parse_budget_arg_wins_over_config() {
        let args = vec!["herdr-starship".to_string(), "40".to_string()];
        let result = parse_budget(&args, Some(50));

        assert_eq!(result, 40);
    }

    #[test]
    fn wants_push_true_when_flag_present() {
        let args = vec![
            "herdr-starship".to_string(),
            "26".to_string(),
            "--push".to_string(),
        ];
        let result = wants_push(&args);

        assert!(result);
    }

    #[test]
    fn wants_push_false_when_flag_absent() {
        let args = vec!["herdr-starship".to_string(), "26".to_string()];
        let result = wants_push(&args);

        assert!(!result);
    }

    /// **Starship**: modules include the composite `starship` entry.
    #[test]
    fn collect_modules_includes_composite_starship_prompt_entry() {
        let _guard = starship::ENV_LOCK.lock().unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let config = repo.join("examples/starship-herdr.toml");
        let modules = resolve_modules(&config::HerdrConfig::default());
        let result = collect_modules(repo, &config, &modules);

        assert!(result.iter().any(|m| m.name == "starship"));
    }

    /// A non-starship `$`-token (for example `$num`) degrades to a logged skip
    /// instead of crashing.
    #[test]
    fn collect_modules_skips_unknown_module_without_crashing() {
        let _guard = starship::ENV_LOCK.lock().unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let config = repo.join("examples/starship-herdr.toml");
        let modules = vec!["num".to_string()];
        let result = collect_modules(repo, &config, &modules);

        assert!(!result.iter().any(|m| m.name == "num"));
    }

    /// With no `$module` tokens in `rows`, this returns the same 5 hardcoded
    /// modules as before, for users who have not changed `rows`.
    #[test]
    fn resolve_modules_falls_back_to_defaults_when_no_module_tokens() {
        let herdr_config = config::HerdrConfig::default();
        let result = resolve_modules(&herdr_config);

        assert_eq!(
            result,
            DEFAULT_MODULES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    }

    /// Regression test. `$starship` in `rows` must not count as a `$module`
    /// token, or every installed user loses the 4 individual per-module tokens.
    #[test]
    fn resolve_modules_falls_back_to_defaults_when_only_starship_token_present() {
        let herdr_config = config::HerdrConfig {
            modules: vec!["starship".to_string()],
            sidebar_width: None,
        };
        let result = resolve_modules(&herdr_config);

        assert_eq!(
            result,
            DEFAULT_MODULES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    }

    /// A real `$module` token in `rows` drives discovery instead of the fallback.
    /// This excludes the reserved `"starship"` name from the result.
    #[test]
    fn resolve_modules_uses_discovered_tokens_excluding_starship() {
        let herdr_config = config::HerdrConfig {
            modules: vec!["starship".to_string(), "aws".to_string()],
            sidebar_width: None,
        };
        let result = resolve_modules(&herdr_config);

        assert_eq!(result, vec!["aws".to_string()]);
    }

    /// Disposable, unfocused `herdr` workspace for `push_tokens` tests. Closes on drop.
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
            // Minimal text search, not a full JSON parse. For example: `{"...,"workspace_id":"w3V",...}`
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

    /// **Herdr**: writes ANSI-stripped tokens to the workspace's metadata.
    #[test]
    fn push_tokens_strips_ansi_before_writing_to_workspace() {
        let _guard = starship::ENV_LOCK.lock().unwrap();
        let ws = ScratchWorkspace::create("herdr-starship-main-test-push-tokens");
        let modules = vec![Module::new("git_state", "\x1b[1;33mREBASING\x1b[0m")];

        push_tokens(&ws.id, &modules).unwrap();

        assert!(ws.tokens_json().contains(r#""git_state":"REBASING""#));
    }
}
