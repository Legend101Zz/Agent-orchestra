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
  contract's allowed paths.
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
- Mrigesh's machines may have two checkouts (external SSD + `~/Agent-orchestra`);
  the remote is the source of truth — always `git fetch` first.
- code-puppy: model/agent config is user-global (`~/.code_puppy/`); this repo
  provides `AGENTS.md` (root) and `.agents/commands/work-issue.md`. Run it
  with `GH_TOKEN` exported so `gh` works for issue reads/comments.
