# pi-orchestra shell helpers — sourced from ~/.zshrc marked block.

# deleg8: fire-and-forget delegation to pi + MiniMax M3 via orc (registered + quota-gated)
# Usage: deleg8 "your task description" [/path/to/cwd]
deleg8() {
  local task="$1"
  local cwd="${2:-$PWD}"
  if [[ -z "$task" ]]; then
    echo 'Usage: deleg8 "<task>" [cwd]' >&2
    return 1
  fi
  local -a session_args
  [[ -n "${ORC_SESSION:-}" ]] && session_args=(--session "$ORC_SESSION")
  orc run "$task" --cwd "$cwd" --brain "${ORC_BRAIN:-human}" "${session_args[@]}"
}

# pi-rpc: streaming delegation (JSON-RPC) via orc; Ctrl+C cancels; kill via `orc kill <id>`
# Usage: pi-rpc "task"
pi-rpc() {
  local task="$1"
  if [[ -z "$task" ]]; then
    echo 'Usage: pi-rpc "<task>"' >&2
    return 1
  fi
  local -a session_args
  [[ -n "${ORC_SESSION:-}" ]] && session_args=(--session "$ORC_SESSION")
  orc rpc "$task" --brain "${ORC_BRAIN:-human}" "${session_args[@]}"
}

# bench-dispatch: confirmed non-interactive delivery to an offered Bench worker.
# Usage: bench-dispatch T0001 hermes worker-pane "bounded brief"
bench-dispatch() {
  local task="$1" harness="$2" pane="$3" brief="$4"
  if [[ -z "${ORC_SESSION:-}" || -z "$task" || -z "$harness" || -z "$pane" || -z "$brief" ]]; then
    echo 'Usage: ORC_SESSION=<id> bench-dispatch <task> <harness> <pane> "brief"' >&2
    return 1
  fi
  orc dispatch send "$task" "$harness" "$brief" --session "$ORC_SESSION" \
    --pane "$pane" --actor "${ORC_ACTOR:-brain}" --json
}
