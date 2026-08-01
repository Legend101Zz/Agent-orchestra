#!/usr/bin/env bash
# Resolve every in-repo path citation and report the ones that do not exist.
#
# Written for issue #14, where `docs/` was restructured and 40 citations had to
# be repointed. Checking that by eye is exactly the failure mode the issue is
# about, so this is a command with an exit code instead.
#
# What counts as a citation, in every tracked text file:
#   1. Markdown links   [text](path)          — excluding http(s):, mailto:, #anchors
#   2. Markdown images  ![alt](path)          — same exclusions
#   3. Backticked paths `docs/...` `tools/...` `rust/...` `skills/...` `shell/...`
#      `codex/...`, plus bare top-level files like `AGENTS.md`
#
# A citation is resolved relative to the citing file's own directory first
# (markdown-link semantics), then relative to the repo root (how this repo
# writes prose citations, e.g. `docs/archive/notes/foo.md` inside progress.md).
# It passes if either resolves. That mirrors how a reader actually follows them.
#
# Usage:  tools/check-doc-links.sh            # check, print a summary, exit 1 if broken
#         tools/check-doc-links.sh --list     # also list every citation checked
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

LIST=0
[ "${1:-}" = "--list" ] && LIST=1

checked=0
broken=0
historical=0
ignored=0
anchors=0
broken_lines=()
historical_lines=()
ignored_lines=()

# Tracked text files only: never walk target/ or .git, never test a binary.
# Kept bash-3.2 compatible (no mapfile): macOS ships bash 3.2 as /bin/bash and
# this repo supports macOS.
FILE_LIST="$(mktemp)"
CITES="$(mktemp)"
CITES_U="$(mktemp)"
trap 'rm -f "$FILE_LIST" "$CITES" "$CITES_U"; rm -rf "${SLUG_CACHE:-}"' EXIT
git ls-files -- '*.md' '*.rs' '*.sh' '*.zsh' '*.toml' '*.html' | sort > "$FILE_LIST"

is_ignorable() {
  case "$1" in
    http://*|https://*|mailto:*|'#'*|'') return 0 ;;
    # Bare command/URL fragments and format placeholders, not paths.
    *'{'*|*'}'*|*'$'*|*'<'*|*'>'*|*'*'*) return 0 ;;
    # rustdoc intra-doc links (`[crate::bench::HarnessRegistry]`) are Rust item
    # paths, not files.
    *'::'*) return 0 ;;
  esac
  return 1
}

# A backticked token is a repo path citation only if it is rooted at a real
# top-level directory AND looks like a file or a directory. Without the second
# half this fires on MCP method names (`tools/list`, `tools/call`), which are
# JSON-RPC verbs that merely share the `tools/` prefix.
#
# Bare filenames are deliberately NOT citations: `harnesses.json`,
# `config.json`, `session.json` and `meta.json` name runtime files under
# `~/.orchestra`, not files in this repo, and `Cargo.toml` is ambiguous between
# eight crates. Root-level repo docs are allowlisted instead.
is_repo_path() {
  case "$1" in
    docs/*|tools/*|rust/*|skills/*|shell/*|codex/*)
      case "$1" in
        */) return 0 ;;      # directory citation, e.g. `docs/archive/notes/`
        *.*) return 0 ;;     # has an extension
        *) return 1 ;;       # `tools/list` — a method name, not a path
      esac ;;
    AGENTS.md|README.md|LOG.md|WORKFLOW.md|ANTI-SLOP.md) return 0 ;;
    task_plan.md|findings.md|progress.md|LICENSE) return 0 ;;
  esac
  return 1
}

