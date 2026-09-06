//! One module per subcommand, plus what they share.

pub mod add;
pub mod check;
pub mod completions;
pub mod decks;
pub mod edit;
pub mod init;
pub mod optimize;
pub mod rename;
pub mod review;
pub mod stats;
pub mod status;

use jiff::{Timestamp, civil::Date};

use crate::clock::Clock;
use crate::config::Settings;
use crate::error::Result;
use crate::memory::Model;
use crate::out;
use crate::planner::{self, NewPolicy};
use crate::store::{LoadedDeck, Store};
use crate::term::{Style, Term};
use crate::text;
use crate::viz;

pub struct Ctx {
    pub store: Store,
    pub term: Term,
    pub clock: Clock,
    pub json: bool,
}

impl Ctx {
    pub fn settings(&self) -> Result<Settings> {
        Settings::load(&self.store)
    }

    pub fn model(&self, settings: &Settings) -> Result<Model> {
        Model::new(settings.params.as_deref(), settings.desired_retention)
    }

    /// Print a deck's parse and log warnings to stderr.
    pub fn report_warnings(&self, decks: &[LoadedDeck]) {
        for d in decks {
            for w in &d.warnings {
                out::warn(&self.term, &w.to_string());
            }
        }
    }

    /// Resolve deck arguments, or every deck when none are given.
    pub fn deck_names(&self, args: &[String]) -> Result<Vec<String>> {
        if args.is_empty() {
            return self.store.list_decks();
        }
        let mut names = Vec::new();
        for a in args {
            let name = self.store.resolve_deck(a)?;
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Ok(names)
    }
}

/// Per-deck numbers shared by `status`, `decks` and `stats`.
/// Learned in green, due in yellow, unseen dim: the same bar everywhere.
pub fn progress_bar(
    style: Style,
    goals: usize,
    learned: usize,
    due: usize,
    cells: usize,
) -> String {
    viz::bar(
        style,
        &[
            viz::Segment {
                n: learned.saturating_sub(due),
                ch: '█',
                paint: Style::green,
            },
            viz::Segment {
                n: due,
                ch: '▓',
                paint: Style::yellow,
            },
        ],
        goals,
        cells,
    )
}

/// A right-aligned count, colored only when nonzero.
pub fn count(style: Style, n: usize, cells: usize, paint: viz::Paint) -> String {
    let s = text::pad_left(&n.to_string(), cells);
    if n == 0 {
        style.dim(&s)
    } else {
        paint(style, &s)
    }
}

pub struct DeckSummary {
    pub name: String,
    pub cards: usize,
    pub goals: usize,
    pub learned: usize,
    pub due: usize,
    pub new_available: usize,
    pub last_review: Option<Timestamp>,
    pub next_due: Option<(Date, usize)>,
}

pub fn summarize(
    decks: &[LoadedDeck],
    model: &Model,
    clock: &Clock,
    settings: &Settings,
    today: Date,
) -> Vec<DeckSummary> {
    decks
        .iter()
        .map(|d| {
            let plan = planner::plan(
                std::slice::from_ref(d),
                model,
                clock,
                today,
                settings,
                NewPolicy::Explicit(usize::MAX),
                0,
                false,
            );
            DeckSummary {
                name: d.name().to_string(),
                cards: d.deck.cards.len(),
                goals: plan.total_goals,
                learned: plan.learned,
                due: plan.due.len(),
                new_available: plan.new.len(),
                last_review: d.ledger.last_activity,
                next_due: plan.next_due,
            }
        })
        .collect()
}

pub fn summary_json(s: &DeckSummary, now: Timestamp) -> serde_json::Value {
    serde_json::json!({
        "name": s.name,
        "cards": s.cards,
        "goals": s.goals,
        "learned": s.learned,
        "due": s.due,
        "new": s.new_available,
        "last_review": s.last_review.map(crate::clock::format_ts),
        "last_review_ago": s.last_review.map(|t| crate::clock::ago(t, now)),
        "next_due": s.next_due.map(|(d, n)| serde_json::json!({"day": d.to_string(), "cards": n})),
    })
}
