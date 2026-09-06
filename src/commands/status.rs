//! `reword` with no subcommand: where things stand, and what to do next.

use super::{Ctx, count, summarize, summary_json};
use crate::clock::days_between;
use crate::clock::{ago, in_days_label};
use crate::error::Result;
use crate::out;
use crate::term::Style;
use crate::text::{cards_come_due, pad_right, plural, width};

pub fn run(ctx: &Ctx) -> Result<i32> {
    let store = &ctx.store;
    if !store.exists() {
        if ctx.json {
            out::json(
                &serde_json::json!({ "dir": store.root.display().to_string(), "initialized": false }),
            );
            return Ok(0);
        }
        out::println(&format!(
            "reword: plain-text flashcards with spaced repetition.\n\n\
             No data directory at {} yet.\n\n  \
             reword init        create it with an example deck\n  \
             reword --help      all commands",
            store.display()
        ));
        return Ok(0);
    }

    let names = store.list_decks()?;
    let decks = store.load_all(&names)?;
    ctx.report_warnings(&decks);
    let settings = ctx.settings()?;
    let model = ctx.model(&settings)?;
    let now = ctx.clock.now();
    let today = ctx.clock.today();
    let summaries = summarize(&decks, &model, &ctx.clock, &settings, today);
    let total_cards: usize = summaries.iter().map(|s| s.cards).sum();
    let total_due: usize = summaries.iter().map(|s| s.due).sum();
    let total_new: usize = summaries.iter().map(|s| s.new_available).sum();

    if ctx.json {
        out::json(&serde_json::json!({
            "dir": store.root.display().to_string(),
            "initialized": true,
            "cards": total_cards,
            "due": total_due,
            "new": total_new,
            "decks": summaries.iter().map(|s| summary_json(s, now)).collect::<Vec<_>>(),
        }));
        return Ok(0);
    }

    let style = ctx.term.out;
    out::println(&format!(
        "{} · {} · {}",
        style.bold(&store.display()),
        plural(summaries.len(), "deck"),
        plural(total_cards, "card")
    ));

    if summaries.is_empty() {
        out::println("");
        out::println("No decks yet. Add a card to create one:");
        out::println("  reword add DECK FRONT BACK");
        out::println("or drop a file into decks/ with one `front::back` card per line.");
        return Ok(0);
    }

    let name_w = summaries.iter().map(|s| width(&s.name)).max().unwrap_or(4);
    for s in &summaries {
        let last = match s.last_review {
            Some(t) => format!("reviewed {}", ago(t, now)),
            None => "never reviewed".into(),
        };
        out::println(&format!(
            "  {}  {} due  {} new   {}",
            pad_right(&s.name, name_w),
            count(style, s.due, 3, Style::yellow),
            count(style, s.new_available, 3, Style::cyan),
            style.dim(&last),
        ));
    }

    out::println("");
    if total_due > 0 || total_new > 0 {
        out::println(&format!("Next: {}", style.bold("reword review")));
    } else {
        let next = summaries
            .iter()
            .filter_map(|s| s.next_due)
            .min_by_key(|(d, _)| *d)
            .map(|(d, n)| {
                format!(
                    " {} {}.",
                    cards_come_due(n),
                    in_days_label(days_between(today, d))
                )
            })
            .unwrap_or_default();
        out::println(&format!("Nothing due.{next}"));
    }
    out::println("");
    out::println(
        &style.dim("Commands: review · add · edit · decks · check · stats · optimize · help"),
    );
    Ok(0)
}
