# README demo

An approximately 30-second recording of the real CLI, made with
[VHS](https://github.com/charmbracelet/vhs). The tape shows a Markdown deck,
recall and typed reviews, then the resulting deck progress.

From the repository root:

```sh
brew install vhs
cargo build
PATH="$PWD/target/debug:$PATH" vhs docs/demo/demo.tape
```

VHS needs `ffmpeg`, `ttyd`, and a browser runtime (see its installation
instructions). The recording uses JetBrains Mono Nerd Font and PingFang TC
for Chinese glyphs; install those or change `FontFamily` in the tape to
fonts available on your system, with a Traditional Chinese fallback.

`setup.sh` runs inside the recording shell, puts this checkout's debug
binary on PATH, and creates a fresh temporary deck directory. It removes
that directory when the shell exits. Your own decks and config are never
used. The only staged presentation is the shell prompt and chapter titles;
the cards, review screens, grades, and final status are real app output.

Edit the pacing, theme, or keystrokes in `demo.tape`, then rerun the command.
Keep `demo.gif` checked in so GitHub can display it directly in the README.
Review the whole loop after recording, especially Chinese glyphs, footer
clipping, the transition to typed mode, and the final count of two learned
cards. Session clocks and the next-due date naturally vary between renders.
