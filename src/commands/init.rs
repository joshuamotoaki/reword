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

pub fn run(ctx: &Ctx) -> Result<i32> {
    let store = &ctx.store;
    let existed = store.exists();
    std::fs::create_dir_all(store.decks_dir())?;
    let mut created = Vec::new();

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
