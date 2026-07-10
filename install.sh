#!/usr/bin/env bash
# pi-orchestra installer — additive only; backs up before any append; idempotent.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
USE_RUST=0
case "${1:-}" in
  "") ;;
  --rust) USE_RUST=1 ;;
  -h|--help)
    echo "usage: ./install.sh [--rust]"
    echo "  default  install the Python/Textual implementation"
    echo "  --rust   build and install rust/target/release/orc"
    exit 0
    ;;
  *) echo "install.sh: unknown option: $1" >&2; exit 2 ;;
esac

if [ "$USE_RUST" -eq 1 ]; then
  echo "==> Rust release build"
  cargo build --manifest-path "$ROOT/rust/Cargo.toml" --release --locked
  ORC_TARGET="$ROOT/rust/target/release/orc"
else
  echo "==> venv + deps"
  [ -d "$ROOT/.venv" ] || python3 -m venv "$ROOT/.venv"
  "$ROOT/.venv/bin/pip" -q install -U pip
  "$ROOT/.venv/bin/pip" -q install -r "$ROOT/requirements.txt"
  ORC_TARGET="$ROOT/bin/orc"
fi

echo "==> orc symlink"
mkdir -p "$HOME/.local/bin"
chmod +x "$ORC_TARGET"
ln -sfn "$ORC_TARGET" "$HOME/.local/bin/orc"

echo "==> ~/.orchestra"
mkdir -p "$HOME/.orchestra/runs"
if [ ! -f "$HOME/.orchestra/config.json" ]; then
  cat > "$HOME/.orchestra/config.json" <<'EOF'
{
  "warn_pct": 25,
  "block_pct": 10,
  "cache_ttl_sec": 60,
  "max_parallel_workers": 3,
  "idle_timeout_sec": 300
}
EOF
fi

echo "==> ~/.zshrc block"
RC="$HOME/.zshrc"
MARK='# >>> pi-orchestra >>>'
if ! grep -qF "$MARK" "$RC" 2>/dev/null; then
  cp "$RC" "$RC.pi-orchestra.bak"
  {
    echo ""
    echo "$MARK"
    echo "source \"$ROOT/shell/orchestra.zsh\""
    echo '# <<< pi-orchestra <<<'
  } >> "$RC"
  echo "    appended (backup: $RC.pi-orchestra.bak)"
else
  echo "    already present"
fi

echo "==> Claude Code skills"
mkdir -p "$HOME/.claude/skills"
for s in pi-delegate orchestrate; do
  [ -d "$ROOT/skills/$s" ] && ln -sfn "$ROOT/skills/$s" "$HOME/.claude/skills/$s"
done

echo "==> Codex AGENTS.md block"
A="$HOME/.codex/AGENTS.md"
if [ -f "$ROOT/codex/AGENTS-block.md" ]; then
  mkdir -p "$HOME/.codex"
  touch "$A"
  if ! grep -qF '<!-- pi-orchestra:begin -->' "$A"; then
    cp "$A" "$A.pi-orchestra.bak"
    cat "$ROOT/codex/AGENTS-block.md" >> "$A"
    echo "    appended (backup: $A.pi-orchestra.bak)"
  else
    echo "    already present"
  fi
fi

echo "==> protected-config checksums (must match pre-install values)"
shasum -a 256 "$HOME/.pi/agent/settings.json" "$HOME/.pi/agent/auth.json" \
  "$HOME/.codex/config.toml" 2>/dev/null || true

echo "done. Open a new shell or: source ~/.zshrc"
