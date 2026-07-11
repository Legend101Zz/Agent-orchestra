# Next-session prompt — pi-orchestra v4 "Bench" Phase 3 only

Copy everything below the line into a fresh session started in
`/Users/comreton/Desktop/pi-orchestra`.

---

You are implementing **only Phase 3** of the approved pi-orchestra v4 Bench
design: tasks, worktree isolation, SCORE, and coordination-skill updates.
Phases 0–2 are complete, gated, committed, and pushed. Do not redo them. Do
not start Phase 4 baton/event animation, the full RUNS port/polish, or any
Phase 5 harness-adapter/dogfood work.

## Establish ground truth first

1. Confirm `cwd=/Users/comreton/Desktop/pi-orchestra` and branch
   `v4-bench`. Fetch without changing user files, then prove local HEAD and
   `origin/v4-bench` are equal and both contain the remote-verified Phase 2
   evidence commit `63a9b39841918bc0551edec0e847976ee3b53945`. Prove that
   commit contains Phase 1 commit `91624a0`. Never force-push and never merge
   to `main`.
2. Run `git status --short --branch`. Preserve the pre-existing untracked
   `findings.md`; do not read, add, edit, delete, stage, or commit it.
3. Record checksums before editing and reproduce them after the final gate:

   ```text
   33ecb4a6c902fdacfb6085c7673438d1b95777413dc19a68e6c7bcb6ddaa7ce3  ~/.pi/agent/settings.json
   83709fd8b25ad3f656aa3e1fd0860a26d939239a3ec610e4ed88ec077bdba491  ~/.claude/settings.json
   e5b9f8acd517ac565d8b7a1a87c4b8b446a453f2bd8ec817d3141f6a18eea461  ~/.codex/config.toml
   d230ae0ba42c1bd9283609df839f0b605d7db3cd92fc0e3bb0b18b4c946691d5  ~/.local/bin/orc
   ```

   `~/.local/bin/orc` is a pre-existing symlink into the repository release
   target. Do all development/final builds with an isolated
   `CARGO_TARGET_DIR`; do not overwrite its followed target.
4. Read completely, in this order:
   - `docs/superpowers/specs/2026-07-11-pi-orchestra-v4-bench-design.md`
   - `docs/notes/2026-07-11-v4-phase2-evidence.md`
   - `docs/reviews/2026-07-11-v3-rust-review.md`, especially the five skill
     wording fixes
   - `progress.md`, `task_plan.md`, README, and `docs/guide.html`
   - `rust/crates/orc-core/`, `orc-cli/`, `orc-proto/`, `orc-daemon/`, and
     `orc-app/`, including all existing tests
   - `skills/pi-delegate/SKILL.md`, `skills/orchestrate/SKILL.md`,
     `codex/AGENTS-block.md`, `shell/orchestra.zsh`, `install.sh`, and
     `uninstall.sh`
5. Run the Rust-only baseline with an isolated target: fmt, clippy with
   `-D warnings`, every Rust test, warning-free rustdoc, and locked release
   build. Run the golden compatibility/fake-pi/install tests as part of the
   suite. Confirm no Python runtime/package/test plumbing has returned. If
   the baseline is unexpectedly red, diagnose and record the exact delta
   before editing.

Phase 2 is authoritative: embedded PTYs with `vt100`, on-demand `orcd`, raw
focused-pane bytes, HOME/STAGE, conductor recovery, Rust-only install, and the
compatibility fixtures are shipped. Do not reopen those decisions without a
new failing regression. Preserve the honest Phase 2 evidence caveats:
kitty had an active Phase 2 socket, Ghostty has process-only evidence, and the
7,608-second flood passed duration/resource bounds but its post-run metrics
snapshot was interrupted by the user.

## Non-negotiable rules

- All implementation stays on `v4-bench`. Use TDD for core behavior, small
  conventional commits, push verified slices, and prove the remote final ref.
- Never modify `~/.pi/agent/*`, `~/.claude/settings.json`,
  `~/.codex/config.toml`, or the live `~/.local/bin/orc` target.
- Plain additive JSON/text remains the durable contract. Preserve unknown
  fields. Writes use same-directory temp + flush + `sync_all` + atomic rename.
- **Single writer:** the client never writes task, session, layout, registry,
  worktree, or merge state. Every mutation goes through `orc-core` command
  paths, exposed by `orc`/daemon protocol as appropriate, and records actor
  `brain` or `human`.
- No API-traffic proxying. Focused terminal input remains verbatim. Do not
  regress detach/replay, recovery, quota fail-open, or the compatibility
  oracle.
