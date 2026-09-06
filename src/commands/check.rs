//! `reword check`: everything that could be wrong with the files, in one list.

use super::Ctx;
use crate::error::Result;
use crate::out;
use crate::text::plural;
use crate::types::Goal;

pub fn run(ctx: &Ctx) -> Result<i32> {
    let store = &ctx.store;
    store.require()?;
    let names = store.list_decks()?;
    let mut problems = 0usize;
    let mut notes = 0usize;
    let mut cards = 0usize;
    let mut lines: Vec<String> = Vec::new();

    for name in &names {
        let loaded = store.load(name)?;
        cards += loaded.deck.cards.len();
        for w in &loaded.warnings {
            problems += 1;
            lines.push(w.to_string());
        }
        let ledger = &loaded.ledger;
        let mut orphans: Vec<(String, Goal, usize)> = Vec::new();
        for (key, evs) in ledger.iter() {
            let (front, goal) = key;
            let present = match loaded.deck.find(front) {
                Some(card) => *goal == Goal::Forward || card.reverse,
                None => false,
            };
            if !present {
                orphans.push((front.clone(), *goal, evs.len()));
            }
        }
        orphans.sort();
        for (front, goal, n) in orphans {
            notes += 1;
            let what = if goal == Goal::Reverse {
                " (reverse side)"
            } else {
                ""
            };
            lines.push(format!(
                "{}: history for \"{front}\"{what} has no card ({}). Re-add the card to resume it, or carry it over: reword rename {name} \"{front}\" NEW",
                store.log_label(name),
                plural(n, "review")
            ));
        }
    }
    for stray in store.stray_logs()? {
        problems += 1;
        lines.push(format!(
            "{}: no deck file beside it (decks/{stray}.md). Restore the deck or delete the log.",
            store.log_label(&stray)
        ));
    }

    if ctx.json {
        out::json(
            &serde_json::json!({ "decks": names.len(), "cards": cards, "problems": problems, "notes": notes, "messages": lines }),
        );
        return Ok(if problems > 0 { 1 } else { 0 });
    }

    for l in &lines {
        out::println(l);
    }
    if !lines.is_empty() {
        out::println("");
    }
    let verdict = if problems == 0 {
        "no problems".to_string()
    } else {
        plural(problems, "problem")
    };
    let notes_s = if notes > 0 {
        format!(", {}", plural(notes, "note"))
    } else {
        String::new()
    };
    out::println(&format!(
        "{}, {}: {verdict}{notes_s}",
        plural(names.len(), "deck"),
        plural(cards, "card")
    ));
    Ok(if problems > 0 { 1 } else { 0 })
}
