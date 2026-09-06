# Reword: plan

A no-frills, general-purpose flashcard CLI. Runs locally, stores everything as
plain text, schedules with FSRS. Sync is your problem (git works).

This plan turns two inputs into concrete decisions: the spaced-repetition
report (`spaced_repetition_language_vocabulary_report.md`) and the Command
Line Interface Guidelines (clig.dev). Where the report says "test before
claiming", this plan picks the simplest defensible rule and marks it tunable.

## 1. Principles

1. **Text is the database.** Decks are text files you edit in any editor.
   Review history is an append-only log. There is no binary state file.
2. **History is the source of truth; memory state is derived.** FSRS state is
   recomputed by replaying the log. Nothing to corrupt, nothing to merge by
   hand, and a scheduler change never destroys data. (Report, section 11.)
3. **Time-bounded sessions, not due-count guilt.** You commit minutes; the app
   picks what fits. Overdue cards are candidates to prioritize, not debt.
   (Report, section 4.)
4. **One mode per session.** A session is either *typed* or *recall*. Modes
   never interleave inside a session; switching modes breaks flow. Both modes
   update the same per-card history. (Report, section 6.)
5. **General purpose.** No language-specific rules, no per-deck defaults, no
   assumptions about what a card contains. A card is a front and a back.
6. **Honest grading.** Two buttons by default: Again and Good. Skipping is
   not failing. Assisted answers are not recall. (Report, sections 4, 7, 8.)
7. **Be a good CLI citizen.** Flags over args, `--help` everywhere, stdout for
   output and stderr for messages, `--json` for machines, `NO_COLOR`, exit
   codes, no prompts when stdin is not a TTY. (clig.dev.)
8. **Crash-only.** Every review is appended to the log the moment it is
   graded. Ctrl-C at any point loses nothing.

## 2. Stack