# GitHub's heading slug: strip markdown, lowercase, drop punctuation, spaces to
# hyphens. Good enough for the headings this repo actually writes.
#
# Slugs are computed once per file and cached, not once per (anchor, heading)
# pair. The obvious shell-loop version spawned three processes per heading per
# anchor and took 25 s on this repo; this takes well under a second.
SLUG_CACHE="$(mktemp -d)"
slug_file() {
  local mdfile="$1"
  local key="$SLUG_CACHE/$(printf '%s' "$mdfile" | tr '/' '_')"
  if [ ! -f "$key" ]; then
    grep -E '^#{1,6}[[:space:]]' "$mdfile" 2>/dev/null \
      | sed -E 's/^#+[[:space:]]*//; s/`//g; s/\*\*//g; s/\*//g; s/\[([^]]*)\]\([^)]*\)/\1/g' \
      | tr '[:upper:]' '[:lower:]' \
      | sed -E 's/[^a-z0-9 _-]//g; s/[[:space:]]+/-/g; s/^-+//; s/-+$//' \
      > "$key"
  fi
  printf '%s' "$key"
}

# Validate an #anchor against the file it points into. Same-file anchors (the
# README's own Contents list) are the ones most likely to rot on a rename, so
# they are checked too.
check_anchor() {
  local citing="$1" line="$2" target_file="$3" anchor="$4"
  [ -n "$anchor" ] || return 0
  case "$target_file" in *.md) ;; *) return 0 ;; esac
  [ -f "$target_file" ] || return 0
  anchors=$((anchors + 1))
  if ! grep -qxF "$anchor" "$(slug_file "$target_file")"; then
    broken=$((broken + 1))
    broken_lines+=("$(printf 'BROKEN %s:%s  (anchor)  -> %s#%s (no such heading)' "$citing" "$line" "$target_file" "$anchor")")
    return 1
  fi
  return 0
}

