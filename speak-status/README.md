# speak-status

Speaks an audible status update when an agent pane's status changes: a chime, then a
KittenTTS voice saying `<workspace label> done` or `<workspace label> waiting`. Works for any
agent Herdr detects (Claude, Codex, droid, ...), not just Claude Code.

## What it does

- `[[events]] on = "pane.agent_status_changed"`: fires a `forwarder` script on every status
  change. It fires `herdr notification show ... --sound done|request` immediately (the same
  native chime Herdr's own detection would eventually fire, but without waiting on its
  poll/debounce/toast-delay path), then resolves the workspace label and hands off to the
  resident daemon for the spoken part.
- `[[startup]]`: ensures a resident daemon is running, keeping the KittenTTS model warm so
  speech doesn't pay Python-start + model-load cost (~3-4s) on every single announcement.
- Only `done` and `blocked` statuses are announced; `idle`, `working`, and `unknown` are silent.

## Configuration

By default (`sound_plus_voice`), every announcement fires both the immediate chime and the
spoken words. If you've already got Herdr's own built-in per-agent sound enabled for an agent
(`[ui.sound.agents]` in `~/.config/herdr/config.toml`), you'll hear that chime too, since it's a
separate code path speak-status doesn't control. To avoid the double chime, set `sound_mode` to
`voice_only`, which skips speak-status's own chime and only speaks:

```toml
# ~/.config/herdr/plugins/config/speak-status/config.toml
sound_mode = "voice_only"
```

Herdr creates that directory automatically and injects its path as `HERDR_PLUGIN_CONFIG_DIR`
into every hook invocation (same pattern `herdr-starship` uses for its own config file). No
rebuild or reinstall needed — the next event picks up the change.

## Quick start

```bash
herdr plugin install alastairsounds/herdr-plugins/speak-status
```

Herdr runs `uv sync` to install dependencies (requires `uv` on `$PATH`), then wires up the
`startup` and `pane.agent_status_changed` hooks automatically.

## Requirements

- macOS (uses `afplay`; Linux/Windows untested)
- `uv` on `$PATH`
- Herdr `>= 0.8.0`

## Uninstall

```bash
herdr plugin uninstall speak-status
```

This does not stop an already-running daemon process; kill it manually (see
`docs/roadmap.md` for known limitations).
