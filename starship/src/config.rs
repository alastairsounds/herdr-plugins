use std::path::{Path, PathBuf};

/// The plugin's refresh setting; `Hook` currently falls back to `Basic`.
#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum RefreshMode {
    #[default]
    Basic,
    Poll,
    Hook,
}

/// Data read from `~/.config/herdr/config.toml`. The caller picks the fallback.
#[derive(Debug, Default, PartialEq)]
pub struct HerdrConfig {
    /// Bare module names taken from every `$`-prefixed token in `rows`.
    /// This list can include `"starship"`. The caller must exclude it if needed.
    pub modules: Vec<String>,
    pub sidebar_width: Option<usize>,
}

/// Data read from the plugin's private config file.
#[derive(Debug, PartialEq)]
pub struct RefreshConfig {
    pub refresh: RefreshMode,
    pub poll_interval_seconds: u64,
}

/// Provides the configured default poll interval instead of zero.
impl Default for RefreshConfig {
    fn default() -> Self {
        RefreshConfig {
            refresh: RefreshMode::default(),
            poll_interval_seconds: DEFAULT_POLL_INTERVAL_SECONDS,
        }
    }
}

/// `~/.config/herdr/config.toml`, or `None` if `$HOME` is not set.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/herdr/config.toml"))
}

/// Returns the plugin config path, preferring `$HERDR_PLUGIN_CONFIG_DIR`.
pub fn refresh_config_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("config.toml"));
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home).join(".config/herdr/plugins/config/herdr-starship/config.toml")
    })
}

/// Reads and parses `path`. Missing or bad TOML returns an empty `HerdrConfig`
/// and logs why, because this file holds settings other tools use too.
pub fn load(path: &Path) -> HerdrConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("config: could not read {}: {e}", path.display());
            return HerdrConfig::default();
        }
    };
    let table: toml::Table = match text.parse() {
        Ok(table) => table,
        Err(e) => {
            eprintln!("config: malformed TOML in {}: {e}", path.display());
            return HerdrConfig::default();
        }
    };
    let value = toml::Value::Table(table);
    HerdrConfig {
        modules: discover_modules(&value),
        sidebar_width: read_sidebar_width(&value),
    }
}

const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;

/// Loads the optional plugin config, defaulting silently when it is missing.
pub fn load_refresh(path: &Path) -> RefreshConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return RefreshConfig::default(),
    };
    let table: toml::Table = match text.parse() {
        Ok(table) => table,
        Err(e) => {
            eprintln!("config: malformed TOML in {}: {e}", path.display());
            return RefreshConfig::default();
        }
    };
    let value = toml::Value::Table(table);
    RefreshConfig {
        refresh: read_refresh_mode(&value),
        poll_interval_seconds: read_poll_interval_seconds(&value),
    }
}

/// Reads the top-level `refresh` key, logging and defaulting on invalid values.
fn read_refresh_mode(value: &toml::Value) -> RefreshMode {
    let Some(raw) = value.get("refresh").and_then(|v| v.as_str()) else {
        return RefreshMode::default();
    };
    match raw {
        "basic" => RefreshMode::Basic,
        "poll" => RefreshMode::Poll,
        "hook" => RefreshMode::Hook,
        other => {
            eprintln!(
                "config: unrecognized refresh value {other:?}, falling back to basic"
            );
            RefreshMode::Basic
        }
    }
}

/// Reads `poll_interval_seconds`, defaulting when absent or invalid.
fn read_poll_interval_seconds(value: &toml::Value) -> u64 {
    value
        .get("poll_interval_seconds")
        .and_then(|v| v.as_integer())
        .and_then(|n| u64::try_from(n).ok())
        .unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS)
}

