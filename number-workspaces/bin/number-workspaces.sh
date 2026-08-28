#!/bin/bash
set -eo pipefail

# herdr's `number` field reflects creation order, not display order.
# Regroups worktrees under root first, mirroring sidebar nesting rule (src/ui/sidebar.rs), then renumbers by position.
ws_json=$(herdr workspace list)

renumbered=$(echo "$ws_json" | jq -c '
  .result.workspaces as $all
  | ([$all[] | select(.worktree != null) | .worktree.repo_key] | group_by(.) | map(select(length >= 2) | .[0])) as $multi_keys
  | ([$all[] | select(.worktree.is_linked_worktree == false) | .worktree.repo_key]) as $root_keys
  | ($multi_keys | map(select(IN($root_keys[])))) as $nested_keys
  | $all
  | sort_by(.number)
  | group_by(if .worktree != null and (.worktree.repo_key | IN($nested_keys[])) then .worktree.repo_key else .workspace_id end)
  | map(if length > 1 then ([.[] | select(.worktree.is_linked_worktree == false)] + [.[] | select(.worktree.is_linked_worktree != false)]) else . end)
  | sort_by(.[0].number)
  | flatten
  | to_entries
  | map({workspace_id: .value.workspace_id, number: (.key + 1)})
')

echo "$renumbered" | jq -r '.[] | "\(.workspace_id)\t\(.number)"' |
while IFS=$'\t' read -r id num; do
  herdr workspace report-metadata "$id" --source herdr-number --token num="$num"
done

herdr pane list | jq -r --argjson nums "$renumbered" '
  ($nums | map({key: .workspace_id, value: .number}) | from_entries) as $num_by_ws
  | .result.panes[] | "\(.pane_id)\t\($num_by_ws[.workspace_id])"
' |
while IFS=$'\t' read -r pane_id num; do
  herdr pane report-metadata "$pane_id" --source herdr-number --token num="$num"
done
