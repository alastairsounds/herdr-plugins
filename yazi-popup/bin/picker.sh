#!/usr/bin/env bash
# Types picks back as "@path " with no trailing Enter, like a real @ autocomplete.
set -eo pipefail

pane_id="${HERDR_TARGET_PANE_ID:-}"
[ -n "$pane_id" ] || exit 0

chooser=$(mktemp)
trap 'rm -f "$chooser"' EXIT
yazi --chooser-file="$chooser"
[ -s "$chooser" ] || exit 0

result=""
while IFS= read -r f || [ -n "$f" ]; do
  result="$result@${f/#$HOME/~} "
done < "$chooser"

"${HERDR_BIN_PATH:-herdr}" pane send-text "$pane_id" "${result% }"
