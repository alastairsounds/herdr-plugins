# yazi-popup

Pops [Yazi](https://yazi-rs.github.io/) open over whichever pane you triggered it from, and types your pick(s) back into that pane as `@path`; unsubmitted, like a real `@` autocomplete. Built for referencing files to coding agents (Claude, Codex, etc.) without leaving the keyboard.

![Picking a file in the popup, typed back as @path into the composer](../_demo/yazi-popup/picker.gif)

## What it does

- `[[panes]] picker`: a `popup`-placement pane (80% width/height) running `bin/picker.sh`, which runs Yazi with `--chooser-file` and, once you pick (or quit), types the result back and exits, closing the popup.
- `[[actions]] pick`: `bin/open.sh` reads the triggering pane's id and cwd from herdr's plugin context, then opens the `picker` popup there via `herdr plugin pane open`, passing the target pane id through `--env`.

## Quick start

```bash
herdr plugin install alastairsounds/herdr-plugins/yazi-popup
```

Add a keybinding in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+cmd+p"
type = "plugin_action"
command = "alastairsounds.yazi-popup.pick"
description = "pick a file with yazi"
```

Reload (`herdr server reload-config`), then press the key: Yazi opens in a centered popup over your current pane, cd'd into that pane's directory. Pick one or more files (or quit with `q`/`ctrl+c`) and the popup closes, typing `@path ` for each pick into the pane you started from.

## Requirements

- macOS (Linux/Windows support untested)
- Yazi
