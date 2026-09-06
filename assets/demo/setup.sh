# Sourced by demo.tape from the repository root in a disposable bash shell.
# Use the binary built from this checkout and never touch the user's decks.
export PATH="$PWD/target/debug:$PATH"
export REWORD_DIR
REWORD_DIR=$(mktemp -d "${TMPDIR:-/tmp}/reword-demo.XXXXXX")
trap 'rm -rf -- "$REWORD_DIR"' EXIT
unset NO_COLOR
export PS1='\[\e[38;2;50;104;174m\]❯\[\e[0m\] '
export PROMPT_COMMAND=''
mkdir -p "$REWORD_DIR/decks"
printf '# Cantonese\n食::to eat\n飲::to drink\n' > "$REWORD_DIR/decks/cantonese.md"
cd "$REWORD_DIR"

demo_title() {
    clear
    printf '\e[1;38;2;37;50;71mreword\e[0m  \e[38;2;50;104;174m%s\e[0m\n\n' "$1"
}
demo_title 'flashcards in your CLI'
