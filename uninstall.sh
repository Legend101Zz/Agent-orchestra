#!/usr/bin/env bash
# pi-orchestra uninstaller — removes symlinks and marked blocks; keeps ~/.orchestra data.
set -euo pipefail

rm -f "$HOME/.local/bin/orc"
rm -f "$HOME/.claude/skills/pi-delegate" "$HOME/.claude/skills/orchestrate"

RC="$HOME/.zshrc"
if grep -qF '# >>> pi-orchestra >>>' "$RC" 2>/dev/null; then
  cp "$RC" "$RC.pi-orchestra.uninstall.bak"
  sed -i '' '/# >>> pi-orchestra >>>/,/# <<< pi-orchestra <<</d' "$RC"
fi

A="$HOME/.codex/AGENTS.md"
if [ -f "$A" ] && grep -qF '<!-- pi-orchestra:begin -->' "$A"; then
  cp "$A" "$A.pi-orchestra.uninstall.bak"
  sed -i '' '/<!-- pi-orchestra:begin -->/,/<!-- pi-orchestra:end -->/d' "$A"
fi

echo "uninstalled (kept ~/.orchestra data and the repo). Backups: *.pi-orchestra.uninstall.bak"
