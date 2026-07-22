# /work-issue — implement one GitHub issue end-to-end

Usage: `/work-issue <issue-number>`

You are the **implementer** in the pi-orchestra multi-agent workflow
(`docs/WORKFLOW.md`). Execute exactly one issue:

1. `git fetch origin && git checkout main && git pull` — start fresh.
2. `gh issue view <N> --repo Legend101Zz/Agent-orchestra` — read the full
   task contract: objective, allowed paths, acceptance checks, out-of-scope,
   dependencies. If a dependency issue is still open, STOP and report.
3. Read `AGENTS.md`, then every file the issue lists under Context.
4. `git checkout -b issue-<N>-<short-slug>`.
5. Implement the objective. Stay strictly inside allowed paths. Write tests
   for every acceptance check that can be automated.
6. Run all gates from `rust/` (see `AGENTS.md`). Fix until green.
7. Commit (conventional prefix, why in the body) and
   `git push -u origin issue-<N>-<short-slug>`.
8. Comment on the issue: branch name, summary of changes, and per
   acceptance check the exact command + output proving it passes. Note any
   deviation from the contract explicitly.
9. Append a dated entry to `progress.md` (actor: code-puppy) in the same
   branch before the final push.

Never: work multiple issues in one branch, push to `main`, touch files
outside allowed paths, or mark a check passed without running it.
