# Reword deck format

Reword is a local spaced-repetition app. Decks are plain Markdown files in `~/reword/decks/`. The deck name is the filename without `.md` (no slashes, no leading dot). History lives in a matching `.log` next to the deck; don't invent or edit logs.

## Cards

One card per line:

```
front::back
front:::back
```

- `::` asks front → back only.
- `:::` asks both ways. Each direction has its own memory. The reverse side waits until the forward side has been recalled once, and both sides are never in the same session.
- Spaces around `::` / `:::` do not matter. `食::to eat` is the usual style.
- A line splits on the first `:::`, otherwise the first `::`. So `Vec::new():::constructor` is a two-way card whose front is `Vec::new()`.
- Cards are a single line. No newlines inside a front or back.

Headings, prose, and blank lines are ignored. Lines starting with `#` and fenced ` ``` ` blocks are never cards, so `# 食::to eat` comments a card out.

```
# Cantonese, food

食::to eat
飲:::to drink
唔該::excuse me / thank you
```

## Identity and matching

- The **front is the card's identity** in that deck. It must be unique. Duplicates are skipped (first line wins).
- Front and back cannot be empty.
- The front cannot contain `::` (that is the separator).
- The front cannot start with `#` or ` ``` ` (those lines are comments).
- A one-way (`::`) back cannot contain `:::`, or the line becomes two-way.
- Fronts are compared after trim + Unicode NFC. Changing a front orphans its history.

Typed-mode answers are matched after NFC, trim, collapsing internal space, and lowercasing. Nothing else — no fuzzy matching, no ignoring punctuation.

` / ` (space, slash, space) on the **answer** side separates alternatives. Any one of them counts as correct:

```
2 + 2::4 / four
唔該::excuse me / thank you
```

`4/four` (no spaces) is one answer, not two.

## What makes a good card

- One fact per card, short enough to grade in a few seconds.
- `:::` when both sides work as a prompt: word ↔ meaning, short unique pairs.
- `::` when only one direction makes sense: a question, a sentence to translate, or a back that would be a bad prompt (long, generic, or shared by many cards).
- If you use `:::`, the back must also be a good unique prompt. Disambiguate (`to eat (food)`) instead of repeating a generic reverse like `to be`.
- Extra senses go on the answer with ` / `, or on another card.
- Cards have to be `::` / `:::` lines. Numbered lists and tables are not cards.

When adding to an existing deck, keep the fronts of cards that already exist, and match the file's style (headings, `::` vs `:::`, spacing).
