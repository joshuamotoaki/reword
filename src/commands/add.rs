//! `reword add [DECK] [FRONT] [BACK]`: append one card line.

use std::io::Write;

use super::Ctx;
use crate::cli::AddArgs;
use crate::deck::{Deck, front_is_ignored};
use crate::error::{Error, Result, bail};
use crate::out;
use crate::store::{Store, deck_name_from_arg};
use crate::term;
use crate::text::{front_key, nfc};

pub fn run(ctx: &Ctx, args: AddArgs) -> Result<i32> {
    let store = &ctx.store;
    store.require()?;
    let decks = store.list_decks()?;
    let interactive = ctx.term.interactive;
    let usage_hint = "Usage: reword add DECK FRONT BACK";

    // Sort the positional words into deck / front / back without guessing:
    // a word that names an existing deck is the deck; with exactly one deck
    // and no deck word, the words are front and back.
    let words: Vec<String> = [args.deck, args.front, args.back]
        .into_iter()
        .flatten()
        .collect();
    let (deck_arg, mut front, mut back): (Option<String>, Option<String>, Option<String>) =
        match words.len() {
            3 => (
                Some(words[0].clone()),
                Some(words[1].clone()),
                Some(words[2].clone()),
            ),
            0 => (None, None, None),
            _ => {
                let first = deck_name_from_arg(&words[0]);
                if decks.contains(&first) || decks.len() != 1 {
                    (Some(words[0].clone()), words.get(1).cloned(), None)
                } else {
                    (None, Some(words[0].clone()), words.get(1).cloned())
                }
            }
        };

    let deck = match deck_arg {
        Some(d) => deck_name_from_arg(&d),
        None => match decks.as_slice() {
            [only] => only.clone(),
            [] => {
                if !interactive {
                    return Err(Error::new("no decks exist yet").hint(usage_hint));
                }
                term::ask("Deck name: ")?.ok_or_else(|| Error::new("no deck given"))?
            }
            many => {
                if !interactive {
                    return Err(Error::new("several decks exist; say which")
                        .hint(format!("Decks: {}. {usage_hint}", many.join(", "))));
                }
                let answer = term::ask(&format!("Deck [{}]: ", many.join(", ")))?
                    .ok_or_else(|| Error::new("no deck given"))?;
                deck_name_from_arg(&answer)
            }
        },
    };
    Store::validate_new_deck_name(&deck)?;

    let path = store.deck_path(&deck);
    if !path.is_file() {
        if !interactive {
            return Err(Error::new(format!("no deck named \"{deck}\""))
                .hint(format!("Decks: {}. Create it by writing decks/{deck}.md, or run without --no-input to be asked.", if decks.is_empty() { "none".into() } else { decks.join(", ") })));
        }
        if !term::confirm(
            &format!("No deck named \"{deck}\". Create decks/{deck}.md?"),
            true,
        )? {
            return Ok(1);
        }
    }

    if front.is_none() {
        if !interactive {
            return Err(Error::new("no front given").hint(usage_hint));
        }
        front = term::ask("Front: ")?;
    }
    let Some(front_raw) = front else {
        bail!("no front given")
    };
    if back.is_none() {
        if !interactive {
            return Err(Error::new("no back given").hint(usage_hint));
        }
        back = term::ask("Back:  ")?;
    }
    let Some(back_raw) = back else {
        bail!("no back given")
    };

    let front = front_key(&front_raw);
    let back = back_raw.trim().to_string();
    if front.is_empty() || back.is_empty() {
        bail!("front and back cannot be empty");
    }
    if front.contains("::") {
        bail!("the front cannot contain \"::\" (it is the card separator)");
    }
    if front_is_ignored(&front) {
        bail!(
            "the front cannot start with \"#\" or \"```\"; the deck parser treats such lines as comments"
        );
    }
    if !args.reverse && back.contains(":::") {
        bail!("a \"::\" card's back cannot contain \":::\"; use --reverse or rephrase");
    }
    if front.contains('\n') || back.contains('\n') {
        bail!("cards are one line each");
    }

    if path.is_file() {
        let existing = store.load_deck(&deck)?;
        if let Some(card) = existing.find(&front) {
            return Err(Error::new(format!(
                "{} already has \"{front}\" on line {}",
                store.deck_label(&deck),
                card.line
            ))
            .hint(format!("Edit that line instead: reword edit {deck}")));
        }
    }

    let sep = if args.reverse { ":::" } else { "::" };
    let line = format!("{front}{sep}{back}");
    let mut text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let needs_newline = !text.is_empty() && !text.ends_with('\n');
    if needs_newline {
        text.push('\n');
    }
    text.push_str(&line);
    text.push('\n');
    let parsed = Deck::parse(&deck, &store.deck_label(&deck), &text);
    if !parsed
        .find(&front)
        .is_some_and(|card| card.back == nfc(&back) && card.reverse == args.reverse)
    {
        bail!(
            "card would not parse as entered; check separator-adjacent colons and unclosed code fences"
        );
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    if needs_newline {
        f.write_all(b"\n")?;
    }
    writeln!(f, "{line}")?;
    out::note(&ctx.term, &format!("Added to {deck}: {line}"));
    Ok(0)
}