/// Reads each row token as a plain string (`"$aws"`) or an inline table
/// like `{ token = "$aws", fg = "blue" }`. Returns names with `$` removed.
fn discover_modules(value: &toml::Value) -> Vec<String> {
    let Some(rows) = value
        .get("ui")
        .and_then(|v| v.get("sidebar"))
        .and_then(|v| v.get("spaces"))
        .and_then(|v| v.get("rows"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };

    rows.iter()
        .filter_map(|row| row.as_array())
        .flatten()
        .filter_map(|token| match token {
            toml::Value::String(s) => Some(s.as_str()),
            toml::Value::Table(t) => t.get("token").and_then(|v| v.as_str()),
            _ => None,
        })
        .filter_map(|s| s.strip_prefix('$'))
        .map(str::to_string)
        .collect()
}

/// `[ui].sidebar_width`, sibling to `sidebar_min_width`/`sidebar_max_width`.
fn read_sidebar_width(value: &toml::Value) -> Option<usize> {
    value
        .get("ui")
        .and_then(|v| v.get("sidebar_width"))
        .and_then(|v| v.as_integer())
        .and_then(|n| usize::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn load_missing_file_returns_empty_config() {
        let path = std::env::temp_dir().join("herdr-starship-test-config-does-not-exist.toml");
        let _ = std::fs::remove_file(&path);
        let result = load(&path);

        assert_eq!(result, HerdrConfig::default());
    }

    #[test]
    fn load_malformed_toml_returns_empty_config() {
        let path = write_config(
            "herdr-starship-test-config-malformed.toml",
            "this is not [[[ toml",
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result, HerdrConfig::default());
    }

    #[test]
    fn discover_modules_plain_string_tokens() {
        let path = write_config(
            "herdr-starship-test-config-plain.toml",
            r#"
            [ui.sidebar.spaces]
            rows = [["state_icon", "$aws"], ["$rust"]]
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.modules, vec!["aws".to_string(), "rust".to_string()]);
    }

    #[test]
    fn discover_modules_inline_table_tokens() {
        let path = write_config(
            "herdr-starship-test-config-inline-table.toml",
            r#"
            [ui.sidebar.spaces]
            rows = [[{ token = "$aws", fg = "blue" }]]
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.modules, vec!["aws".to_string()]);
    }

    #[test]
    fn discover_modules_row_mixing_both_shapes() {
        let path = write_config(
            "herdr-starship-test-config-mixed-row.toml",
            r#"
            [ui.sidebar.spaces]
            rows = [["$aws", { token = "$rust", bold = true }]]
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.modules, vec!["aws".to_string(), "rust".to_string()]);
    }

    #[test]
    fn discover_modules_missing_rows_key_returns_empty() {
        let path = write_config(
            "herdr-starship-test-config-no-rows.toml",
            r#"
            [ui.sidebar.spaces]
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.modules, Vec::<String>::new());
    }

    #[test]
    fn discover_modules_empty_rows_returns_empty() {
        let path = write_config(
            "herdr-starship-test-config-empty-rows.toml",
            r#"
            [ui.sidebar.spaces]
            rows = []
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.modules, Vec::<String>::new());
    }

    /// Non-starship `$`-tokens (for example `$num`) pass through unfiltered.
    /// Parser does not need to know which tokens are starship modules.
    #[test]
    fn discover_modules_includes_non_starship_tokens() {
        let path = write_config(
            "herdr-starship-test-config-non-starship-token.toml",
            r#"
            [ui.sidebar.spaces]
            rows = [["$num", "$rust"]]
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.modules, vec!["num".to_string(), "rust".to_string()]);
    }

    /// Parser only extracts tokens; it does not know "starship" is reserved.
    /// main.rs excludes it, where that reservation is defined.
    #[test]
    fn discover_modules_includes_starship_token_unfiltered() {
        let path = write_config(
            "herdr-starship-test-config-starship-token.toml",
            r#"
            [ui.sidebar.spaces]
            rows = [["$starship"]]
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.modules, vec!["starship".to_string()]);
    }

    #[test]
    fn sidebar_width_reads_ui_sidebar_width() {
        let path = write_config(
            "herdr-starship-test-config-sidebar-width.toml",
            r#"
            [ui]
            sidebar_width = 40
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.sidebar_width, Some(40));
    }

    #[test]
    fn sidebar_width_missing_is_none() {
        let path = write_config(
            "herdr-starship-test-config-no-sidebar-width.toml",
            r#"
            [ui]
            sidebar_min_width = 10
            "#,
        );
        let result = load(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.sidebar_width, None);
    }

    #[test]
    fn refresh_missing_key_defaults_to_basic() {
        let path = write_config(
            "herdr-starship-test-config-refresh-missing.toml",
            r#"
            unrelated_key = 40
            "#,
        );
        let result = load_refresh(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.refresh, RefreshMode::Basic);
    }

    #[test]
    fn refresh_reads_basic() {
        let path = write_config(
            "herdr-starship-test-config-refresh-basic.toml",
            r#"refresh = "basic""#,
        );
        let result = load_refresh(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.refresh, RefreshMode::Basic);
    }

    #[test]
    fn refresh_reads_poll() {
        let path = write_config(
            "herdr-starship-test-config-refresh-poll.toml",
            r#"refresh = "poll""#,
        );
        let result = load_refresh(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.refresh, RefreshMode::Poll);
    }

    #[test]
    fn refresh_reads_hook() {
        let path = write_config(
            "herdr-starship-test-config-refresh-hook.toml",
            r#"refresh = "hook""#,
        );
        let result = load_refresh(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.refresh, RefreshMode::Hook);
    }

    #[test]
    fn refresh_garbage_value_falls_back_to_basic() {
        let path = write_config(
            "herdr-starship-test-config-refresh-garbage.toml",
            r#"refresh = "yolo""#,
        );
        let result = load_refresh(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.refresh, RefreshMode::Basic);
    }

    #[test]
    fn poll_interval_seconds_missing_defaults_to_5() {
        let path = write_config(
            "herdr-starship-test-config-poll-interval-missing.toml",
            r#"refresh = "poll""#,
        );
        let result = load_refresh(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.poll_interval_seconds, 5);
    }

    #[test]
    fn poll_interval_seconds_reads_configured_value() {
        let path = write_config(
            "herdr-starship-test-config-poll-interval-set.toml",
            r#"
            refresh = "poll"
            poll_interval_seconds = 30
            "#,
        );
        let result = load_refresh(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.poll_interval_seconds, 30);
    }

    /// Ensures a missing refresh config uses the nonzero default interval.
    #[test]
    fn load_refresh_missing_file_returns_basic_and_default_interval() {
        let path = std::env::temp_dir().join("herdr-starship-test-refresh-does-not-exist.toml");
        let _ = std::fs::remove_file(&path);
        let result = load_refresh(&path);

        assert_eq!(result, RefreshConfig::default());
        assert_eq!(result.poll_interval_seconds, 5);
    }

    #[test]
    fn load_refresh_malformed_toml_returns_defaults() {
        let path = write_config(
            "herdr-starship-test-refresh-malformed.toml",
            "this is not [[[ toml",
        );
        let result = load_refresh(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(result, RefreshConfig::default());
    }

    #[test]
    fn refresh_config_path_prefers_env_dir_when_set() {
        let _guard = crate::starship::ENV_LOCK.lock().unwrap();
        let original = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR");
        let dir = std::env::temp_dir().join("herdr-starship-test-refresh-config-dir");
        unsafe { std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", &dir) };

        let result = refresh_config_path();

        match original {
            Some(value) => unsafe { std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", value) },
            None => unsafe { std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR") },
        }
        assert_eq!(result, Some(dir.join("config.toml")));
    }

    #[test]
    fn refresh_config_path_falls_back_to_home_when_env_unset() {
        let _guard = crate::starship::ENV_LOCK.lock().unwrap();
        let original_dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR");
        let original_home = std::env::var_os("HOME");
        unsafe { std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR") };
        let home = std::env::temp_dir().join("herdr-starship-test-refresh-home");
        unsafe { std::env::set_var("HOME", &home) };

        let result = refresh_config_path();

        match original_dir {
            Some(value) => unsafe { std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", value) },
            None => unsafe { std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR") },
        }
        match original_home {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        assert_eq!(
            result,
            Some(home.join(".config/herdr/plugins/config/herdr-starship/config.toml"))
        );
    }
}