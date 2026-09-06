//! `reword stats [DECK...]`: retention, pace, and a 14-day forecast.

use std::collections::BTreeMap;

use super::{Ctx, summarize};
use crate::clock::{Clock, days_between};
use crate::error::Result;
use crate::memory::Model;
use crate::out;
use crate::store::LoadedDeck;
use crate::text::{pad_left, plural};

const FORECAST_DAYS: i32 = 14;

struct Stats {
    cards: usize,
    goals: usize,
    learned: usize,
    due: usize,
    new_available: usize,
    reviews_total: usize,
    reviews_7d: usize,
    reviews_30d: usize,
    per_day_14d: f64,
    /// (successes, tests) over the last 30 days, counting only the first
    /// review of a card on a day after it was last seen; new cards excluded.
    retention_30d: (usize, usize),
    minutes_30d: f64,
    forecast: Vec<usize>,
    review_days: usize,
}

fn compute(decks: &[LoadedDeck], model: &Model, clock: &Clock) -> Stats {
    let now = clock.now();
    let today = clock.today();
    let mut s = Stats {
        cards: decks.iter().map(|d| d.deck.cards.len()).sum(),
        goals: 0,
        learned: 0,
        due: 0,
        new_available: 0,
        reviews_total: 0,
        reviews_7d: 0,
        reviews_30d: 0,
        per_day_14d: 0.0,
        retention_30d: (0, 0),
        minutes_30d: 0.0,
        forecast: vec![0; FORECAST_DAYS as usize],
        review_days: 0,
    };
    let mut reviews_14d = 0usize;
    let mut days: BTreeMap<jiff::civil::Date, ()> = BTreeMap::new();
    for d in decks {
        for (_, evs) in d.ledger.iter() {
            let mut prev_day = None;
            for ev in evs {
                s.reviews_total += 1;
                let age = now.duration_since(ev.ts).as_secs();
                let day = clock.study_day(ev.ts);
                days.insert(day, ());
                if age <= 7 * 86_400 {
                    s.reviews_7d += 1;
                }
                if age <= 14 * 86_400 {
                    reviews_14d += 1;
                }
                if age <= 30 * 86_400 {
                    s.reviews_30d += 1;
                    s.minutes_30d += ev.elapsed_ms as f64 / 60_000.0;
                    if let Some(p) = prev_day
                        && day > p
                    {
                        s.retention_30d.1 += 1;
                        if ev.grade.is_success() {
                            s.retention_30d.0 += 1;
                        }
                    }
                }
                prev_day = Some(day);
            }
        }
    }
    s.per_day_14d = reviews_14d as f64 / 14.0;
    s.review_days = days.len();

    for d in decks {
        for card in &d.deck.cards {
            let goals: &[crate::types::Goal] = if card.reverse {
                &[crate::types::Goal::Forward, crate::types::Goal::Reverse]
            } else {
                &[crate::types::Goal::Forward]
            };
            for goal in goals {
                s.goals += 1;
                match model.memory(d.ledger.reviews(&card.front, *goal), clock) {
                    None => {}
                    Some(m) => {
                        s.learned += 1;
                        let ahead = days_between(today, model.due_day(&m)).max(0);
                        if ahead < FORECAST_DAYS {
                            s.forecast[ahead as usize] += 1;
                        }
                    }
                }
            }
        }
    }
    s.due = s.forecast[0];
    s.new_available = s.goals - s.learned;
    s
}

pub fn run(ctx: &Ctx, deck_args: &[String]) -> Result<i32> {
    let store = &ctx.store;
    store.require()?;
    let names = ctx.deck_names(deck_args)?;
    let decks = store.load_all(&names)?;
    ctx.report_warnings(&decks);
    let settings = ctx.settings()?;
    let model = ctx.model(&settings)?;
    let today = ctx.clock.today();
    let st = compute(&decks, &model, &ctx.clock);
    let summaries = summarize(&decks, &model, &ctx.clock, &settings, today);
    let new_eligible: usize = summaries.iter().map(|s| s.new_available).sum();
    let retention = if st.retention_30d.1 > 0 {
        Some(st.retention_30d.0 as f64 / st.retention_30d.1 as f64)
    } else {
        None
    };

    if ctx.json {
        out::json(&serde_json::json!({
            "decks": names,
            "cards": st.cards,
            "goals": st.goals,
            "learned": st.learned,
            "unseen": st.new_available,
            "new_eligible": new_eligible,
            "due": st.due,
            "reviews": { "total": st.reviews_total, "last_7d": st.reviews_7d, "last_30d": st.reviews_30d, "per_day_14d": st.per_day_14d, "days_with_reviews": st.review_days, "minutes_30d": st.minutes_30d },
            "retention_30d": retention,
            "retention_30d_sample": st.retention_30d.1,
            "forecast": st.forecast,
            "desired_retention": settings.desired_retention,
            "fsrs_parameters": { "source": if model.custom { "params.toml" } else { "default" }, "values": model.params },
        }));
        return Ok(0);
    }

    let style = ctx.term.out;
    let which = if deck_args.is_empty() {
        format!("all decks ({})", names.len())
    } else {
        names.join(", ")
    };
    out::println(&style.bold(&which));
    let both_ways = st.goals - st.cards;
    let sides = if both_ways > 0 {
        format!(" · {} prompts ({} asked both ways)", st.goals, both_ways)
    } else {
        String::new()
    };
    out::println(&format!(
        "  {}{sides} · {} learned · {} unseen ({} ready to start) · {} due today",
        plural(st.cards, "card"),
        st.learned,
        st.new_available,
        new_eligible,
        st.due
    ));
    out::println("");
    out::println(&format!(
        "  Reviews: {} total on {} · {} in the last 7 days · {} in the last 30 · {:.1}/day over 14",
        st.reviews_total,
        plural(st.review_days, "day"),
        st.reviews_7d,
        st.reviews_30d,
        st.per_day_14d
    ));
    match retention {
        Some(r) => out::println(&format!(
            "  Retention (30d): {:.0}% of {} across-day recalls · target {:.0}% · {:.0} min studied",
            r * 100.0,
            st.retention_30d.1,
            settings.desired_retention * 100.0,
            st.minutes_30d
        )),
        None => out::println(&format!(
            "  Retention (30d): no across-day recalls yet · target {:.0}%",
            settings.desired_retention * 100.0
        )),
    }
    out::println("");
    out::println(&format!("  Due in the next {FORECAST_DAYS} days:"));
    let labels: Vec<String> = (0..FORECAST_DAYS)
        .map(|d| match d {
            0 => "today".into(),
            1 => "tmrw".into(),
            n => format!("+{n}"),
        })
        .collect();
    let row1: Vec<String> = labels.iter().map(|l| pad_left(l, 5)).collect();
    let row2: Vec<String> = st
        .forecast
        .iter()
        .map(|n| pad_left(&n.to_string(), 5))
        .collect();
    out::println(&format!("  {}", style.dim(&row1.join(""))));
    out::println(&format!("  {}", row2.join("")));
    out::println("");
    out::println(&style.dim(&format!(
        "  FSRS parameters: {} · desired retention {:.2}",
        if model.custom {
            "params.toml"
        } else {
            "default"
        },
        settings.desired_retention
    )));
    Ok(0)
}
