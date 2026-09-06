//! `reword stats [DECK...]`: retention, pace, and a 14-day forecast.

use std::collections::BTreeSet;

use super::{Ctx, leech_json, leeches, progress_bar, summarize};
use crate::clock::{Clock, days_between};
use crate::error::Result;
use crate::memory::Model;
use crate::out;
use crate::store::LoadedDeck;
use crate::term::Style;
use crate::text::{self, pad_right, plural};
use crate::viz;

const FORECAST_DAYS: i32 = 14;
const ACTIVITY_DAYS: usize = 30;

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
    /// Reviews per study day over the last 30 days, oldest first.
    activity: Vec<usize>,
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
        activity: vec![0; ACTIVITY_DAYS],
    };
    let mut reviews_14d = 0usize;
    let mut days = BTreeSet::new();
    for d in decks {
        for (_, _, evs) in d.ledger.iter() {
            let mut prev_day = None;
            for ev in evs {
                s.reviews_total += 1;
                let age = now.duration_since(ev.ts).as_secs();
                let day = clock.study_day(ev.ts);
                days.insert(day);
                let back = days_between(day, today);
                if (0..ACTIVITY_DAYS as i32).contains(&back) {
                    s.activity[ACTIVITY_DAYS - 1 - back as usize] += 1;
                }
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
    let worst = leeches(&decks);
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
            "leeches": leech_json(&worst),
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
    let label = |l: &str| format!(" {}", pad_right(l, 11));
    let bar_w = ctx.term.width().saturating_sub(48).clamp(10, 24);
    let both_ways = st.goals - st.cards;

    out::println(&style.bold(&which));
    out::println("");

    let mut cards = vec![plural(st.cards, "card")];
    if both_ways > 0 {
        cards.push(format!("{} prompts", st.goals));
        cards.push(format!("{both_ways} asked both ways"));
    }
    out::println(&format!("{}{}", label("Cards"), cards.join(" · ")));
    out::println(&format!(
        "{}{}   {} · {} · {}",
        label(""),
        progress_bar(style, st.goals, st.learned, st.due, bar_w),
        style.green(&format!("{} learned", st.learned)),
        if st.due > 0 {
            style.yellow(&format!("{} due", st.due))
        } else {
            style.dim("0 due")
        },
        style.dim(&format!("{} unseen", st.new_available)),
    ));
    out::println("");

    let target = settings.desired_retention as f64;
    match retention {
        Some(r) => {
            let paint: viz::Paint = if r >= target {
                Style::green
            } else {
                Style::yellow
            };
            let filled = (r * 20.0).round() as usize;
            out::println(&format!(
                "{}{}  {}{}   {}",
                label("Retention"),
                style.bold(&format!("{:>3.0}%", r * 100.0)),
                paint(style, &"█".repeat(filled)),
                style.dim(&"░".repeat(20 - filled)),
                style.dim(&format!(
                    "{} recalls in 30 days · target {:.0}%",
                    st.retention_30d.1,
                    target * 100.0
                ))
            ));
        }
        None => out::println(&format!(
            "{}{}",
            label("Retention"),
            style.dim(&format!(
                "no across-day recalls yet · target {:.0}%",
                target * 100.0
            ))
        )),
    }
    out::println(&format!(
        "{}{} total · {} this week · {}/day · {} min studied",
        label("Reviews"),
        style.bold(&st.reviews_total.to_string()),
        style.bold(&st.reviews_7d.to_string()),
        style.bold(&format!("{:.1}", st.per_day_14d)),
        style.bold(&format!("{:.0}", st.minutes_30d)),
    ));
    let active = st.activity.iter().filter(|n| **n > 0).count();
    out::println(&format!(
        "{}{}   {}",
        label("Activity"),
        viz::strip(style, &st.activity),
        style.dim(&format!("{active} of the last {ACTIVITY_DAYS} days"))
    ));
    out::println("");

    if !worst.is_empty() {
        let multi = decks.len() > 1;
        for (i, l) in worst.iter().enumerate() {
            let name = if multi {
                format!("{} {}", l.deck, l.prompt)
            } else {
                l.prompt.clone()
            };
            let head = if i == 0 { "Leeches" } else { "" };
            out::println(&format!(
                "{}{} · {} again in {}",
                label(head),
                text::truncate(&name, 28),
                l.again,
                l.reviews,
            ));
        }
        out::println("");
    }

    out::println(&format!(
        "{}{}",
        label("Due ahead"),
        style.dim("next 14 days")
    ));
    let labels: Vec<String> = (0..FORECAST_DAYS)
        .map(|d| match d {
            0 => "td".into(),
            n => format!("+{n}"),
        })
        .collect();
    for line in viz::columns(style, &st.forecast, &labels, 4, "     ", |i| {
        if i == 0 { Style::yellow } else { Style::cyan }
    }) {
        out::println(&line);
    }
    out::println("");
    out::println(&style.dim(&format!(
        " FSRS {} · desired retention {:.2}",
        if model.custom {
            "params.toml"
        } else {
            "default"
        },
        settings.desired_retention
    )));
    Ok(0)
}
