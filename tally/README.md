# tally

Stamps the sidebar `$num` token onto each workspace and pane, with its display position. This matches what you see in the sidebar, not herdr's raw creation-order `number` field.

## What it does

- `[[startup]]`: runs `bin/tally.sh` at launch. Numbers are correct as soon as herdr starts, with no manual step.
- `[[events]] workspace.created` and `workspace.closed`: run the same script, to keep numbers correct as the total workspace count changes.
- `[[actions]] run`: runs the same script on demand. Use it after you reorder workspaces, to update the numbers right away. Bind it to a key in `~/.config/herdr/config.toml`.
- The script reads `herdr workspace list` and `herdr pane list`. It groups linked worktrees under their root workspace, to match herdr's sidebar nesting rule. Then it writes each position back with `herdr workspace report-metadata` and `herdr pane report-metadata`, using `--token num=N`.

## Quick start

```bash
herdr plugin install alastairsounds/herdr-plugins/tally
```

Add a keybinding in `~/.config/herdr/config.toml`, to run the action on demand:

```toml
[[keys.command]]
key = "prefix+alt+n"
type = "plugin_action"
command = "alastairsounds.tally.run"
description = "Renumber workspaces"
```

Reload with `herdr server reload-config`. Numbers appear on startup with no key press. If the sidebar order changes, press the key, to update the numbers.

## Pairs well with `switch_workspace`

The `$num` token is most useful together with herdr's `switch_workspace` key, which jumps to workspace N by number. This key is not set by default; add it under `[keys]` in `~/.config/herdr/config.toml`:

```toml
[keys]
switch_workspace = "prefix+1..9"
```

With both set, the sidebar number and the jump key always agree: `prefix+3` goes to the workspace tally marks `3`.

## Requirements

- macOS (Linux and Windows are untested)
- `jq` on `$PATH`
- Herdr `>= 0.8.0`
