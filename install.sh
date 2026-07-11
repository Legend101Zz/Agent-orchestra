#!/usr/bin/env bash
# pi-orchestra Rust-only installer: locked build, safe links, additive shell blocks.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "${1:-}" in
  "") ;;
  -h|--help)
    echo "usage: ./install.sh"
    echo "builds and installs the Rust orc, orcd, and pi-orchestra binaries"
    exit 0
    ;;
  *) echo "install.sh: unknown option: $1" >&2; exit 2 ;;
esac

if [ "${ORC_INSTALL_SKIP_BUILD:-0}" != 1 ]; then
  echo "==> locked Rust release build"
  cargo build --manifest-path "$ROOT/rust/Cargo.toml" --release --locked
fi
BIN_DIR="${ORC_INSTALL_BIN_DIR:-$ROOT/rust/target/release}"
DEST_DIR="$HOME/.local/bin"
mkdir -p "$DEST_DIR"

install_link() {
  local name="$1"
  local target="$BIN_DIR/$name"
  local destination="$DEST_DIR/$name"
  [ -x "$target" ] || { echo "install.sh: missing executable $target" >&2; exit 1; }
  if [ -e "$destination" ] || [ -L "$destination" ]; then
    local current=""
    current="$(readlink "$destination" 2>/dev/null || true)"
    if [ "$current" != "$target" ] && [ ! -e "$destination.pi-orchestra.bak" ] && [ ! -L "$destination.pi-orchestra.bak" ]; then
      mv "$destination" "$destination.pi-orchestra.bak"
      echo "    backed up $destination"
    fi
  fi
  ln -sfn "$target" "$destination"
}

echo "==> command links"
install_link orc
install_link orcd
install_link pi-orchestra

echo "==> private orchestra data directory"
mkdir -p "$HOME/.orchestra/runs" "$HOME/.orchestra/sessions"
chmod 700 "$HOME/.orchestra"
if [ ! -f "$HOME/.orchestra/config.json" ]; then
  printf '%s\n' '{"warn_pct":25,"block_pct":10,"cache_ttl_sec":60,"max_parallel_workers":3,"idle_timeout_sec":300,"theme":"ember"}' > "$HOME/.orchestra/config.json"
fi

echo "==> ~/.zshrc marked block"
RC="$HOME/.zshrc"
MARK='# >>> pi-orchestra >>>'
touch "$RC"
if ! grep -qF "$MARK" "$RC"; then
  cp "$RC" "$RC.pi-orchestra.bak"
  printf '\n%s\nsource "%s/shell/orchestra.zsh"\n%s\n' "$MARK" "$ROOT" '# <<< pi-orchestra <<<' >> "$RC"
  echo "    appended (backup: $RC.pi-orchestra.bak)"
else
  echo "    already present"
fi

echo "==> Claude Code skills"
mkdir -p "$HOME/.claude/skills"
for skill in pi-delegate orchestrate; do
  [ -d "$ROOT/skills/$skill" ] && ln -sfn "$ROOT/skills/$skill" "$HOME/.claude/skills/$skill"
done

echo "==> Codex AGENTS.md block"
AGENTS="$HOME/.codex/AGENTS.md"
if [ -f "$ROOT/codex/AGENTS-block.md" ]; then
  mkdir -p "$HOME/.codex"
  touch "$AGENTS"
  if ! grep -qF '<!-- pi-orchestra:begin -->' "$AGENTS"; then
    cp "$AGENTS" "$AGENTS.pi-orchestra.bak"
    printf '\n' >> "$AGENTS"
    sed -n '1,$p' "$ROOT/codex/AGENTS-block.md" >> "$AGENTS"
    echo "    appended (backup: $AGENTS.pi-orchestra.bak)"
  else
    echo "    already present"
  fi
fi

echo "==> protected-config checksums"
shasum -a 256 "$HOME/.pi/agent/settings.json" "$HOME/.claude/settings.json" \
  "$HOME/.codex/config.toml" "$HOME/.local/bin/orc" 2>/dev/null || true
echo "done. Open a new shell or run: source ~/.zshrc"
