# From the repository root: cargo build
# Then: PATH="$PWD/target/debug:$PATH" vhs assets/demo/demo.tape
Output assets/demo/demo.gif

Require bash
Require reword

Set Shell "bash"
Set Width 1200
Set Height 520
Set FontSize 22
Set FontFamily "JetBrainsMono Nerd Font, PingFang TC"
Set LineHeight 1.3
Set Padding 28
Set Margin 24
Set MarginFill "#8fb4e8"
Set BorderRadius 14
Set WindowBar Rings
Set WindowBarSize 32
Set Framerate 15
Set TypingSpeed 45ms
Set Theme {"name":"Reword Light","background":"#faf9f6","foreground":"#253247","black":"#253247","red":"#b64242","green":"#397351","yellow":"#956514","blue":"#3268ae","magenta":"#7a559b","cyan":"#236c88","white":"#e5e8ed","brightBlack":"#647084","brightRed":"#b64242","brightGreen":"#397351","brightYellow":"#956514","brightBlue":"#3268ae","brightMagenta":"#7a559b","brightCyan":"#236c88","brightWhite":"#faf9f6"}

Hide
Type@1ms "source assets/demo/setup.sh"
Enter
Sleep 500ms
Show

# The deck is the file. No imported database or invented UI.
Sleep 1s
Type "cat decks/cantonese.md"
Enter
Sleep 3s

# Recall: the footer teaches space to reveal, then space for Good.
Type "reword review cantonese --recall -n 1"
Sleep 400ms
Enter
Sleep 2s
Space
Sleep 3s
Space
Sleep 1500ms
Type "n"
Sleep 1200ms

# A clean chapter transition; both sessions share the same real history.
Hide
Type@1ms "demo_title 'typed review'"
Enter
Sleep 300ms
Show
Type "reword review cantonese --typed"
Sleep 400ms
Enter
Sleep 2s
Type@140ms "to drink"
Sleep 1s
Enter
Sleep 2s

# The real deck counts show both cards learned, with none left due.
Hide
Type@1ms "demo_title 'progress'"
Enter
Sleep 300ms
Show
Type "reword decks"
Enter
Sleep 4s
