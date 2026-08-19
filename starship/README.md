# herdr-starship

Herdr plugin that renders a `starship.toml` file onto a Herdr workspace's sidebar.

## Requirements

- `starship` and `herdr` on `$PATH`
- Herdr `>= 0.8.0`
- macOS (only platform exercised so far, see `herdr-plugin.toml`'s `platforms`)

## Install

```bash
herdr plugin install alastairsounds/herdr-plugins/starship
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

Available tokens: any [starship module](https://starship.rs/config/#modules) name. Examples: `$directory`, `$git_branch`, `$aws`, `$nodejs`. You can also use `$starship`, the composite `starship prompt` line that renders the config above. Add any starship module name to `rows`, and it renders. This needs no herdr-starship code changes or rebuild.

You can use individual starship-module tokens, or use `$starship` to run your config file. If you have not provided your own file, herdr-starship uses the bundled default. If `rows` has no starship-module tokens at all, herdr-starship falls back to its default five modules: `directory`, `git_branch`, `git_status`, `git_state`, `rust`.

Then reload:

```bash
herdr server reload-config
```

### Sidebar width

Herdr's `~/.config/herdr/config.toml` has an `[ui].sidebar_width` value, next to `sidebar_min_width` and `sidebar_max_width`. This value sets the column budget that herdr-starship fits its output to. If this value is not set, herdr-starship uses the default value of 26.

### Refresh mode

By default (`basic`), tokens refresh on four events: `startup`, `worktree.created`, `workspace.created`, and `workspace.focused`. A workspace that stays focused does not get new git activity until it is focused again.

To fix this, create this file:

```
~/.config/herdr/plugins/config/herdr-starship/config.toml
```

```toml
refresh = "poll"              # "basic" (default) | "poll" | "hook" (reserved, see TODOS.md)
poll_interval_seconds = 5     # default 5, only meaningful when refresh = "poll"
```

This file is separate from `starship.toml` in the same directory. `starship.toml` sets what the plugin renders. `config.toml` sets the refresh schedule. The `~/.config/herdr/config.toml` file belongs to Herdr and rejects unknown plugin tables, so these settings cannot live there.

Restart herdr. `poll` mode starts a background process, with a pid file at `~/.local/state/herdr/plugins/herdr-starship/poll.pid`, on both macOS and Linux, with no OS-level scheduler entry. It ticks every `poll_interval_seconds` and refreshes every open workspace, not only the one that triggered a hook.

To stop `poll` mode, do one of these:
- Set `refresh` back to `basic`.
- Remove the config file.

Then restart herdr. This stops the process and removes the pid file. No manual step is needed.

`refresh = "hook"` is reserved for a future git-change watcher. This watcher will use file system events on `.git/index`, `.git/HEAD`, and `.git/refs/**`. Today it parses without error but behaves like `basic`, with one logged notice.

**Manual cleanup**

This procedure applies only in one case. You install an earlier version of this plugin, one without the `poll` loop. Herdr does not restart before you do this. In this case, remove the process by hand:

```bash
kill "$(grep -oE 'pid=[0-9]+' ~/.local/state/herdr/plugins/herdr-starship/poll.pid | cut -d= -f2)"
rm ~/.local/state/herdr/plugins/herdr-starship/poll.pid
```

## Roadmap

- Color in the sidebar (requires a PR into `herdrdev/herdr`)
- Parallelize per-module subprocess invocation
- `hook` mode: real git-change fs-watch, replacing `poll` (see `TODOS.md`)

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
