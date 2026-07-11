#!/usr/bin/env bash
# Removes installed links and marked blocks; preserves ~/.orchestra data.
set -euo pipefail

remove_link() {
  local destination="$HOME/.local/bin/$1"
  if [ -L "$destination" ]; then
    rm "$destination"
    if [ -e "$destination.pi-orchestra.bak" ] || [ -L "$destination.pi-orchestra.bak" ]; then
      mv "$destination.pi-orchestra.bak" "$destination"
    fi
  elif [ -e "$destination" ]; then
    echo "kept non-symlink $destination" >&2
  fi
}

remove_link orc
remove_link orcd
remove_link pi-orchestra
rm -f "$HOME/.claude/skills/pi-delegate" "$HOME/.claude/skills/orchestrate"

RC="$HOME/.zshrc"
if grep -qF '# >>> pi-orchestra >>>' "$RC" 2>/dev/null; then
  cp "$RC" "$RC.pi-orchestra.uninstall.bak"
  sed -i '' '/# >>> pi-orchestra >>>/,/# <<< pi-orchestra <<</d' "$RC"
fi

AGENTS="$HOME/.codex/AGENTS.md"
if [ -f "$AGENTS" ] && grep -qF '<!-- pi-orchestra:begin -->' "$AGENTS"; then
  cp "$AGENTS" "$AGENTS.pi-orchestra.uninstall.bak"
  sed -i '' '/<!-- pi-orchestra:begin -->/,/<!-- pi-orchestra:end -->/d' "$AGENTS"
fi

echo "uninstalled Rust command links and marked blocks; kept ~/.orchestra data"
