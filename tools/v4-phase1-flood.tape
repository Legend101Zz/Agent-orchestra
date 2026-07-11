Output docs/v4-phase1-flood.gif

Set Shell "zsh"
Set FontSize 15
Set Width 1600
Set Height 900

Type "env -u NO_COLOR rust/target/release/pi-orchestra --socket /tmp/orcd-flood-spike.sock --theme ember"
Enter
Sleep 8s
Ctrl+g
Type "q"
Sleep 300ms
