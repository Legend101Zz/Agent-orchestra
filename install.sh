#!/usr/bin/env bash
# pi-orchestra Rust-only installer: locked build, safe links, additive shell blocks.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

WIRE_CLAUDE_HOOK="${ORC_INSTALL_WIRE_CLAUDE_HOOK:-0}"
while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help)
      echo "usage: ./install.sh [--wire-claude-hook]"
      echo "builds and installs the Rust pio, piod, and pi-orchestra binaries"
      echo
      echo "  --wire-claude-hook  register the UserPromptSubmit trigger hook in"
      echo "                      ~/.claude/settings.json. Opt-in and off by"
      echo "                      default: that file is yours. Backs it up first,"
      echo "                      merges rather than overwrites, and does nothing"
      echo "                      at all if the hook is already registered."
      exit 0
      ;;
    --wire-claude-hook) WIRE_CLAUDE_HOOK=1 ;;
    *) echo "install.sh: unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

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
install_link pio-mcp

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
# No `theme` here: harnesses.json's app.theme is the authoritative record
# (issue #37), and config.json's copy is derived from it on the first write.
# Seeding a second, disagreeing value is what made `pio config set theme`
# look like a no-op.
if [ ! -f "$HOME/.orchestra/config.json" ]; then
  printf '%s\n' '{"warn_pct":25,"block_pct":10,"cache_ttl_sec":60,"max_parallel_workers":3,"idle_timeout_sec":300}' > "$HOME/.orchestra/config.json"
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
for skill in pi-delegate orchestrate deliberate; do
  install_skill "$skill"
done

echo "==> Claude Code trigger hook"
# The hook lives in a pi-orchestra-owned dir. Registering it means editing
# ~/.claude/settings.json, which belongs to the user — so that stays opt-in
# behind --wire-claude-hook (issue #10 AC1 keeps the no-flag checksums
# identical across runs). What is NOT acceptable, and was the state issue #45
# found, is finishing quietly while the headline gesture is inert: whichever
# path we take, the summary at the end says plainly whether `delegate:` works.
HOOK_SRC="$ROOT/shell/claude-userpromptsubmit-hook.py"
HOOK_DIR="$HOME/.claude/pi-orchestra"
HOOK_LINK="$HOOK_DIR/claude-userpromptsubmit-hook.py"
SETTINGS="$HOME/.claude/settings.json"
HOOK_SNIPPET="      { \"hooks\": { \"UserPromptSubmit\": [ { \"hooks\": [ { \"type\": \"command\",
          \"command\": \"$HOOK_LINK\" } ] } ] } }"
CLAUDE_HOOK_STATE="absent"

# Read-only: is our hook already referenced by settings.json? Never writes,
# and treats a missing, empty or unparseable file as "not wired" rather than
# failing the install.
claude_hook_registered() {
  [ -f "$SETTINGS" ] || return 1
  HOOK_LINK="$HOOK_LINK" SETTINGS="$SETTINGS" python3 - <<'PY'
import json, os, sys
try:
    with open(os.environ["SETTINGS"], encoding="utf-8") as handle:
        settings = json.load(handle)
except Exception:
    sys.exit(1)
target = os.path.basename(os.environ["HOOK_LINK"])
entries = (settings.get("hooks") or {}).get("UserPromptSubmit") or []
for entry in entries if isinstance(entries, list) else []:
    for hook in (entry.get("hooks") or []) if isinstance(entry, dict) else []:
        if isinstance(hook, dict) and target in str(hook.get("command", "")):
            sys.exit(0)
sys.exit(1)
PY
}

# Opt-in, backed up, idempotent: merges one UserPromptSubmit entry and leaves
# every other key — and every other hook — exactly as it found them.
wire_claude_hook() {
  cp "$SETTINGS" "$SETTINGS.pi-orchestra.bak" 2>/dev/null || true
  HOOK_LINK="$HOOK_LINK" SETTINGS="$SETTINGS" python3 - <<'PY'
import json, os, sys
path, command = os.environ["SETTINGS"], os.environ["HOOK_LINK"]
try:
    with open(path, encoding="utf-8") as handle:
        text = handle.read().strip()
    settings = json.loads(text) if text else {}
except FileNotFoundError:
    settings = {}
except Exception as error:
    print(f"    refused to edit {path}: {error}", file=sys.stderr)
    print("    it is not valid JSON; add the snippet by hand.", file=sys.stderr)
    sys.exit(2)
if not isinstance(settings, dict):
    print(f"    refused to edit {path}: top level is not an object", file=sys.stderr)
    sys.exit(2)
hooks = settings.setdefault("hooks", {})
submit = hooks.setdefault("UserPromptSubmit", [])
for entry in submit:
    for hook in (entry.get("hooks") or []) if isinstance(entry, dict) else []:
        if isinstance(hook, dict) and hook.get("command") == command:
            sys.exit(0)  # already wired: leave the bytes untouched
submit.append({"hooks": [{"type": "command", "command": command}]})
with open(path, "w", encoding="utf-8") as handle:
    json.dump(settings, handle, indent=2)
    handle.write("\n")
PY
}