- Bounded memory/concurrency everywhere; no busy loops. Task watching and
  SCORE refresh must be event-driven or bounded, never an unbounded scan per
  frame.
- Ember and phosphor only. State is written as words. No emojis, raw ad-hoc
  colors, stock-ratatui box grids, web UI, Tauri, or browser surface.
- New/touched public APIs and modules require rustdoc under
  `#![warn(missing_docs)]`. No `unwrap`/`expect` in `orc-core` or `orcd`
  outside tests. Use `thiserror` in libraries, `anyhow` in binaries, and
  `tracing` in daemon paths.
- Worktree and merge operations are safety-sensitive: reject traversal and
  symlink escapes; never delete an unrelated directory/branch; never
  auto-resolve conflicts; never merge implicitly; state dirty-base,
  detached-HEAD, non-git, submodule, and conflict limitations plainly.
- Delegated output is untrusted. If delegation is genuinely useful, use the
  repository's bounded `orc` rules, explicitly attribute the brain, apply a
  kill-after timeout, verify every claim locally, retry at most once, and
  relay any `ORC WARNING:`/`ORC BLOCKED:` line verbatim.

## Phase 3 deliverables — implement all, in this order

### 3A. Durable task model and `orc task`

Add the task domain to `orc-core` and the real Rust CLI. Use file-per-task,
plain additive JSON with stable T-prefixed IDs and these statuses:
`backlog`, `assigned`, `running`, `review`, `done`, `dropped`.

Each task must support at least title/description, session, status,
`depends_on`, assignee/run linkage, optional worktree metadata, created and
updated timestamps, and append-only actor-attributed history. Define and
document the valid transition/assignment rules. Unknown future fields must
survive mutations. Concurrent ID allocation/mutations must not collide, lose
history, or produce partial JSON.

Implement and document:

```text
orc task add
orc task list
orc task show
orc task assign
orc task start
orc task review
orc task done
orc task drop
orc task move
orc task diff
orc task merge
```

Include `--session`, `--actor brain|human`, machine-readable `--json` where it
is meaningful, dependency and assignee inputs, and `--isolate` on add. Use
`ORC_SESSION` as the default session when safe, but require a clear session
instead of silently writing to an unrelated board. A pane-launched brain may
default mutations to actor `brain`; interactive client actions use `human`.
Invalid IDs, actors, transitions, dependencies, cycles, missing sessions,
corrupt siblings, and conflicting concurrent mutations must return explicit
errors and non-zero exits. Read-only list/show must tolerate legacy/additive
and corrupt sibling files without hiding valid tasks.

Gate and commit the task model/CLI before adding worktree side effects.

### 3B. Worktree-per-task isolation and explicit merge lifecycle

Implement worktree ownership in `orc-core` and invoke it only through the
task command path:

- `orc task add --isolate` or session default `"isolation":"worktree"`
  creates `~/.orchestra/worktrees/<session>/<task-id>` on branch
  `orc/<session-slug>/<task-id>` from the session's recorded base repository,
  branch, and commit.
- Assignment/start launches or directs the worker from that worktree cwd so
  isolated workers cannot trample one another.
- `orc task diff <id>` reports the real worktree diff and review stats
  (insertions, deletions, files) without mutating it.
- Moving to REVIEW preserves the worktree and exposes those stats.
- `orc task merge <id>` is always explicit. Require the correct clean base,
  prove branch/worktree ownership, squash-merge onto the recorded session base
  branch, record actor/history/result commit, then prune only the owned
  worktree/branch after success.
- `orc task drop <id>` records the dropped state and prunes only the owned
  worktree/branch without merging. Refuse destructive cleanup when ownership
  cannot be proven.
- Non-git cwd degrades gracefully: normal tasks still work, isolation/diff/
  merge state `ISOLATION UNAVAILABLE` with a useful reason.

Never mutate the user's source checkout just to make a test pass. Integration
tests must create temporary Git repositories and cover creation/branch/cwd,
parallel isolated edits, diff stats, explicit squash merge, history actor,
safe prune, drop-without-merge, dirty base, detached HEAD, non-git cwd,
conflicts, missing/reused paths, and attempts to operate on unrelated
worktrees or branches. A merge conflict must leave recoverable state and must
not be auto-resolved or falsely mark the task done.

Gate and commit the worktree lifecycle before SCORE.

### 3C. Production SCORE view and board↔stage navigation

Add SCORE to `pi-orchestra` using the existing client/theme/compositor system:

- Columns: Backlog / Assigned / Running / Review / Done. Dropped tasks remain
  durable and inspectable but do not masquerade as active work.