| Choice | Decision | Why |
| --- | --- | --- |
| Language | Rust (edition 2024) | Single static binary, fast startup, clig.dev's distribution advice. |
| Arg parsing | `clap` 4 (derive) | Help text, typo suggestions, subcommands. |
| Scheduler | `fsrs` crate 6.6.2 (Open Spaced Repetition's `fsrs-rs`, BSD-3) | The FSRS-6 code Anki ships. Includes the **optimizer**, so `reword optimize` can fit parameters to your own history. Cost: it pulls in `burn`, so clean builds take a few minutes. The scheduler-only `rs-fsrs` crate is the fallback if that ever hurts. |
| Terminal | `crossterm` for raw single-key input and colors | Line-oriented output, not full-screen. |
| Time | `jiff` | Local-day arithmetic for elapsed days. |
| Config | `toml` | Small, human-editable. |

Crate name on crates.io: `reword` is taken by an unrelated library, so the
package is `reword-cli` with the binary named `reword`. No conflict with any
Homebrew formula or common command.

## 3. Data layout

One directory holds everything. Sync that directory however you like.

```
~/reword/                     (override: --dir, $REWORD_DIR)
  config.toml                 settings; synced with the rest
  params.toml                 FSRS parameters written by `reword optimize` (optional)
  decks/
    cantonese.md              the cards
    cantonese.log             that deck's append-only review log
    capitals.md
    capitals.log
  .gitattributes              "*.log merge=union" so two devices never conflict
```

Deck files use `.md` so they render and edit nicely anywhere; the parser only
cares about card lines. Each deck's history sits beside it with the same stem,
so `mv`, `cp`, and `rm` on the pair keep a deck self-contained without the tool
knowing. A `.log` with no matching `.md` is reported by `reword decks` as an
orphan rather than ignored.

### Deck file format

The Obsidian spaced-repetition convention. One card per line.

```
# Cantonese, food
食::to eat
飲:::to drink
唔該::excuse me / thank you (for a service)
```

- `front::back` makes one card, front → back (goal `forward`).
- `front:::back` makes two cards from one line: front → back (`forward`) and
  back → front (`reverse`). Each goal has its own FSRS state.
- ` / ` in the answer side separates alternatives for typed-mode checking.
  In recall mode it is just text.
- Anything else, including `#` headings, prose, and blank lines, is ignored.
  A deck file can be an ordinary Markdown note with cards in it.
- Whitespace around `::` is ignored; `食 :: to eat` and `食::to eat` are the
  same card. Fronts are compared after trimming and Unicode NFC normalization
  so an editor that re-normalizes text does not orphan history.
- A line splits on the first `:::` if present, otherwise the first `::`, so a
  front like `Vec::new()` still works with a `:::` card. Documented, not
  guessed.
- Multi-line cards (`?` and `??` on their own line, as in Obsidian) are a
  later addition, not v1.

Rules:

- The card key within a deck is the front. Editing the back keeps history.
  Editing the front makes a new card; use `reword rename` to carry history
  along. Moving a card between decks is not supported in v1; its history stays
  in the old deck's log.
- A duplicate front within one deck is an error, reported with the line
  number. One deck per language variety: 食 in `cantonese.md` and 食 in
  `mandarin.md` are different cards with different histories.
- Lines are the unit of git diff, so `add`, `rename`, and manual edits all
  produce readable, mergeable changes.

### Editing deck files directly

This is the primary way cards get written, so every edit has a defined,
boring outcome. The tool never writes to a deck file during review, and
history is keyed on the normalized front, so an edit only changes which cards
exist.

| Edit | Effect |
| --- | --- |
| Add a line | New card, shown as new next session. |
| Change the back | Nothing scheduled changes; the new text shows next review. |
| Delete a line | Card leaves the queue. Its log rows stay. Re-adding the same front resumes its history rather than starting over. |
| Change the front | Old history is orphaned; the new front is a fresh card. `reword rename` repairs it, before or after the edit. |
| `::` → `:::` | Reverse card appears as new; forward history untouched. |
| `:::` → `::` | Reverse card disappears; its rows are orphaned. |
| Reorder, add headings or prose, change spacing | No effect. |
| Duplicate a front | Warning with both line numbers at review; first wins. Hard error only from `add`. |
| Rename or delete the deck file | Move or delete the `.md` and `.log` together. A stray `.log` is reported, not ignored. |
| Edit during a session | Safe. The session read the deck at start and only appends to the log. Changes apply next session. |
| Edit a log by hand | Malformed lines are warned with line numbers and skipped. Never fatal. |

Rename detection: at session start, if exactly one history is orphaned and
exactly one never-reviewed card appeared since the last session, ask
`Did you rename 食 to 食物? [y/N]` on a TTY. Suggest, never guess silently.
Anything less clear-cut gets a one-line hint pointing at `reword check`.

### History logs

One per deck, `decks/<name>.log`. Tab-separated, one event per line, appended
only. Columns:

```
ts                    front  goal     mode    grade  elapsed_ms  answer
2026-09-06T02:11:09Z  食     forward  recall  good   4120
2026-09-06T02:11:31Z  唔該   forward  typed   again  9800        excuse
2026-09-06T02:12:02Z  唔該   forward  typed   good   3000        excuse me   override
```

A multi-deck session appends each review to the log of the deck the card came
from. `stats` and `optimize` read every log. Renaming a deck is renaming the
`.md` and `.log` pair; deleting a deck deletes its history with it.

- `goal`: `forward` or `reverse`.
- `mode`: `recall` (self-graded after reveal) or `typed`.
- `grade`: `again`, `hard`, `good`, `easy`, `skip` (a postponed card, not a
  memory observation; never fed to FSRS), or `undo` (cancels the most recent
  non-undo row for the same card; replay drops both).
- `answer`: the typed response, kept verbatim so grading disputes can be
  replayed. A trailing `override` marks a typo override.
- `rename` rows record front changes so replay can follow a card.

Timestamps are UTC. "Days elapsed" for FSRS is computed on local calendar
days with a 4 am rollover, so a midnight session is one day, not two.

Replay cost: one line per review, single user. Tens of thousands of lines
replay in milliseconds, so there is no cache file. A session only replays the
logs of the decks it touches. If it ever matters, a
gitignored cache keyed on the log's length is a later optimization.

### Config

Everything here is optional. With no config file, the only behaviors that
need a value ask for it on a TTY and fail with a hint under `--no-input`.

```toml
desired_retention = 0.90   # FSRS target; the report's recommended start
session_minutes   = 10
new_per_day       = 10
mode              = "recall"   # set this to stop `review` asking each time
```

Precedence: flags > environment (`REWORD_DIR`, `NO_COLOR`) > config file.

## 4. Scheduling

**Memory model.** FSRS-6 with default parameters until `reword optimize` has
enough history. State per (card, goal): stability, difficulty, last review.
Retrievability is computed from real elapsed time at session start, so a
week away is reflected honestly and never counted as a failure.

**Grades → FSRS.** `again` = 1, `hard` = 2, `good` = 3, `easy` = 4. The UI
offers Again and Good; Hard and Easy exist behind extra keys. Typed mode maps
checked-correct to Good and checked-wrong to Again, with no second rating.

**Modes share state.** A `forward` card reviewed in a typed session and later
in a recall session is one history and one FSRS state, with the mode recorded
on each event. The report's launch recommendation, section 6.

**Same-day relearning.** A card graded Again is requeued in the current
session after at least five other cards. Same-day repeats are passed to FSRS
with zero elapsed days, which FSRS-6 handles with its short-term stability
rule. Extra reviews on a long study day are real events, logged and applied
the same way.

**Siblings.** For a `:::` line, `reverse` becomes eligible as a new card only
after `forward` has one successful review, and the two are never shown in the
same session. Seeing one side's answer teaches the other.

**Session planner (v1, deliberately simple).** Inputs: minutes, decks, mode.

1. Relearning cards from earlier in this session, once their gap has passed.
2. Due cards, lowest retrievability first, until the time budget is spent.
3. New cards, one per five reviews, up to `new_per_day`. New intake drops to
   zero automatically while the overdue backlog exceeds two sessions' worth
   of reviews (recovery throttle).

The report's cost-aware priority score is the v2 planner. The v1 rule fits in
one sentence, and the log captures everything needed to evaluate a
replacement.

**Session end.** The session stops at the time budget or when the queue is
empty, prints a summary, and, if cards remain, one line offering to continue.
Continuing is a choice, not a nag.

## 5. Commands

Git-style subcommands. `reword` alone prints status plus concise help.

```
reword                      status: due now, new available, last session, next step
reword init [--dir PATH]    create ~/reword with an example deck
reword review [DECK...]     start a session
    --typed | --recall      session mode; asked for on a TTY if unset in config
    -m, --minutes N         time budget
    --new N | --no-new      new-card intake for this session
reword add [DECK] [FRONT] [BACK] [--reverse]
                            append a card line; prompts for missing pieces on a
                            TTY, fails with a hint under --no-input
reword edit [DECK]          open the deck file in $EDITOR

`add` and `edit` use the only deck when exactly one exists and ask otherwise.
reword decks                list decks with card counts and due counts
reword rename DECK OLD NEW  change a front and carry its history along; works
                            whether or not the deck file already has NEW
reword check                validate decks and logs: duplicates, orphaned
                            histories, malformed lines, stray logs
reword completions SHELL    shell completions for commands and deck names
reword stats [--json]       retention, reviews per day, 14-day forecast
reword optimize             fit FSRS parameters to your history; reports log
                            loss before and after; writes params.toml
reword help [COMMAND]
```

Global flags: `--dir`, `--json` where output is data, `-q/--quiet`,
`--no-color`, `--no-input`, `-h/--help`, `--version`.

### The review loop

Recall mode:

```
$ reword review cantonese -m 10 --recall
cantonese · 10 min · 23 due, 4 new · recall

  食
                                     [space] reveal  [s] skip  [q] quit
  to eat
                                     [space] good  [a] again  [h] hard  [e] easy
✓ 3d 12h · 14/28 · 6 min left        [←] undo
```

Typed mode:

```
$ reword review cantonese -m 10 --typed
cantonese · 10 min · 23 due, 4 new · typed

  唔該
  > excuse
✗ excuse me                          [enter] next  [o] that was a typo
```

- Enter is a synonym for space everywhere.
- When a session spans several decks, each card carries a dim deck tag, so 食
  in `cantonese` and 食 in `mandarin` are never confused.
- The post-grade line doubles as progress: new interval, cards done, minutes
  left. No progress bar.
- The mode prompt defaults to whatever you chose last session, so Enter
  repeats it. It still asks.
- Nothing due is a real screen: `Nothing due in cantonese. 12 cards come due
  tomorrow. Use --new 5 to learn ahead.`
- Typed answers are read as a cooked line, not in raw key mode, so IME
  composition for Chinese, Japanese, and Korean works. Raw mode is used only
  for single-key prompts.
- Skip logs a `skip` row and moves on. It does not touch FSRS state.
- `←` or `u` undoes the last grade or skip, repeatably, back through the
  session. The previous card returns in its revealed (or checked) state with
  the grading keys, and any retry it had queued is withdrawn. Undo appends an
  `undo` row rather than deleting anything. It is scoped to the current
  session; after quitting, a wrong grade is corrected by the next review.
- Typed mode checks the answer and shows the expected text on mismatch, with
  `[o]` to override a typo. Original answer and override are both logged.
- `q` exits immediately with a summary. Nothing is lost.
- `review` refuses to run when stdin is not a TTY and says so.
- The end-of-session summary: cards reviewed, accuracy, minutes, and what
  remains, followed by the single most useful next command.

### Typed-answer checking

Deterministic and language-agnostic, per the report's warning against
edit-distance judges:

- Normalize Unicode to NFC, trim, collapse whitespace, ignore case.
- The answer matches if it equals the answer side or any ` / ` alternative.
- Anything else is a mismatch, with the override key as the escape hatch.

Language-specific normalizers (tone marks, romanization variants) are not in
v1. If they come later they are opt-in per deck, never on by default.

## 6. Output and errors

- Human-readable by default, color only when stdout is a TTY and `NO_COLOR`
  is unset. `--json` on `status`, `decks`, and `stats`.
- Messages go to stderr. Data goes to stdout.
- Exit codes: 0 success, 1 user error (bad deck line, unknown deck), 2 usage.
- Errors say what to do: `decks/cantonese.md:14: duplicate front "食" (first
  seen on line 3). Merge the backs or disambiguate the front.`
- Deck parse problems are warnings during `review` (the bad line is skipped)
  and hard errors during `add`, so a typo never locks you out of reviewing.

## 7. Build sequence

Each stage ends with something usable.

| Stage | Deliverable | Notes |
| --- | --- | --- |
| 1 | `init`, `add`, `decks`, `edit`, `check`, deck parser with tests, dir discovery | No scheduling yet. Confirms the file format feels right in an editor. |
| 2 | `review --recall`, per-deck history logs, FSRS replay, `status` | The core loop. Default FSRS parameters. |
| 3 | `review --typed` with normalization and override | Answer logging. |
| 4 | `stats`, forecast, `optimize`, `rename`, rename detection, `completions` | Optimizer needs a few hundred reviews; the command says so when there are too few. |
| 5 | Polish and ship | Help examples, README as the web docs, `NO_COLOR`, release binaries for macOS and Linux via GitHub Actions, `cargo install reword-cli`. |

Later, only if earned by real use: multi-line cards, cost-aware planner,
FSRS-7, per-deck parameter groups, opt-in language normalizers, a `sync`
wrapper around git.

## 8. Deliberately left out of v1

- Any GUI, web UI, or full-screen TUI.
- Audio, images, cloze deletions, HTML in cards.
- Streaks, badges, notifications.
- Multiple users, remote servers, telemetry of any kind.
- Automatic Hard/Easy from response time (the report says validate first).
- Any default that assumes what a deck is about.

## 9. Decisions made

- Session mode is typed or recall, chosen per session, never mixed.
- Deck syntax is the Obsidian `::` / `:::` convention.
- Data lives in `~/reword` by default.
- History is logged per deck, beside the deck file, not globally.
- No language-specific or per-deck defaults anywhere.
- Rust with the official `fsrs` crate.
