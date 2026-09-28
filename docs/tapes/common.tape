Set Shell "bash"
Set FontSize 16
Set Width 1280
Set Padding 18
Set Theme "Builtin Dark"
Set TypingSpeed 25ms
Set Framerate 15
Env COLORTERM "truecolor"

# The recording must use the freshly built command rather than a shell alias or an installed copy.
Hide
Type@0ms 'export PATH="$PWD/build/tapes/cargo/release:$PATH"'
Enter
Type@0ms "unset NO_COLOR"
Enter
Type@0ms "PS1='$ '"
Enter
Type@0ms "clear"
Enter
Sleep 200ms
Show
