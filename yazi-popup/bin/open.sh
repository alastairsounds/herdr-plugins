#!/usr/bin/env bash
# Opens the picker popup over the pane that triggered the keybind.
set -eo pipefail

extract_cwd() {
  printf '%s' "$1" | grep -o '"focused_pane_cwd":"[^"]*"' | sed -E 's/.*:"(.*)"$/\1/'
}

if [ "${1:-}" = "--self-test" ]; then
  result=$(extract_cwd '{"focused_pane_id":"p1","focused_pane_cwd":"/tmp/some dir"}')
  [ "$result" = "/tmp/some dir" ] || { echo "self-test failed: got '$result'" >&2; exit 1; }
  echo "ok"
  exit 0
fi

[ -n "${HERDR_PANE_ID:-}" ] || exit 0

cwd=$(extract_cwd "${HERDR_PLUGIN_CONTEXT_JSON:-{}}")

args=(plugin pane open --plugin "$HERDR_PLUGIN_ID" --entrypoint picker --env "HERDR_TARGET_PANE_ID=$HERDR_PANE_ID")
[ -n "$cwd" ] && args+=(--cwd "$cwd")

exec "${HERDR_BIN_PATH:-herdr}" "${args[@]}"
