# 🛡️ Anti-slop checklist — the human's job

*Agents write the code. You are the only thing standing between this repo and
a pile of plausible-looking garbage. Your leverage is not writing code — it's
refusing to accept bad work. This file is what "doing your job" means.*

## Your loop (one issue at a time)

```
pick issue → puppy builds → Claude reviews → puppy fixes → Claude re-reviews
→ YOU test locally → merge → update LOG.md → next issue
```

Prompts for every arrow are in `LOG.md`. Run **one** puppy on **one** issue
at a time (parallel only for the marked no-conflict set, and only when
you're comfortable).

## Rules that prevent slop

**1. Never merge what you haven't run.**
`./install.sh` from the branch, then actually use the feature for 2 minutes.
Compiling is not working. If you can't tell whether it works, that's a
reviewer's fault — send it back with that exact complaint.

**2. The contract is the whole truth.**
If the branch does things the issue didn't ask for — extra features, drive-by
refactors, new dependencies — reject it, even if the extras look nice.
Unrequested code is where bugs hide, because nobody reviews what nobody asked
for.

**3. Believe evidence, not adjectives.**
"Implemented and tested ✅" means nothing. The issue comment must show each
acceptance check's command + output. Missing evidence = not done. You never
need to read Rust to enforce this.

**4. Two fix rounds, then stop.**
If an issue isn't accepted after build → fix → fix, do NOT run a third round
on top. The contract is probably wrong or too big. Close the branch, split or
rewrite the issue (ask a Claude session to re-scope it), start clean. Sunk
tokens are gone either way; iterating on confusion buys more confusion.

**5. One session, one job, fresh context.**
New puppy issue = fresh clone + fresh session (`/work-issue N` and nothing
else). Review = its own Claude session per issue. Never say "while you're at
it, also…" mid-issue — that's how scope leaks past every gate. Want more
work? Make it an issue.

**6. Read the ship log entry before anything else.**
If code-puppy's plain-English entry in `LOG.md` doesn't make sense to YOU, the
work doesn't merge. You are the readability test. "I don't understand what
this shipped" is a full-strength rejection reason.

**7. Watch the diff size.**
`git diff main --stat` on the branch. An issue scoped to "registry + CLI"
that touches 40 files across every crate is a red flag, whatever the tests
say. Big diff on a small contract = send back.

**8. Keep `main` sacred and the board honest.**
Nothing lands on `main` except your merges. After every merge: tick epic #15,
set ✅ in `LOG.md`, close the issue. A stale board makes every future session
(human and agent) dumber.

**9. Token hygiene.**
Fresh `GH_TOKEN` per working day, pasted into the terminal env only — never
into a chat with any agent, never into a file. Revoke when done. (Both tokens
from 2026-07-22 chats must stay revoked.)

**10. When lost, re-anchor — don't improvise.**
Confused about state? Read `LOG.md` ship log top-down, then `git log --oneline
-10`. Never ask an agent "what's the status?" and trust it from memory — point
it at these files.

## Red flags — reject on sight

- "All checks pass" with no command output pasted.
- New dependencies not mentioned in the issue.
- Changes outside the issue's allowed paths ("I had to touch X to make it work" — then the contract was wrong; stop and re-scope).
- A reviewer verdict that reads like a compliment ("solid implementation!") instead of an attack report — re-run the review with the adversarial prompt.
- Ship-log entry full of jargon or vague ("improved robustness") — demand a rewrite; vagueness usually hides "I'm not sure what I did".
- Any agent telling you to skip the gates "just this once".
