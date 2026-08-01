# pi-orchestra README — the phosphor theme and the monochrome tier.
#
# Drive with tools/record.sh. See hero-nocturne.tape for why paths are relative
# and why `orcd` is shimmed onto PATH.
#
# Phosphor is the 16-colour-safe purist tier: one hue, five luminances, state
# carried by brightness, bold and reverse-video. The second half of this tape
# re-launches under NO_COLOR to capture the Monochrome tier, where there is no
# colour at all and every state has to read from its glyph alone. That tier is
# an acceptance concern rather than a nicety, so it is recorded rather than
# asserted.

# VHS requires an Output, but this tape's deliverables are its stills — an
# animated phosphor GIF is 1.1 MB that the README does not show, and media
# nobody reads is just weight in the clone. So the reel goes to the gitignored
# build tree and only the screenshots are committed.
Output rust/target/vhs/stage-phosphor.gif

Set Shell "bash"
Set FontSize 14
Set Width 1500
Set Height 900
Set Padding 12
Set TypingSpeed 28ms

Hide
Type "export PATH=/tmp/pio-record-shim:$PIO_BIN:$PATH; export ORC_HOME=/tmp/pio-record-home; cd /tmp/pio-record-demo; clear"
Enter
Sleep 2s
Show

Type "pi-orchestra --socket /tmp/pio-record-home/orcd.sock home"
Enter
Sleep 4s

# `--theme` is only a fallback for when the daemon has no configured theme;
# the registry's `app.theme` wins whenever it is set, and it is set to the
# nocturne default the moment the registry exists. So the theme is changed the
# way a user changes it: the leader chord, which works on every screen and asks
# the daemon to persist the choice.
#
# nocturne → ember → phosphor, so twice.
Ctrl+g
Type "t"
Sleep 1200ms
Ctrl+g
Type "t"
Sleep 1500ms
Screenshot rust/target/vhs/home-phosphor.png

# Same launch flow, same codex conductor (see hero-nocturne.tape).
Type "n"
Sleep 1200ms
Type "j"
Sleep 800ms
Enter
Sleep 1200ms
Enter
Sleep 1200ms
Enter

Sleep 10s
Enter
Sleep 7s
Screenshot docs/media/stage-phosphor.png
Sleep 2s

# Detach; the panes stay up in piod, which is what lets the next launch attach
# to the same session rather than building a new one.
Ctrl+g
Type "q"
Sleep 3s

# The monochrome tier. NO_COLOR is honoured by pi-orchestra's own chrome; a CLI
# hosted *inside* a pane still emits its own colours, which is a deliberate
# decision recorded in the notes rather than a gap. What must survive here is
# every pi-orchestra state being readable from glyph, bold and reverse-video
# alone.
Type "clear"
Enter
Sleep 500ms
Type "NO_COLOR=1 pi-orchestra --socket /tmp/pio-record-home/orcd.sock home"
Enter
Sleep 5s
Screenshot docs/media/home-monochrome.png
Sleep 1s
Enter
Sleep 6s
Screenshot docs/media/stage-monochrome.png
Sleep 2s
Ctrl+g
Type "q"
Sleep 2s
