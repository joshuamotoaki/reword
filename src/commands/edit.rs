//! `reword edit [DECK]`: open the deck file in $VISUAL / $EDITOR, then check it.

use super::Ctx;
use crate::error::{Error, Result};
use crate::out;
use crate::store::deck_name_from_arg;
use crate::term;
use crate::text::plural;

pub fn run(ctx: &Ctx, deck: Option<&str>) -> Result<i32> {
    let store = &ctx.store;
    store.require()?;
    let decks = store.list_decks()?;
    let name = match deck {
        Some(d) => store.resolve_deck(d)?,
        None => match decks.as_slice() {
            [only] => only.clone(),
            [] => {
                return Err(
                    Error::new("no decks yet").hint("Create one: reword add DECK FRONT BACK")
                );
            }
            many => {
                if !ctx.term.interactive {
                    return Err(Error::new("several decks exist; say which")
                        .hint(format!("Decks: {}", many.join(", "))));
                }
                let answer = term::ask(&format!("Deck [{}]: ", many.join(", ")))?
                    .ok_or_else(|| Error::new("no deck given"))?;
                store.resolve_deck(&deck_name_from_arg(&answer))?
            }
        },
    };
    let path = store.deck_path(&name);

    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|v| !v.trim().is_empty())
        });
    let Some(editor) = editor else {
        return Err(Error::new(format!(
            "$EDITOR is not set; the deck is at {}",
            path.display()
        ))
        .hint("Set it, e.g. `export EDITOR=vim`, or open the file directly."));
    };
    if !ctx.term.interactive {
        return Err(Error::new(format!(
            "edit needs a terminal; the deck is at {}",
            path.display()
        )));
    }

    let before = store.load_deck(&name)?.cards.len();
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("vi");
    let status = std::process::Command::new(program)
        .args(parts)
        .arg(&path)
        .status()
        .map_err(|e| Error::new(format!("could not run {program}: {e}")))?;
    if !status.success() {
        return Err(Error::new(format!("{program} exited with {status}")));
    }

    let deck = store.load_deck(&name)?;
    for w in &deck.warnings {
        out::warn(&ctx.term, &w.to_string());
    }
    let after = deck.cards.len();
    let delta = after as i64 - before as i64;
    let change = match delta {
        0 => String::new(),
        d if d > 0 => format!(" (+{d})"),
        d => format!(" ({d})"),
    };
    out::note(
        &ctx.term,
        &format!("{name}: {}{change}", plural(after, "card")),
    );
    Ok(0)
}
