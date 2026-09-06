# Developing reword

For contributors and AI agents. User-facing behavior is documented in
[README.md](README.md); this file covers what is not obvious from the code.

## Build and test

```bash
cargo test
cargo run -- --dir /tmp/reword-play init
cargo run -- --dir /tmp/reword-play review
```

Rust 2024 edition. The `fsrs` crate pulls in `burn`, so a clean build takes
a few minutes; incremental builds are fast. `tests/cli.rs` covers
non-interactive behavior end to end. The interactive loop (`review`) has no
automated test; check it in a real terminal after touching `session.rs` or
`term.rs`.

## Principles

These shaped every decision. Changes that break one need a good reason.

1. **Text is the database.** Decks are text files edited in any editor. The
   tool never writes to a deck file during review.
2. **History is the source of truth; memory state is derived.** FSRS state
   is recomputed by replaying the log every run. There is no state file, no
   cache, nothing to corrupt.
3. **Time-bounded sessions.** You commit minutes; the planner picks what
   fits. Overdue cards are prioritized, not counted as debt.
4. **One mode per session.** Recall or typed, never mixed. Both feed the
   same per-card history.
5. **General purpose.** No language-specific rules, no per-deck defaults,
   no assumptions about what a card contains. Numeric defaults (session
   minutes, retention) are fine; content defaults are not.
6. **Honest grading.** Again and Good are the default buttons. Skipping is
   not failing. Assisted answers are not recall.
7. **Good CLI citizen.** Follows [clig.dev](https://clig.dev): stdout for
   data, stderr for messages, `--json` for machines, `NO_COLOR`, exit codes
   0/1/2, no prompts when stdin is not a TTY.
8. **Crash-only.** Every grade is appended to the log the moment it is
   given. Ctrl-C loses nothing.

## Stack

| Choice | Why |
| --- | --- |
| `clap` 4 (derive) | Help text, typo suggestions, subcommands, completions. |
| `fsrs` 6.6.2 | The FSRS-6 code Anki ships, including the optimizer behind `reword optimize`. Scheduler-only `rs-fsrs` is the fallback if build time ever hurts. |
| `crossterm` | Raw single-key input, colors, and the alternate screen used during `review`. Every other command is line-oriented. |
| `ctrlc` | Restores the terminal if Ctrl-C lands during cooked-mode typed input, when raw mode is off and crossterm cannot see it. |
| `jiff` | Local-day arithmetic with a 4 am rollover. |
| `toml`, `serde` | `config.toml` and `params.toml`. |

The crates.io name `reword` is taken, so the package is `reword-cli` and the
binary is `reword`.

## Module map

```
src/
  main.rs      entry: parse args, dispatch, map errors to exit codes
  cli.rs       clap definitions; `reword` alone prints status
  commands/    one file per subcommand
  store.rs     the data directory: finding it, listing decks and logs
  deck.rs      deck parser (`::` / `:::`, headings and fences ignored)
  history.rs   append-only logs and replay into per-card histories
  memory.rs    FSRS-6 state derived from replayed history
  planner.rs   what is due, what is new, how much fits in the time budget
  session.rs   the review loop: reveal, grade, undo, requeue, time budget
  term.rs      TTY detection, colors, single-key reads, line prompts, the
               review Screen (alternate screen, fixed rows, footer keymap)
  text.rs      Unicode-aware keys, typed-answer matching, log escaping
  clock.rs     UTC timestamps, local study days, 4 am rollover
  config.rs    config.toml and params.toml, everything optional
  types.rs     Goal, Mode, Grade
  error.rs     one error type: message plus optional hint
  out.rs       stdout vs stderr; a closed pipe is not an error
```

## Invariants worth knowing

- **Card identity** is the deck plus the NFC-normalized, trimmed front.
  Changing the front orphans history; `rename` rows carry it over. A
  `:::` line is two cards (`forward`, `reverse`) with separate state.
- **Log rows are never edited or deleted.** Undo appends an `undo` row that
  cancels the previous non-undo row for that card. Replay drops both.
  `skip` rows are postponements and never reach FSRS.
- **A typed-mode typo override** is a single `good` row carrying the typed
  answer and an `override` marker, not two rows.
- **Rename detection** at session start fires only when exactly one history
  is orphaned and exactly one never-reviewed card sits before the last
  reviewed line. New cards enter in line order, so an unseen card among seen
  ones is the anomaly. Anything less clear-cut points at `reword check`.
- **Same-day repeats** are passed to FSRS with zero elapsed days. A card
  graded Again comes back after at least five other cards.
- **Siblings.** `reverse` becomes eligible only after `forward` has one
  successful review, and the two are never in the same session.
- **New intake** is one per five reviews, capped by `new_per_day` across
  all decks, and pauses while the overdue backlog exceeds two sessions'
  worth. An explicit `--new N` bypasses the throttle.
- **Typed answers** are read as a cooked line so IME composition works. Raw
  mode is only for single-key prompts.
- **The review screen** redraws a whole frame per state on the alternate
  screen: header (deck · mode, progress), rule, ticker (previous card's
  result), body from row 4 (front in bold, answer in cyan), footer keymap on
  the last row. Nothing is appended, so the scrollback only gets the
  summary printed after the screen is left. Colors carry roles only:
  green good, red again, yellow skip, cyan answer, dim everything
  secondary. Layout must still work with `NO_COLOR`.
- **Deck parse problems** are warnings in `review` (the line is skipped)
  and hard errors in `add`, so a typo never locks anyone out of reviewing.
- **Precedence:** flags > environment (`REWORD_DIR`, `NO_COLOR`) > config.

## Release

Releases are built by [dist](https://github.com/axodotdev/cargo-dist) from
`dist-workspace.toml` via `.github/workflows/release.yml`. Pushing a tag
like `v0.2.0` builds macOS and Linux binaries, creates the GitHub release,
and updates the formula in `joshuamotoaki/homebrew-tap`. Bump `version` in
`Cargo.toml` first.

## Deliberately not in scope

- Any GUI, web UI, or full-screen TUI.
- Audio, images, cloze deletions, HTML in cards.
- Streaks, badges, notifications, telemetry, multiple users, servers.
- Automatic Hard/Easy from response time.
- Any default that assumes what a deck is about. Language-specific
  normalizers, if ever added, are opt-in per deck.

Possible later work, only if earned by real use: multi-line cards, a
cost-aware planner, FSRS-7, per-deck parameter groups, a `sync` wrapper
around git.
