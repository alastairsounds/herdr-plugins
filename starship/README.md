# herdr-starship

Herdr plugin that renders a `starship.toml` file onto a Herdr workspace's sidebar.

## Requirements

- `starship` and `herdr` on `$PATH`
- Herdr `>= 0.8.0`
- macOS (only platform exercised so far, see `herdr-plugin.toml`'s `platforms`)

## Install

```bash
herdr plugin install alastairsounds/herdr-starship
```

Herdr builds the plugin (`cargo build`) and wires up its `startup`/`worktree.created`/`workspace.created`/`workspace.focused` hooks automatically.

### Example starship config

The plugin renders using `examples/starship-herdr.toml`, bundled in this repo, unless you provide your own (see below). It's used automatically as soon as the plugin is installed or linked, no `~/.config/herdr/config.toml` wiring needed for this part:

```toml
add_newline = false

format = """
$rust$nodejs$bun$python\
$git_branch\
$git_status\
$git_state\
"""

[git_branch]
format = '[$symbol$branch(:$remote_branch)]($style) '

[git_status]
disabled = false
style = "bold yellow"
format = '[$all_status$ahead_behind]($style)'
conflicted = '[=$count ](fg:196)'
ahead = '[⇡$count ](fg:76)'
behind = '[⇣$count ](fg:76)'
diverged = '[⇕⇡$ahead_count⇣$behind_count ](fg:76)'
untracked = '[?$count ](fg:39)'
stashed = '[*$count ](fg:76)'
modified = '[!$count ](fg:178)'
staged = '[+$count ](fg:178)'
renamed = '[»$count ](fg:178)'
deleted = '[✘$count ](fg:196)'

[rust]
disabled = false
symbol = "󱘗 "
style = "fg:#DEA584"
format = '[$symbol]($style)'

[nodejs]
disabled = false
symbol = "󰎙 "
style = "fg:#68A063"
format = '[$symbol]($style)'

[bun]
disabled = false
symbol = " "
style = "fg:#FBF0DF"
format = '[$symbol]($style)'

[python]
disabled = false
symbol = " "
style = "fg:#3776AB"
format = '[$symbol]($style)'
```

### Customizing the prompt

Want a different prompt without forking this repo? Drop your own `starship.toml` at:

```
~/.config/herdr/plugins/config/herdr-starship/starship.toml
```

Herdr injects `HERDR_PLUGIN_CONFIG_DIR` into every hook invocation of this plugin, pointing at that directory. If `starship.toml` exists there, it wins over the bundled default above, no rebuild needed, no touching this repo's `examples/starship-herdr.toml`. Just save the file and let the next hook (or `herdr server reload-config` plus a workspace focus) pick it up.

### Sidebar setup (required, one-time)

`herdr plugin install`/`link` alone won't make anything show up. The plugin pushes tokens via `report-metadata`, but Herdr only renders a pushed token if `~/.config/herdr/config.toml`'s `[ui.sidebar.spaces].rows` lists a matching `$name` entry. For the fastest possible verification, paste this in as-is, it matches the config above exactly:

```toml
[ui.sidebar.spaces]
rows = [
  ["state_icon", "workspace", "$num"],
  ["$starship"]
]
```

Available tokens: `$directory`, `$git_branch`, `$git_status`, `$git_state`, `$rust`, `$starship` (the composite `starship prompt` line, this is the one that renders the config above).

You can use either the starship-flavored individual tokens above, or `$starship` to invoke your config file (or the bundled default if you haven't provided one).

Then reload:

```bash
herdr server reload-config
```

## Uninstall

```bash
herdr plugin uninstall herdr-starship
```

This does not remove the `rows` entry above. Remove it manually and reload if you want the sidebar row gone too.

## Contributing

For local development, link this checkout instead of installing the published plugin:

```bash
herdr plugin link .
```

To change the bundled default prompt, edit `examples/starship-herdr.toml` directly and rebuild:

```bash
cargo build
```
