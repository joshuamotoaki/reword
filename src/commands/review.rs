//! `reword review [DECK...]`: plan a session and run it.

use std::io::Write;

use super::Ctx;
use crate::cli::ReviewArgs;
use crate::clock::{days_between, in_days_label};
use crate::deck::{collect_headings, normalize_section};
use crate::error::{Error, Result};
use crate::history::{Row, RowKind};
use crate::out;
use crate::planner::{self, NewPolicy};
use crate::session;
use crate::store::LoadedDeck;
use crate::term::{self, Key};
use crate::text::{cards_come_due, plural};
use crate::types::{Goal, Mode};

pub fn run(ctx: &Ctx, args: ReviewArgs) -> Result<i32> {
    let store = &ctx.store;
    store.require()?;
    let names = ctx.deck_names(&args.decks)?;
    if names.is_empty() {
        return Err(
            Error::new("no decks yet").hint("Add a card to create one: reword add DECK FRONT BACK")
        );
    }
    let mut decks = store.load_all(&names)?;
    ctx.report_warnings(&decks);
    if matches!(args.cards, Some(0)) {
        return Err(Error::new("-n needs at least 1 card"));
    }
    if let Some(under) = &args.under {
        apply_under(&mut decks, under)?;
    }
    if !ctx.term.interactive {
        return Err(Error::new(
            "review needs an interactive terminal (stdin and stdout must be a TTY, without --no-input)",
        ));
    }
    let mut settings = ctx.settings()?;
    if let Some(minutes) = args.minutes {
        settings.session_minutes = minutes.max(1);
    }
    let model = ctx.model(&settings)?;
    let today = ctx.clock.today();

    detect_renames(ctx, &mut decks)?;

    // The daily new-card cap counts every deck, not just the ones reviewed.
    let all_names = store.list_decks()?;
    let mut introduced = planner::introduced_today(&decks, &ctx.clock, today);
    for other in all_names.iter().filter(|n| !names.contains(n)) {
        let (ledger, _) = store.load_ledger(other)?;
        introduced += ledger
            .iter()
            .filter(|(_, _, evs)| ctx.clock.study_day(evs[0].ts) == today)
            .count();
    }

    let policy = if args.no_new {
        NewPolicy::Explicit(0)
    } else if let Some(n) = args.new {
        NewPolicy::Explicit(n)
    } else if args.endless {
        NewPolicy::All
    } else {
        NewPolicy::Daily
    };
    let plan = planner::plan(
        &decks,
        &model,
        &ctx.clock,
        today,
        &settings,
        policy,
        introduced,
        args.endless,
    );
    let session_names: Vec<String> = decks.iter().map(|d| d.name().to_string()).collect();
    let mut label = if args.decks.is_empty() && session_names.len() > 1 {
        format!("all decks ({})", session_names.len())
    } else {
        session_names.join(", ")
    };
    if let Some(under) = &args.under {
        label = format!("{label} · {under}");
    }

    if plan.is_empty() {
        let style = ctx.term.out;
        if plan.total_goals == 0 {
            out::println(&format!(
                "{label} has no cards. Add some: {} or edit {}.",
                style.bold(&format!("reword add {}", names[0])),
                store.deck_label(&names[0])
            ));
            return Ok(0);
        }
        let mut msg = format!("Nothing due in {label}.");
        if let Some((day, n)) = plan.next_due {
            msg.push_str(&format!(
                " {} {}.",
                cards_come_due(n),
                in_days_label(days_between(today, day))
            ));
        }
        if plan.throttled {
            msg.push_str(" New cards are paused while the backlog is large.");
        } else if plan.new_waiting() > 0 {
            let n = plan.new_waiting();
            msg.push_str(&format!(
                " {} waiting; {} to learn ahead.",
                if n == 1 {
                    "1 new card is".to_string()
                } else {
                    format!("{n} new cards are")
                },
                style.bold("--new 5")
            ));
        }
        if plan.learned > 0 {
            msg.push_str(&format!(" {} keeps going anyway.", style.bold("--endless")));
        }
        out::println(&msg);
        return Ok(0);
    }

    let mode = match (args.typed, args.recall, settings.mode) {
        (true, _, _) => Mode::Typed,
        (_, true, _) => Mode::Recall,
        (_, _, Some(m)) => m,
        _ => match ask_mode()? {
            Some(m) => m,
            None => return Ok(0),
        },
    };
    let minutes = if args.cards.is_some() && args.minutes.is_none() {
        None
    } else {
        Some(settings.session_minutes)
    };
    let cards = args.cards.filter(|&n| n > 0);
    let label = if args.endless {
        format!("{label} · endless")
    } else {
        label
    };

    let style = ctx.term.out;
    let new_waiting = plan.new_waiting();
    let next_due = plan.next_due;
    let summary = session::run(
        store,
        &decks,
        plan,
        &model,
        &ctx.clock,
        &ctx.term,
        session::Options {
            mode,
            minutes,
            cards,
            endless: args.endless,
            label,
        },
    )?;

    let deck_arg = if args.decks.is_empty() {
        String::new()
    } else {
        format!(" {}", names.join(" "))
    };
    if !args.endless {
        if summary.remaining > 0 {
            out::println(&format!(
                "{} remain. Next: {}",
                plural(summary.remaining, "card"),
                style.bold(&format!("reword review{deck_arg}"))
            ));
        } else if new_waiting > 0 {
            out::println(&format!(
                "All caught up. {} waiting: {}",
                if new_waiting == 1 {
                    "1 new card is".to_string()
                } else {
                    format!("{new_waiting} new cards are")
                },
                style.bold(&format!("reword review{deck_arg} --new 5"))
            ));
        } else if let Some((day, n)) = next_due {
            out::println(&format!(
                "All caught up. {} {}. {} keeps going.",
                cards_come_due(n),
                in_days_label(days_between(today, day)),
                style.bold(&format!("reword review{deck_arg} --endless"))
            ));
        } else {
            out::println("All caught up.");
        }
    }
    Ok(0)
}

