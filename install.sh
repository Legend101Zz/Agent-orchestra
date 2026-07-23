#!/usr/bin/env bash
# pi-orchestra Rust-only installer: locked build, safe links, additive shell blocks.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "${1:-}" in
  "") ;;
  -h|--help)
    echo "usage: ./install.sh"
    echo "builds and installs the Rust pio, piod, and pi-orchestra binaries"
    exit 0
    ;;
  *) echo "install.sh: unknown option: $1" >&2; exit 2 ;;
esac

TARGET_DIR="${ORC_INSTALL_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-$HOME/.local/share/pi-orchestra/target}}"
if [ "${ORC_INSTALL_SKIP_BUILD:-0}" != 1 ]; then
  echo "==> locked Rust release build"
  CARGO_TARGET_DIR="$TARGET_DIR" cargo build --manifest-path "$ROOT/rust/Cargo.toml" --release --locked
fi
BIN_DIR="${ORC_INSTALL_BIN_DIR:-$TARGET_DIR/release}"
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

# `orc`/`orcd` were renamed to `pio`/`piod` (issue #17). Leave a forwarding shim
# at the old name so existing muscle memory and scripts keep working while
# nudging toward the new command. A pre-existing command that is not already our
# shim is backed up once, mirroring install_link.
RENAME_SHIM_MARK='pi-orchestra-rename-shim'
retire_command() {
  local old="$1"
  local new="$2"
  local destination="$DEST_DIR/$old"
  if { [ -e "$destination" ] || [ -L "$destination" ]; } \
     && ! grep -qF "$RENAME_SHIM_MARK" "$destination" 2>/dev/null \
     && [ ! -e "$destination.pi-orchestra.bak" ] && [ ! -L "$destination.pi-orchestra.bak" ]; then
    mv "$destination" "$destination.pi-orchestra.bak"
    echo "    backed up old $destination"
  fi
  # rm first so we never write through a surviving symlink into its target.
  rm -f "$destination"
  cat > "$destination" <<EOF
#!/usr/bin/env bash
# $RENAME_SHIM_MARK
# '$old' was renamed to '$new' (pi-orchestra issue #17); forwarding for now.
echo "pi-orchestra: '$old' is now '$new' — forwarding this call. Please switch to '$new'." >&2
exec "\$(dirname "\$0")/$new" "\$@"
EOF
  chmod +x "$destination"
  echo "    installed $old → $new shim"
}

echo "==> command links"
install_link pio
install_link piod
install_link pi-orchestra

echo "==> retiring old orc/orcd names"
retire_command orc pio
retire_command orcd piod

echo "==> running daemon check"
# piod persists across installs; a daemon on an older build makes clients
# fail their build handshake until it is restarted.
DAEMON_RC=0
"$DEST_DIR/pio" daemon status >/dev/null 2>&1 || DAEMON_RC=$?
case "$DAEMON_RC" in
  0) echo "    piod is running the installed build" ;;
  3) echo "    piod is not running (it starts on demand)" ;;
  5)
    echo "    WARNING: the running daemon predates this install."
    echo "    Detach clients, then run: pio daemon restart"
    echo "    (live panes die with the daemon; the command lists them first)"
    ;;
  *) echo "    could not probe the daemon (pio daemon status exit $DAEMON_RC)" ;;
esac

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
install_skill() {
  local name="$1"
  local source="$ROOT/skills/$name"
  local destination="$HOME/.claude/skills/$name"
  [ -d "$source" ] || return 0
  if [ -L "$destination" ]; then
    if [ "$(readlink "$destination")" = "$source" ]; then
      return 0
    fi
    if [ ! -e "$destination" ]; then
      # A dangling link (for example an old checkout that moved) teaches
      # nothing; replacing it restores the skill without touching content.
      rm "$destination"
      ln -s "$source" "$destination"
      echo "    replaced dead symlink $destination"
      return 0
    fi
    echo "    kept user symlink $destination" >&2
    return 0
  fi
  if [ -e "$destination" ]; then
    echo "    kept user content $destination" >&2
    return 0
  fi
  ln -s "$source" "$destination"
}
for skill in pi-delegate orchestrate; do
  install_skill "$skill"
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
    cp "$AGENTS" "$AGENTS.pi-orchestra.bak"
    sed -i '' '/<!-- pi-orchestra:begin -->/,/<!-- pi-orchestra:end -->/d' "$AGENTS"
    printf '\n' >> "$AGENTS"
    sed -n '1,$p' "$ROOT/codex/AGENTS-block.md" >> "$AGENTS"
    echo "    refreshed owned block (backup: $AGENTS.pi-orchestra.bak)"
  fi
fi

echo "==> protected-config checksums"
shasum -a 256 "$HOME/.pi/agent/settings.json" "$HOME/.claude/settings.json" \
  "$HOME/.codex/config.toml" "$HOME/.local/bin/pio" 2>/dev/null || true
echo "done. Open a new shell or run: source ~/.zshrc"
