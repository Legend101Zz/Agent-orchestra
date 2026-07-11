Output docs/v4-phase1-narrow-phosphor.gif

Set Shell "zsh"
Set FontSize 15
Set Width 1000
Set Height 700
Set TypingSpeed 5ms

Type "stty cols 72 rows 30; env -u NO_COLOR rust/target/release/pi-orchestra --socket /tmp/orcd-real-spike.sock --theme phosphor"
Enter
Sleep 4s
Ctrl+g
Type "q"
Sleep 300ms
