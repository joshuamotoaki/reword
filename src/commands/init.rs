//! `reword init`: create the data directory.

use super::Ctx;
use crate::config::CONFIG_TEMPLATE;
use crate::error::Result;
use crate::out;

const EXAMPLE_DECK: &str = "\
# Example deck
#
# One card per line, front::back. A ::: line is asked in both directions.
# Alternatives on the answer side are separated by \" / \".
# Lines that are not cards, like these, are ignored.
# Edit this file, or replace it with your own decks in this folder.

what is this file::an example deck; edit it or delete it
front:::back
2 + 2::4 / four
";

const GITATTRIBUTES_LINE: &str = "*.log merge=union";

const README: &str = "\
# Reword

This folder is the database. Decks are ordinary Markdown files. Review
history lives next to them. Sync the whole directory if you want the
same cards on another machine.

## Layout

```
.
  README.md          this file
  config.toml        settings; every key optional
  params.toml        written by `reword optimize`
  decks/
    cantonese.md     the cards
    cantonese.log    that deck's review log, append-only
  .gitattributes     lets git merge logs from two machines
```

A deck is `decks/<name>.md`. The name is the filename without `.md`
(no slashes, no leading dot). Its log is `decks/<name>.log`.

## Adding a deck by hand

Create a `.md` file in `decks/`. That is enough. Reword will see it
the next time you run `reword`, `reword decks`, or `reword review`.

```
decks/cantonese.md
```

```
# Cantonese, food
食::to eat
飲:::to drink
唔該::excuse me / thank you
```

One card per line:

- `front::back` asks front → back.
- `front:::back` asks both ways. Each direction has its own memory.
- ` / ` on the answer side separates alternatives for typed mode.
- Headings, prose, and blank lines are ignored. A line starting with
  `#` that is also a card (`# 食::to eat`) comments that card out.

Or let the tool do it: `reword add cantonese 食 'to eat'` creates the
file if needed and appends a card. `-r` writes `:::`. `reword edit`
opens a deck in `$EDITOR`.

The front is the card's identity in that deck. Keep fronts unique.
Changing a front orphans its history; `reword rename DECK OLD NEW`
carries the log over.

## Logs

The matching `.log` is created the first time you review that deck.
Do not invent or edit logs. Every grade is appended as a row; memory
is recomputed from the file each run, so there is nothing else to
keep in sync.

If you rename or delete a deck, move or delete the `.md` and `.log`
together. `reword check` reports a `.log` without its `.md`.
";

pub fn run(ctx: &Ctx) -> Result<i32> {
    let store = &ctx.store;
    let existed = store.exists();
    std::fs::create_dir_all(store.decks_dir())?;
    let mut created = Vec::new();

    let readme = store.root.join("README.md");
    if !readme.exists() {
        std::fs::write(&readme, README)?;
        created.push("README.md           how decks and logs are laid out");
    }

    let config = store.config_path();
    if !config.exists() {
        std::fs::write(&config, CONFIG_TEMPLATE)?;
        created.push("config.toml         settings; every key optional");
    }

    if store.list_decks()?.is_empty() {
        std::fs::write(store.deck_path("example"), EXAMPLE_DECK)?;
        created.push("decks/example.md    an example deck to edit or delete");
    }

    let attrs = store.root.join(".gitattributes");
    let current = std::fs::read_to_string(&attrs).unwrap_or_default();
    if !current.lines().any(|l| l.trim() == GITATTRIBUTES_LINE) {
        let mut text = current;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(GITATTRIBUTES_LINE);
        text.push('\n');
        std::fs::write(&attrs, text)?;
        created.push(".gitattributes      lets git merge review logs from two machines");
    }

    let style = ctx.term.out;
    if existed && created.is_empty() {
        out::println(&format!("{} is already set up.", store.display()));
    } else {
        out::println(&format!(
            "{} {}",
            if existed { "Updated" } else { "Created" },
            style.bold(&store.display())
        ));
        for line in created {
            out::println(&format!("  {line}"));
        }
    }
    out::println("");
    out::println(&format!(
        "Next: {}   or add cards with {}",
        style.bold("reword review"),
        style.bold("reword add DECK FRONT BACK")
    ));
    Ok(0)
}