fn apply_under(decks: &mut Vec<LoadedDeck>, under: &str) -> Result<()> {
    let needle = normalize_section(under);
    if needle.is_empty() {
        return Err(
            Error::new("--under needs a heading").hint("Example: reword review --under food")
        );
    }
    let available = {
        let mut out = Vec::new();
        for d in decks.iter() {
            for h in collect_headings(&d.deck.cards) {
                if !out.iter().any(|x| x == &h) {
                    out.push(h);
                }
            }
        }
        out
    };
    for d in decks.iter_mut() {
        d.deck.cards.retain(|c| c.under(&needle));
    }
    decks.retain(|d| !d.deck.cards.is_empty());
    if decks.is_empty() {
        let hint = if available.is_empty() {
            "Add a `# Food` heading above a group of cards.".into()
        } else {
            format!("Headings: {}", available.join(", "))
        };
        return Err(Error::new(format!("no cards under \"{under}\"")).hint(hint));
    }
    Ok(())
}

/// Ask for the mode. Enter always picks recall.
fn ask_mode() -> Result<Option<Mode>> {
    print!("Mode: [r]ecall or [t]yped? (enter = recall) ");
    let _ = std::io::stdout().flush();
    loop {
        let key = term::read_key()?;
        let chosen = match key {
            Key::Char('r') | Key::Char('R') | Key::Enter | Key::Space => Some(Mode::Recall),
            Key::Char('t') | Key::Char('T') => Some(Mode::Typed),
            Key::Char('q') | Key::Esc | Key::CtrlC | Key::CtrlD => {
                term::clear_line();
                return Ok(None);
            }
            _ => continue,
        };
        term::clear_line();
        return Ok(chosen);
    }
}

/// If exactly one history is orphaned and exactly one never-reviewed card
/// sits among the reviewed ones, ask whether that was a rename. New cards
/// are introduced in line order, so an unseen card before the last reviewed
/// line is the anomaly; unseen cards after it are just the deck's future.
/// Anything less clear-cut is left to `reword check`.
fn detect_renames(ctx: &Ctx, decks: &mut [LoadedDeck]) -> Result<()> {
    for ld in decks.iter_mut() {
        let orphans: Vec<String> = ld
            .ledger
            .fronts()
            .filter(|f| !ld.deck.has(f))
            .map(str::to_string)
            .collect();
        if orphans.len() != 1 {
            continue;
        }
        let seen = |c: &crate::deck::Card| {
            !ld.ledger.reviews(&c.front, Goal::Forward).is_empty()
                || !ld.ledger.reviews(&c.front, Goal::Reverse).is_empty()
        };
        let frontier = ld
            .deck
            .cards
            .iter()
            .filter(|c| seen(c))
            .map(|c| c.line)
            .max();
        let unseen: Vec<&str> = ld
            .deck
            .cards
            .iter()
            .filter(|c| !seen(c) && frontier.is_none_or(|f| c.line < f))
            .map(|c| c.front.as_str())
            .collect();
        if unseen.len() != 1 {
            continue;
        }
        let (old, new) = (orphans[0].clone(), unseen[0].to_string());
        if term::confirm(
            &format!("Did you rename \"{old}\" to \"{new}\" in {}?", ld.name()),
            false,
        )? {
            let row = Row {
                ts: ctx.clock.now(),
                front: old.clone(),
                kind: RowKind::Rename { to: new.clone() },
            };
            ctx.store.append_row(ld.name(), &row)?;
            let (ledger, _) = ctx.store.load_ledger(ld.name())?;
            ld.ledger = ledger;
            out::note(
                &ctx.term,
                &format!("History of \"{old}\" now belongs to \"{new}\"."),
            );
        }
    }
    Ok(())
}