if [ -f "$HOOK_SRC" ]; then
  mkdir -p "$HOOK_DIR"
  if [ -e "$HOOK_LINK" ] && [ ! -L "$HOOK_LINK" ]; then
    echo "    kept user file $HOOK_LINK" >&2
  else
    ln -sfn "$HOOK_SRC" "$HOOK_LINK"
    echo "    linked $HOOK_LINK"
  fi
  if claude_hook_registered; then
    CLAUDE_HOOK_STATE="wired"
    echo "    already registered in $SETTINGS"
  elif [ "$WIRE_CLAUDE_HOOK" = 1 ]; then
    mkdir -p "$HOME/.claude"
    if wire_claude_hook; then
      CLAUDE_HOOK_STATE="wired"
      echo "    registered in $SETTINGS (backup: $SETTINGS.pi-orchestra.bak)"
    else
      CLAUDE_HOOK_STATE="unwired"
    fi
  else
    CLAUDE_HOOK_STATE="unwired"
  fi
fi

echo "==> Codex AGENTS.md block"
AGENTS="$HOME/.codex/AGENTS.md"
if [ -f "$ROOT/codex/AGENTS-block.md" ]; then
  mkdir -p "$HOME/.codex"
  touch "$AGENTS"
  # Trim trailing blank lines before re-appending: the owned block carries its
  # own leading separator, so without this every refresh left the old separator
  # behind and grew the user's file by a blank line per install (issue #10
  # review). Command substitution strips trailing newlines; printf restores one.
  trim_trailing_blanks() {
    local text
    text="$(cat "$1")"
    printf '%s\n' "$text" > "$1.pi-orchestra.trim" && mv "$1.pi-orchestra.trim" "$1"
  }
  cp "$AGENTS" "$AGENTS.pi-orchestra.bak"
  if ! grep -qF '<!-- pi-orchestra:begin -->' "$AGENTS"; then
    ACTION="appended"
  else
    sed -i '' '/<!-- pi-orchestra:begin -->/,/<!-- pi-orchestra:end -->/d' "$AGENTS"
    ACTION="refreshed owned block"
  fi
  trim_trailing_blanks "$AGENTS"
  cat "$ROOT/codex/AGENTS-block.md" >> "$AGENTS"
  echo "    $ACTION (backup: $AGENTS.pi-orchestra.bak)"
fi

echo "==> protected-config checksums"
shasum -a 256 "$HOME/.pi/agent/settings.json" "$HOME/.claude/settings.json" \
  "$HOME/.codex/config.toml" "$HOME/.local/bin/pio" 2>/dev/null || true

# The trigger grammar, per harness, LAST — so it is on screen when the install
# finishes rather than scrolled away. `pio doctor` marks four harnesses
# conductor-capable; only two of them have an integration surface we can
# actually write to, and saying so is the whole point of this block. Issue #45
# check 10: wire every conductor-capable harness, or state which are not and
# what the user must do.
echo
echo "==> trigger grammar (delegate: / orchestrate: / deliberate:)"
harness_installed() { command -v "$1" >/dev/null 2>&1; }
report_harness() {
  # name, whether present, state word, remedy
  local label="$1" present="$2" state="$3" remedy="$4"
  if [ "$present" != 1 ]; then
    printf '    %-13s not installed  —\n' "$label"
    return
  fi
  printf '    %-13s %s\n' "$label" "$state"
  [ -n "$remedy" ] && printf '    %-13s   %s\n' "" "$remedy"
  return 0
}

if [ "$CLAUDE_HOOK_STATE" = "wired" ]; then
  report_harness "Claude Code" "$(harness_installed claude && echo 1 || echo 0)" \
    "WIRED — skills + live hook" ""
else
  report_harness "Claude Code" "$(harness_installed claude && echo 1 || echo 0)" \
    "skills linked, hook NOT registered" \
    "\`delegate:\` will NOT fire until it is. Re-run: ./install.sh --wire-claude-hook"
  echo
  echo "    Or add this to $SETTINGS yourself:"
  echo "$HOOK_SNIPPET"
  echo
fi
report_harness "Codex" "$(harness_installed codex && echo 1 || echo 0)" \
  "WIRED — static block in ~/.codex/AGENTS.md" \
  "static text only: no live quota relay and no session context"
report_harness "Pi/MiniMax" "$(harness_installed pi && echo 1 || echo 0)" \
  "NOT wired" \
  "pi has no integration pi-orchestra installs. Paste skills/pi-delegate/SKILL.md into your pi agent instructions by hand."
report_harness "OpenCode" "$(harness_installed opencode && echo 1 || echo 0)" \
  "NOT wired" \
  "opencode has no integration pi-orchestra installs. Paste skills/pi-delegate/SKILL.md into its instructions by hand."
echo "    Hermes        worker only — needs no trigger grammar"
echo
echo "    Verify any time with: pio doctor   (exit 1 while the grammar is inert)"
echo "done. Open a new shell or run: source ~/.zshrc"
