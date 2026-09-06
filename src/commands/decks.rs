//! `reword decks`: every deck with its counts.

use super::{Ctx, summarize, summary_json};
use crate::clock::ago;
use crate::error::Result;
use crate::out;
use crate::text::{pad_left, pad_right, width};

pub fn run(ctx: &Ctx) -> Result<i32> {
    let store = &ctx.store;
    store.require()?;
    let names = store.list_decks()?;
    let decks = store.load_all(&names)?;
    ctx.report_warnings(&decks);
    let settings = ctx.settings()?;
    let model = ctx.model(&settings)?;
    let now = ctx.clock.now();
    let today = ctx.clock.today();
    let summaries = summarize(&decks, &model, &ctx.clock, &settings, today);
    let strays = store.stray_logs()?;

    if ctx.json {
        out::json(&serde_json::json!({
            "decks": summaries.iter().map(|s| summary_json(s, now)).collect::<Vec<_>>(),
            "stray_logs": strays,
        }));
        return Ok(0);
    }

    if summaries.is_empty() {
        out::println("No decks. Add a card to create one: reword add DECK FRONT BACK");
    }
    let name_w = summaries
        .iter()
        .map(|s| width(&s.name))
        .max()
        .unwrap_or(4)
        .max(4);
    let style = ctx.term.out;
    out::println(&style.dim(&format!(
        "  {}  cards  learned  due  new   last review",
        pad_right("deck", name_w)
    )));
    for s in &summaries {
        let last = s
            .last_review
            .map(|t| ago(t, now))
            .unwrap_or_else(|| "never".into());
        out::println(&format!(
            "  {}  {}  {}  {}  {}   {last}",
            pad_right(&s.name, name_w),
            pad_left(&s.cards.to_string(), 5),
            pad_left(&s.learned.to_string(), 7),
            pad_left(&s.due.to_string(), 3),
            pad_left(&s.new_available.to_string(), 3),
        ));
    }
    for stray in &strays {
        out::warn(
            &ctx.term,
            &format!(
                "{} has no deck file beside it (decks/{stray}.md). Restore the deck or delete the log.",
                store.log_label(stray)
            ),
        );
    }
    Ok(0)
}
