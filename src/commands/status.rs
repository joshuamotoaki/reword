//! `reword` with no subcommand: where things stand, and what to do next.

use super::{Ctx, count, leech_json, leeches, progress_bar, summarize, summary_json};
use crate::clock::{add_days, ago, days_between, in_days_label};
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
    let worst = leeches(&decks);

    if ctx.json {
        out::json(&serde_json::json!({
            "dir": store.root.display().to_string(),
            "initialized": true,
            "cards": total_cards,
            "due": total_due,
            "new": total_new,
            "leeches": leech_json(&worst),
            "decks": summaries.iter().map(|s| summary_json(s, now)).collect::<Vec<_>>(),
        }));
        return Ok(0);
    }

    let style = ctx.term.out;
    out::println(&format!(
        " {}  {}",
        style.gradient("reword"),
        style.dim(&format!(
            "{} · {} · {}",
            store.display(),
            plural(summaries.len(), "deck"),
            plural(total_cards, "card")
        ))
    ));

    if summaries.is_empty() {
        out::println("");
        out::println("No decks yet. Add a card to create one:");
        out::println(&format!("  {}", style.bold("reword add DECK FRONT BACK")));
        out::println("or drop a file into decks/ with one `front::back` card per line.");
        return Ok(0);
    }

    let goals: usize = summaries.iter().map(|s| s.goals).sum();
    let learned: usize = summaries.iter().map(|s| s.learned).sum();
    let bar_w = ctx.term.width().saturating_sub(44).clamp(10, 24);
    out::println(&format!(
        " {}   {} · {} · {}",
        progress_bar(style, goals, learned, total_due, bar_w),
        style.green(&format!("{learned} learned")),
        if total_due > 0 {
            style.yellow(&format!("{total_due} due"))
        } else {
            style.dim("0 due")
        },
        style.dim(&format!("{} unseen", goals - learned)),
    ));
    out::println("");

    // Today: reviews so far, and the streak of consecutive study days.
    let mut days = std::collections::BTreeSet::new();
    let mut reviewed_today = 0usize;
    for d in &decks {
        for (_, _, evs) in d.ledger.iter() {
            for ev in evs {
                let day = ctx.clock.study_day(ev.ts);
                days.insert(day);
                if day == today {
                    reviewed_today += 1;
                }
            }
        }
    }
    let mut streak = 0;
    let mut day = if days.contains(&today) {
        today
    } else {
        add_days(today, -1)
    };
    while days.contains(&day) {
        streak += 1;
        day = add_days(day, -1);
    }
    let mut today_parts = vec![if reviewed_today > 0 {
        format!("{} reviewed", style.bold(&reviewed_today.to_string()))
    } else {
        "nothing reviewed yet".to_string()
    }];
    if total_new > 0 {
        today_parts.push(style.cyan(&format!("{total_new} new ready")));
    }
    if streak > 0 {
        today_parts.push(format!("{} streak", style.bold(&plural(streak, "day"))));
    }
    out::println(&format!(
        " {}  {}",
        pad_right("today", 9),
        today_parts.join(" · ")
    ));
    if !worst.is_empty() {
        let multi = decks.len() > 1;
        let names: Vec<String> = worst
            .iter()
            .map(|l| {
                let p = crate::text::truncate(&l.prompt, 18);
                if multi { format!("{} {p}", l.deck) } else { p }
            })
            .collect();
        out::println(&format!(
            " {}  {}",
            pad_right("leeches", 9),
            style.dim(&names.join(" · "))
        ));
    }
    out::println("");

    let name_w = summaries.iter().map(|s| width(&s.name)).max().unwrap_or(4);
    for s in &summaries {
        let mut tail = Vec::new();
        match s.last_review {
            Some(t) => tail.push(format!("reviewed {}", ago(t, now))),
            None => tail.push("never reviewed".into()),
        }
        if s.due == 0
            && let Some((d, n)) = s.next_due
        {
            tail.push(format!(
                "{} {}",
                cards_come_due(n),
                in_days_label(days_between(today, d))
            ));
        }
        out::println(&format!(
            " {}  {} {}  {} {}   {}",
            style.sgr("1;36", &pad_right(&s.name, name_w)),
            count(style, s.due, 3, Style::yellow),
            style.dim("due"),
            count(style, s.new_available, 3, Style::cyan),
            style.dim("new"),
            style.dim(&tail.join(" · ")),
        ));
    }

    out::println("");
    if total_due > 0 || total_new > 0 {
        out::println(&format!(
            " {}  {}",
            style.magenta("next "),
            style.sgr("1;32", "reword review")
        ));
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
        out::println(&format!(
            " {}  nothing due.{next} {} keeps going.",
            style.magenta("next "),
            style.sgr("1;32", "reword review --endless")
        ));
    }
    out::println("");
    out::println(&style.dim(" review · add · edit · decks · check · stats · optimize · help"));
    Ok(0)
}
