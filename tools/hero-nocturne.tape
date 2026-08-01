# pi-orchestra README hero — Nocturne (the flagship, and the default).
#
# Drive with tools/record.sh, which builds the scratch ORC_HOME and demo cwd
# this tape expects. Do not run `vhs` on it directly.
#
# VHS 0.11 cannot parse absolute paths in Output/Screenshot, so every path here
# is relative and record.sh runs vhs from the repo root.
#
# Deliberately NOT captured: the ⏻ CONDUCTOR DOWN overlay. It is drawn before
# the pane's cell blit and erased by it in the same frame, so it has never been
# visible (#59). Recording it would mean recording a blank.

Output docs/media/hero-nocturne.gif

Set Shell "bash"
Set FontSize 14
Set Width 1500
Set Height 900
Set Padding 12
Set TypingSpeed 28ms

# The recording environment: a throwaway ORC_HOME, a throwaway demo repo, and
# the freshly built binaries. `orcd` is shimmed onto PATH because the client
# still looks for the pre-#17 daemon name (filed separately).
Hide
Type "export PATH=/tmp/pio-record-shim:$PIO_BIN:$PATH; export ORC_HOME=/tmp/pio-record-home; cd /tmp/pio-record-demo; clear"
Enter
Sleep 2s
Show

Type "pi-orchestra --socket /tmp/pio-record-home/orcd.sock home"
Enter
Sleep 4s
Screenshot docs/media/home-nocturne.png

# Launch flow: brain → worker pool → working directory.
#
# `j` selects codex rather than the default claude. Not an editorial preference:
# Claude Code's start-up panel renders the signed-in account's name, email and
# organisation until the conversation has content, and it cannot be dismissed
# non-interactively (`/clear` reaches its autocomplete, not its submit). Codex
# starts clean, so the hero can be a real frontier conductor without publishing
# somebody's personal details.
Type "n"
Sleep 1500ms
Type "j"
Sleep 800ms
Screenshot docs/media/flow-brain.png
Enter
Sleep 1200ms
Screenshot docs/media/flow-workers.png
Enter
Sleep 1200ms
Screenshot docs/media/flow-cwd.png
Enter

# STAGE: the conductor pane, the bench, and the circuit between them.
Sleep 10s

# Codex asks once whether it trusts a new folder. Answering it is what a real
# first session does.
Enter
Sleep 7s
Screenshot docs/media/stage-nocturne.png
Sleep 2s

# The trigger grammar, typed at the conductor's own prompt: `delegate:` lights
# up per-character and the pane title grows a ◆ DELEGATE badge.
#
# Deliberately NOT submitted. Pressing Enter here would spend real provider
# quota and produce a recording whose content nobody chose. The affordance is
# what this frame is for; what happens after you press Enter is covered by the
# README's worked example, captured from the CLI where the output is
# reproducible.
Type "delegate: add a docstring to app.py and run the tests"
Sleep 2500ms
Screenshot docs/media/stage-trigger.png
Sleep 2s

# Detach — panes keep running in piod.
Ctrl+g
Type "q"
Sleep 2s
