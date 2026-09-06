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
use crate::types::{Goal, Grade};
use crate::viz;

/// Again grades before a card is called a leech.
const LEECH_AGAIN: usize = 3;
/// How many leeches status and stats show.
const LEECH_SHOW: usize = 5;

pub struct Leech {
    pub deck: String,
    pub prompt: String,
    pub again: usize,
    pub reviews: usize,
}

/// The worst cards still in the decks: most Again grades, at least
/// `LEECH_AGAIN`. Orphans are ignored.
pub fn leeches(decks: &[LoadedDeck]) -> Vec<Leech> {
    let mut out = Vec::new();
    for d in decks {
        for card in &d.deck.cards {
            let goals: &[Goal] = if card.reverse {
                &[Goal::Forward, Goal::Reverse]
            } else {
                &[Goal::Forward]
            };
            for goal in goals {
                let evs = d.ledger.reviews(&card.front, *goal);
                let reviews = evs.len();
                let again = evs.iter().filter(|e| e.grade == Grade::Again).count();
                if again >= LEECH_AGAIN {
                    out.push(Leech {
                        deck: d.name().to_string(),
                        prompt: card.prompt(*goal).to_string(),
                        again,
                        reviews,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| {
        b.again
            .cmp(&a.again)
            .then((b.again * a.reviews.max(1)).cmp(&(a.again * b.reviews.max(1))))
            .then(a.prompt.cmp(&b.prompt))
    });
    out.truncate(LEECH_SHOW);
    out
}

pub fn leech_json(leeches: &[Leech]) -> Vec<serde_json::Value> {
    leeches
        .iter()
        .map(|l| {
            serde_json::json!({
                "deck": l.deck,
                "prompt": l.prompt,
                "again": l.again,
                "reviews": l.reviews,
            })
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_ts;
    use crate::deck::Deck;
    use crate::history::{Ledger, Row, RowKind};
    use crate::types::{Grade, Mode};

    fn row(t: &str, front: &str, grade: Grade) -> Row {
        Row {
            ts: parse_ts(t).unwrap(),
            front: front.into(),
            kind: RowKind::Review {
                goal: Goal::Forward,
                mode: Mode::Recall,
                grade,
                elapsed_ms: 1000,
                answer: None,
                overridden: false,
            },
        }
    }

    fn loaded(text: &str, rows: Vec<Row>) -> LoadedDeck {
        LoadedDeck {
            deck: Deck::parse("t", "decks/t.md", text),
            ledger: Ledger::replay(&rows),
            warnings: vec![],
        }
    }

    #[test]
    fn leeches_are_the_worst_cards_still_in_the_deck() {
        let mut rows = Vec::new();
        for i in 0..8 {
            rows.push(row(
                &format!("2026-01-0{}T10:00:00Z", i + 1),
                "worse",
                Grade::Again,
            ));
        }
        for i in 0..4 {
            rows.push(row(
                &format!("2026-02-0{}T10:00:00Z", i + 1),
                "bad",
                Grade::Again,
            ));
        }
        rows.push(row("2026-03-01T10:00:00Z", "ok", Grade::Again));
        rows.push(row("2026-03-02T10:00:00Z", "ok", Grade::Good));
        rows.push(row("2026-03-03T10:00:00Z", "gone", Grade::Again));
        rows.push(row("2026-03-04T10:00:00Z", "gone", Grade::Again));
        rows.push(row("2026-03-05T10:00:00Z", "gone", Grade::Again));
        let decks = vec![loaded("worse::w\nbad::b\nok::o\n", rows)];
        let got: Vec<(String, usize)> = leeches(&decks)
            .into_iter()
            .map(|l| (l.prompt, l.again))
            .collect();
        assert_eq!(got, [("worse".into(), 8), ("bad".into(), 4)]);
    }
}