record() {
  local file="$1" line="$2" target="$3" how="$4" anchor="${5:-}"
  checked=$((checked + 1))
  local dir base_rel base_root
  dir="$(dirname "$file")"
  base_rel="$dir/$target"
  base_root="$target"
  # A gitignored path is build output, not a repository file, so whether it
  # exists depends on whether the reader has run cargo — not on whether the
  # citation is correct. Checking it against the working tree makes this gate
  # pass on a built tree and fail on a fresh clone, which is exactly backwards
  # for a gate meant to certify that nothing is stale. `rust/target/` was that
  # bug: it passed here and exited 1 on a clean checkout.
  if git check-ignore -q "$base_root" 2>/dev/null \
     || git check-ignore -q "$base_rel" 2>/dev/null; then
    ignored=$((ignored + 1))
    ignored_lines+=("$(printf 'build-output %s:%s  -> %s (gitignored; not a repository file)' "$file" "$line" "$target")")
    return 0
  fi
  if [ -e "$base_rel" ] || [ -e "$base_root" ]; then
    [ "$LIST" = 1 ] && printf '  ok    %s:%s  %s\n' "$file" "$line" "$target"
    if [ -f "$base_rel" ]; then check_anchor "$file" "$line" "$base_rel" "$anchor"
    elif [ -f "$base_root" ]; then check_anchor "$file" "$line" "$base_root" "$anchor"; fi
    return 0
  fi
  # A document under docs/archive/ is a frozen record. When it cites something
  # that has since been deleted from the live tree — the Python capture scripts,
  # for instance — that citation is a historical statement about what existed
  # when it was written. Editing it would falsify the record, so it is reported
  # and does not fail the check.
  #
  # The archive's *internal* consistency is still enforced: an archived file
  # citing another archived file, or archived media, must resolve. That is what
  # keeps the archive usable rather than merely present.
  case "$file" in
    docs/archive/*)
      case "$target" in
        docs/archive/*) ;;   # internal: falls through to a hard failure
        *)
          historical=$((historical + 1))
          historical_lines+=("$(printf 'historical %s:%s  -> %s (target no longer in the live tree)' "$file" "$line" "$target")")
          return 0 ;;
      esac ;;
  esac
  broken=$((broken + 1))
  broken_lines+=("$(printf 'BROKEN %s:%s  (%s)  -> %s' "$file" "$line" "$how" "$target")")
  return 1
}

while IFS= read -r file; do
  [ -f "$file" ] || continue
  # This script cannot scan itself: its own comments and regex literals contain
  # `](path)` fragments and an illustrative `docs/archive/notes/foo.md`, all of
  # which are descriptions of the syntax rather than citations of it.
  case "$file" in tools/check-doc-links.sh) continue ;; esac

  # Strip fenced code blocks before scanning. Text inside a fence is an example
  # or a transcript, not a citation — and Python like
  # `{...}[args.cmd](args)` is otherwise read as a markdown link to `args`.
  # Line numbers are preserved by blanking rather than deleting.
  SCAN="$(mktemp)"
  awk '/^[[:space:]]*```/ {inblock = !inblock; print ""; next} {print inblock ? "" : $0}' \
      "$file" > "$SCAN"

  # 1 + 2. Markdown links and images: [..](target) / ![..](target)
  #
  # One `grep -noE` over the whole file rather than a per-line loop that pipes
  # each line through two more greps in subshells. The loop version cost ~20 s
  # on this repo; this is a fraction of that, and the tool is meant to be run
  # often enough that nobody thinks twice about it.
  while IFS= read -r hit; do
    lineno="${hit%%:*}"; payload="${hit#*:}"
    target="${payload#](}"
    anchor="${target#*#}"; [ "$anchor" = "$target" ] && anchor=""
    target="${target%%#*}"
    if [ -z "$target" ] && [ -n "$anchor" ]; then
      printf '%s\t%s\tSELF\tsame-file-anchor\t%s\n' "$file" "$lineno" "$anchor"
      continue
    fi
    is_ignorable "$target" && continue
    printf '%s\t%s\t%s\tmd-link\t%s\n' "$file" "$lineno" "$target" "$anchor"
  done < <(grep -noE '\]\([^)[:space:]]+' "$SCAN" 2>/dev/null)

  # 3. Backticked repo paths.
  while IFS= read -r hit; do
    lineno="${hit%%:*}"; target="${hit#*:}"
    target="${target//\`/}"
    target="${target%%#*}"
    target="${target%,}"; target="${target%.}"; target="${target%:}"; target="${target%;}"
    is_ignorable "$target" && continue
    is_repo_path "$target" || continue
    printf '%s\t%s\t%s\tbacktick\t\n' "$file" "$lineno" "$target"
  done < <(grep -noE '`[A-Za-z0-9_./-]+`' "$SCAN" 2>/dev/null)
  rm -f "$SCAN"
done < "$FILE_LIST" > "$CITES" 2>/dev/null

# De-duplicate identical (file, target) pairs so a path cited twice on one line
# is not counted twice.
sort -u "$CITES" > "$CITES_U"
while IFS="$(printf '\t')" read -r file lineno target how anchor; do
  [ -n "${file:-}" ] || continue
  if [ "$how" = "same-file-anchor" ]; then
    checked=$((checked + 1))
    check_anchor "$file" "$lineno" "$file" "$anchor"
    continue
  fi
  record "$file" "$lineno" "$target" "$how" "${anchor:-}"
done < "$CITES_U"

echo
echo "citations checked : $checked"
echo "broken            : $broken"
echo "historical        : $historical  (archived docs citing since-deleted files; not a failure)"
echo "anchors resolved  : $anchors"
echo "build output      : $ignored  (gitignored paths; existence depends on having built, so not checkable)"
if [ "$ignored" -gt 0 ]; then
  echo
  printf '%s\n' "${ignored_lines[@]}"
fi
if [ "$historical" -gt 0 ]; then
  echo
  printf '%s\n' "${historical_lines[@]}"
fi
if [ "$broken" -gt 0 ]; then
  echo
  printf '%s\n' "${broken_lines[@]}"
  exit 1
fi
echo
echo "All live path citations resolve, and the archive is internally consistent."
exit 0
