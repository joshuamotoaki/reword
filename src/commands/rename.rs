//! `reword rename DECK OLD NEW`: change a front and carry its history along.

use super::Ctx;
use crate::deck::{front_is_ignored, split_card};
use crate::error::{Error, Result, bail};
use crate::history::{Row, RowKind};
use crate::out;
use crate::text::{front_key, plural};
use crate::types::Goal;

pub fn run(ctx: &Ctx, deck_arg: &str, old: &str, new: &str) -> Result<i32> {
    let store = &ctx.store;
    store.require()?;
    let name = store.resolve_deck(deck_arg)?;
    let loaded = store.load(&name)?;
    let old_key = front_key(old);
    let new_key = front_key(new);
    if old_key.is_empty() || new_key.is_empty() {
        bail!("fronts cannot be empty");
    }
    if old_key == new_key {
        bail!("old and new fronts are the same");
    }
    if new_key.contains("::") {
        bail!("the new front cannot contain \"::\"");
    }
    if front_is_ignored(&new_key) {
        bail!(
            "the new front cannot start with \"#\" or \"```\"; the deck parser treats such lines as comments"
        );
    }

    let in_deck_old = loaded.deck.find(&old_key).cloned();
    let in_deck_new = loaded.deck.find(&new_key).cloned();
    let history: usize = [Goal::Forward, Goal::Reverse]
        .iter()
        .map(|g| loaded.ledger.reviews(&old_key, *g).len())
        .sum();
    let has_history = loaded.ledger.fronts().any(|f| f == old_key);

    if in_deck_old.is_none() && !has_history {
        return Err(
            Error::new(format!("{name} has no card or history named \"{old_key}\""))
                .hint("Fronts are compared after trimming; check the spelling."),
        );
    }
    if let (Some(o), Some(n)) = (&in_deck_old, &in_deck_new) {
        return Err(Error::new(format!(
            "{name} has both \"{old_key}\" (line {}) and \"{new_key}\" (line {})",
            o.line, n.line
        ))
        .hint("Delete one of the lines, then rename."));
    }

    if has_history {
        let row = Row {
            ts: ctx.clock.now(),
            front: old_key.clone(),
            kind: RowKind::Rename {
                to: new_key.clone(),
            },
        };
        store.append_row(&name, &row)?;
    }

    let mut line_note = String::new();
    if let Some(card) = in_deck_old {
        let path = store.deck_path(&name);
        let text = std::fs::read_to_string(&path)?;
        let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
        let idx = card.line - 1;
        let raw = lines.get(idx).cloned().unwrap_or_default();
        let Some((front_part, _, _)) = split_card(&raw) else {
            bail!(
                "{}:{}: not a card line any more; edit the deck and retry",
                store.deck_label(&name),
                card.line
            )
        };
        let lead_len = front_part.len() - front_part.trim_start().len();
        let trail_len = front_part.len() - front_part.trim_end().len();
        let rest = &raw[front_part.len()..];
        let rebuilt = format!(
            "{}{new_key}{}{rest}",
            &front_part[..lead_len],
            &front_part[front_part.len() - trail_len..]
        );
        lines[idx] = rebuilt;
        let tmp = path.with_extension("md.tmp");
        std::fs::write(&tmp, lines.concat())?;
        std::fs::rename(&tmp, &path)?;
        line_note = format!(", line {} updated", card.line);
    }

    let carried = if has_history {
        format!("{} carried over", plural(history, "review"))
    } else {
        "no history to carry".into()
    };
    out::note(
        &ctx.term,
        &format!("Renamed \"{old_key}\" → \"{new_key}\" in {name} ({carried}{line_note})"),
    );
    Ok(0)
}