- Cards show T-id, title, assignee/worker, written status, exact tokens or
  `~` estimate, worktree isolation mark, and dependency/attention state.
- REVIEW cards show real worktree diff stats such as
  `+412 -88 · 9 files`.
- History detail/popover shows actor-attributed moves.
- Keyboard moves and mouse drag both invoke the same core/CLI/daemon mutation
  path as actor `human`; the client never edits task JSON.
- `g` from a task navigates to/focuses its assignee's STAGE card. From STAGE,
  provide a clear route back to the linked task. `V`, `?`, and `,` retain the
  established view/help/settings language.
- Empty, non-git, blocked dependency, unassigned, missing-run, merge-conflict,
  narrow, and corrupt-sibling states must explain the next valid action in
  plain words.

This phase implements navigation/linkage, not Phase 4 baton animation or view
transition polish. Do not expand the RUNS placeholder into the Phase 4 port.

Add TestBackend snapshots/assertions for SCORE in ember and phosphor, wide and
exactly 72×30. Test keyboard and mouse moves, actor attribution, drag target
calculation, event-driven refresh, review stats, history, `g` navigation,
detach/reattach durability, and no pane-input regression. Capture and inspect
reproducible SCORE evidence in both themes/sizes, but do not claim real-app
image evidence where macOS only provides process evidence.

Gate and commit SCORE before editing coordination instructions.

### 3D. Skills, AGENTS, shell/install propagation, and re-orientation

Update the repository-owned skill/AGENTS sources and ensure install/uninstall
still propagates/removes only marked or symlinked content safely:

- Treat `pi-orchestra` as an alias trigger for the product workflow.
- Teach `orc task` board maintenance and actor/session usage.
- Offer the configured `default_workers` (Hermes + pi/MiniMax-M3 today) but
  never silently assume the user's worker pool.
- Teach `ORC_SESSION`/`ORC_PANE_ID` awareness and resumed-brain
  re-orientation: read `orc task list --session ...` plus `orc list` before
  acting, preserve durable inbox/task context, and do not recreate completed
  work.
- Apply all five concrete wording fixes from
  `docs/reviews/2026-07-11-v3-rust-review.md`: brain-correct attribution plus
  explicit `--session`, exact-token/cost reporting with estimated fallback and
  `orc stats` receipt, no invented pi/`orc` flags such as `--thinking`, advertise
  `orc send/retry/handoff`, and remove the obsolete Python-runner preference in
  favor of the sole Rust runner while retaining the bounded-log warning.
- Inspect local Hermes documentation/help first. Add a Hermes instruction
  block only if a real AGENTS-equivalent is supported; otherwise document that
  no block was installed. Do **not** build the Phase 5 Hermes adapter here.

Add tests or deterministic install fixtures proving marked blocks are
idempotent, no duplicate instructions appear, existing user content survives,
and uninstall removes only pi-orchestra-owned content. Run an actual isolated
HOME install/uninstall, never the live HOME.

## Phase 3 gate and handoff

Before claiming Phase 3 complete:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`, including task model/CLI, concurrent mutation, temporary-Git
  worktree/diff/merge/drop safety, SCORE interactions/snapshots, daemon refresh,
  compatibility oracle, fake-pi, and install tests
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`
- locked release build, all with isolated `CARGO_TARGET_DIR`
- no `unwrap`/`expect` in `orc-core`/`orcd` outside tests
- no Python runtime/package/test plumbing
- protected checksums exact and `findings.md` untouched
- actual isolated-HOME install/uninstall pass
- task JSON unknown-field/atomic-write/actor-history proof
- explicit worktree ownership, conflict, non-git, dirty-base, squash-merge,
  and safe-prune proof against temporary repositories
- SCORE in ember + phosphor, wide + exactly 72×30, keyboard + mouse, inspected
  visual evidence, and board↔stage navigation
- existing Phase 2 compatibility, raw-input, recovery, detach/replay, latency,
  and idle tests remain green; remeasure only if a hot path changed

Write `docs/notes/2026-07-11-v4-phase3-evidence.md` with exact commands and
results. Update README/guide, `progress.md`, and `task_plan.md`. Use small
verified commits, push `v4-bench`, and prove local HEAD,
`origin/v4-bench`, and `git ls-remote origin refs/heads/v4-bench` all equal
the final Phase 3 commit. End with shipped/cut/risks and the next blocker.

**Stop there. Do not implement or plan Phase 4 in code, do not begin Phase 5,
and do not merge `v4-bench` to `main`.**
