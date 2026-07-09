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
  orc run "$task" --cwd "$cwd" --brain "${ORC_BRAIN:-human}"
}

# pi-rpc: streaming delegation (JSON-RPC) via orc; Ctrl+C cancels; kill via `orc kill <id>`
# Usage: pi-rpc "task"
pi-rpc() {
  local task="$1"
  if [[ -z "$task" ]]; then
    echo 'Usage: pi-rpc "<task>"' >&2
    return 1
  fi
  orc rpc "$task" --brain "${ORC_BRAIN:-human}"
}
