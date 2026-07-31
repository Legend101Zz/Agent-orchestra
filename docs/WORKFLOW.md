# Multi-agent development workflow (V1 program)

This repo is built by a team of humans and agents. This file is the contract
that keeps everyone — including future sessions of each agent — oriented.
**Every agent session starts by reading: `AGENTS.md` → `task_plan.md` →
`progress.md` → open GitHub issues.** Every session ends by updating
`progress.md`.

## Roles

| Actor | Runs | Responsibility |
|---|---|---|
| **Mrigesh** (human) | — | Product owner. Approves scope, tests locally, merges to `main`. |
| **Implementer** | code-puppy · Opus 4.8 long (max/high thinking as needed) | Picks ONE GitHub issue, implements it on a branch, pushes. |
| **Architect/Reviewer** | Claude Code (Fable) | Writes specs and issues, reviews pushed branches, maintains planning docs. Multiple sessions; continuity via this workflow. |
| **Cheap labor** | pi / MiniMax-M3 via `pio` (pre-rename: `orc`) | Bulk reads, summaries, mechanical transforms — delegated by either agent. |

## The loop (one issue at a time)

1. **Pick** — the implementer takes the next unblocked issue from the V1
   epic (respect `Depends on:` lines; never work two issues in one branch).
2. **Branch** — `git checkout -b issue-<N>-<slug>` from fresh `main`.
3. **Implement** — honor the issue's task contract: objective, allowed
   paths, acceptance checks, out-of-scope. If the contract is wrong or
   impossible, STOP and comment on the issue instead of improvising.
4. **Gate** — all of these must pass before pushing (from `rust/`):
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
   cargo build --release --locked
   ```
5. **Push + report** — push the branch, comment on the issue: what changed,
   evidence that each acceptance check passes (paste command output), any
   deviations. Open a PR if convenient; a pushed branch + issue comment is
   the minimum.
6. **Review** — a Claude session reviews the branch against the contract
   (correctness first, then simplification), comments findings on the issue.
7. **Test locally + merge** — Mrigesh runs `./install.sh` from the branch,
   exercises the feature, merges to `main`, closes the issue.

## Context-continuity rules (why LLM code rots and how we prevent it)

Long projects fail when humans and agents lose track of what exists and why.
Hard rules:

- **One issue = one branch = one merge.** No drive-by changes outside the
  contract's allowed paths — with one standing exception: the four process
  files below (`progress.md`, `task_plan.md`, `findings.md`, and `LOG.md`) are
  always in scope, because this document requires updating them. An allowed-path
  list fences code, not the record.
- **`progress.md`** is the running session log: date, actor, issue, what
  was done, what's next. Append, never rewrite history.
- **`task_plan.md`** maps the V1 program to issues and tracks phase status.
- **`findings.md`** records durable discoveries and decisions (gotchas,
  measurements, rejected approaches) — check it before re-deriving anything.
- **Evidence over claims.** A task is done when its acceptance checks are
  demonstrated, not when code compiles. Evidence lives in the issue comment
  and, for larger phases, `docs/notes/YYYY-MM-DD-*.md`.
- **Dead code is debt.** If an approach is abandoned, delete it in the same
  branch; don't leave parallel half-implementations.
- **Secrets never enter the repo.** GitHub tokens come from the `GH_TOKEN`
  env var; provider keys stay in each harness's own config.

## Working setup notes

- Canonical remote: `github.com/Legend101Zz/Agent-orchestra` (`main`).
- **Checkout location (moved 2026-07-27, corrected 2026-07-30):** the working
  checkout is `/Volumes/Mrigesh SSD/pi-orchestra` — the old `~/Agent-orchestra`
  is gone (freed disk space). The external SSD must be mounted to work on the
  repo. Note the **space in the path**: quote it in shell commands
  (`cd "/Volumes/Mrigesh SSD/pi-orchestra"`).
  A second checkout `/Volumes/Mrigesh SSD/Agent-orchestra` also exists. It is
  the *same* repo — same remote — but it is stale: parked on the unmerged
  `issue-12-single-harness-mode` branch at roughly the #12 era. Don't work in
  it. **This note previously said the opposite** (that `pi-orchestra` was not
  this repo); that was true when written and cost a later session real time.
  Confirm with `git log --oneline -1` against `origin/main`, never by path.
- **Worktrees live on the SSD too, in one folder:**
  `/Volumes/Mrigesh SSD/pi-orchestra-worktrees/issue-<N>`. A reviewer or a
  second parallel session takes one from there rather than checking the branch
  out over the main checkout — which is also how you avoid the
  `git diff main` trap below.
  ```bash
  git -C "/Volumes/Mrigesh SSD/pi-orchestra" \
      worktree add "/Volumes/Mrigesh SSD/pi-orchestra-worktrees/issue-<N>" issue-<N>-<slug>
  ```
  Remove it when the issue merges (`git worktree remove <path>`) — each one
  carries its own 2–4 GB `target/`.
- **Nothing for this repo goes on the system disk** — no checkout, no worktree,
  no `target/`. It has ~35 GB free against the SSD's ~645 GB. `~/Agent-orchestra`
  is gone for exactly this reason and must not come back.
- **If the SSD is not mounted, stop and report back.** Do not fall back to
  `$HOME` and do not clone "somewhere convenient": an unmounted
  `/Volumes/Mrigesh SSD` is a plain directory on the system disk, so anything
  written there silently fills the wrong volume under a path that looks right.
  Check `/Volumes/Mrigesh SSD/pi-orchestra/.git` exists before doing anything
  else; if it does not, say so and wait for the human to plug the drive in.
- The remote is still the source of truth (`git fetch` first), but "clone
  somewhere convenient" is **not** licence to use the system disk — convenient
  means another folder on the SSD. Never assume a path —
  `git rev-parse --show-toplevel` if you need one.
- **After moving the checkout, re-run `./install.sh`.** Everything the installer
  links (`~/.claude/skills/*`, the `UserPromptSubmit` hook in
  `~/.claude/pi-orchestra/`, the `~/.zshrc` block) is an absolute symlink into
  the checkout, so a move leaves them all dangling and the trigger grammar
  silently stops firing. `install.sh` is idempotent and replaces dead symlinks.
- code-puppy: model/agent config is user-global (`~/.code_puppy/`); this repo
  provides `AGENTS.md` (root) and `.agents/commands/work-issue.md`. Run it
  with `GH_TOKEN` exported so `gh` works for issue reads/comments.
