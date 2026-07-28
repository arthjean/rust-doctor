Output artifacts/rust-doctor-terminal.gif
Output artifacts/rust-doctor-terminal.mp4

Set Shell "bash"
Set Width 1200
Set Height 560
Set FontSize 18
Set TypingSpeed 35ms
Set Framerate 30
Set CursorBlink false
Set Theme "Catppuccin Mocha"

Hide
Type "source scripts/setup-terminal-recording.sh"
Enter
Wait+Screen /terminal-recording-ready/
Type "clear"
Enter
Sleep 500ms
Show

Type "rust-doctor --scope full --blocking none --disable-adapter compiler-lint,supply-chain,quality,network-dependent"
Enter
Wait+Screen /Select projects/
Sleep 1s
Space
Enter
Wait+Screen /Panic in library/
Sleep 5s
Escape
Sleep 1s
