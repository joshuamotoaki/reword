//! The session planner: what is due, what is new, how much of each fits.

use jiff::civil::Date;

use crate::clock::Clock;
use crate::config::Settings;
use crate::memory::Model;
use crate::store::LoadedDeck;
use crate::types::Goal;

#[derive(Clone, Debug)]
pub struct Item {
    pub deck: usize,
    pub card: usize,
    pub goal: Goal,
    pub retrievability: f32,
}

#[derive(Debug)]
pub struct Plan {
    /// Due now, lowest retrievability first.
    pub due: Vec<Item>,
    /// Eligible new cards in deck and line order.
    pub new: Vec<Item>,
    /// How many of `new` this session may introduce.
    pub new_limit: usize,
    /// New intake was set to zero because the overdue backlog is large.
    pub throttled: bool,
    /// Earliest future due day and how many cards come due that day.
    pub next_due: Option<(Date, usize)>,
    /// Siblings held back because the other side is in this session.
    pub deferred_siblings: usize,
    pub total_goals: usize,
    pub learned: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum NewPolicy {
    /// `new_per_day` minus what was introduced today, throttled by backlog.
    Daily,
    /// `--new N` / `--no-new`: exactly this many, no throttle.
    Explicit(usize),
}

impl Plan {
    pub fn new_waiting(&self) -> usize {
        self.new.len().saturating_sub(self.new_limit)
    }

    pub fn is_empty(&self) -> bool {
        self.due.is_empty() && self.new_limit == 0
    }
}

/// Keys whose first surviving review happened today, across the given decks.
pub fn introduced_today(decks: &[LoadedDeck], clock: &Clock, today: Date) -> usize {
    decks
        .iter()
        .flat_map(|d| d.ledger.iter())
        .filter(|(_, _, evs)| clock.study_day(evs[0].ts) == today)
        .count()
}

/// Mean review time in ms over all history, clamped to something sane.
fn average_review_ms(decks: &[LoadedDeck]) -> u64 {
    let mut total = 0u128;
    let mut n = 0u64;
    for d in decks {
        for (_, _, evs) in d.ledger.iter() {
            for ev in evs {
                if ev.elapsed_ms > 0 {
                    total += u128::from(ev.elapsed_ms);
                    n += 1;
                }
            }
        }
    }
    total
        .checked_div(u128::from(n))
        .map_or(10_000, |avg| avg.clamp(2_000, 60_000) as u64)
}

enum Candidate {
    Due(Item),
    New(Item),
}

pub fn plan(
    decks: &[LoadedDeck],
    model: &Model,
    clock: &Clock,
    today: Date,
    settings: &Settings,
    policy: NewPolicy,
    introduced_today_all: usize,
) -> Plan {
    let mut due = Vec::new();
    let mut new = Vec::new();
    let mut next_due: Option<(Date, usize)> = None;
    let mut deferred = 0;
    let mut total_goals = 0;
    let mut learned = 0;

    for (di, ld) in decks.iter().enumerate() {
        for (ci, card) in ld.deck.cards.iter().enumerate() {
            let mut classify = |goal: Goal, eligible_new: bool| -> Option<Candidate> {
                total_goals += 1;
                let memory = model.memory(ld.ledger.reviews(&card.front, goal), clock);
                match memory {
                    None => eligible_new.then_some(Candidate::New(Item {
                        deck: di,
                        card: ci,
                        goal,
                        retrievability: 0.0,
                    })),
                    Some(m) => {
                        learned += 1;
                        if model.is_due(&m, today) {
                            let r = model.retrievability(&m, today);
                            Some(Candidate::Due(Item {
                                deck: di,
                                card: ci,
                                goal,
                                retrievability: r,
                            }))
                        } else {
                            let day = model.due_day(&m);
                            next_due = Some(match next_due {
                                Some((d, n)) if d == day => (d, n + 1),
                                Some((d, n)) if d < day => (d, n),
                                _ => (day, 1),
                            });
                            None
                        }
                    }
                }
            };

            let forward = classify(Goal::Forward, true);
            let reverse = if card.reverse {
                let forward_ok = ld
                    .ledger
                    .reviews(&card.front, Goal::Forward)
                    .iter()
                    .any(|e| e.grade.is_success());
                classify(Goal::Reverse, forward_ok)
            } else {
                None
            };

            // Never both sides of one line in a session: keep the more urgent.
            let (keep, defer) = match (forward, reverse) {
                (Some(f), Some(r)) => match (&f, &r) {
                    (Candidate::Due(a), Candidate::Due(b)) => {
                        if a.retrievability <= b.retrievability {
                            (Some(f), Some(r))
                        } else {
                            (Some(r), Some(f))
                        }
                    }
                    (Candidate::Due(_), Candidate::New(_)) => (Some(f), Some(r)),
                    (Candidate::New(_), Candidate::Due(_)) => (Some(r), Some(f)),
                    (Candidate::New(_), Candidate::New(_)) => (Some(f), Some(r)),
                },
                (f, r) => (f.or(r), None),
            };
            if defer.is_some() {
                deferred += 1;
            }
            match keep {
                Some(Candidate::Due(item)) => due.push(item),
                Some(Candidate::New(item)) => new.push(item),
                None => {}
            }
        }
    }

    due.sort_by(|a, b| {
        a.retrievability
            .partial_cmp(&b.retrievability)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut throttled = false;
    let new_limit = match policy {
        NewPolicy::Explicit(n) => n,
        NewPolicy::Daily => {
            let base = (settings.new_per_day as usize).saturating_sub(introduced_today_all);
            let per_session =
                (settings.session_minutes as u64 * 60_000 / average_review_ms(decks)) as usize;
            if base > 0 && due.len() > 2 * per_session.max(1) {
                throttled = true;
                0
            } else {
                base
            }
        }
    }
    .min(new.len());

    Plan {
        due,
        new,
        new_limit,
        throttled,
        next_due,
        deferred_siblings: deferred,
        total_goals,
        learned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_ts;
    use crate::deck::Deck;
    use crate::history::{Ledger, Row, RowKind};
    use crate::types::{Grade, Mode};

    fn row(t: &str, front: &str, goal: Goal, grade: Grade) -> Row {
        Row {
            ts: parse_ts(t).unwrap(),
            front: front.into(),
            kind: RowKind::Review {
                goal,
                mode: Mode::Recall,
                grade,
                elapsed_ms: 5000,
                answer: None,
                overridden: false,
            },
        }
    }

    fn loaded(text: &str, rows: Vec<Row>) -> LoadedDeck {
        let deck = Deck::parse("t", "decks/t.md", text);
        LoadedDeck {
            deck,
            ledger: Ledger::replay(&rows),
            warnings: vec![],
        }
    }

    #[test]
    fn new_cards_reverse_gating_and_limits() {
        let clock = Clock::utc();
        let model = Model::new(None, 0.9).unwrap();
        let settings = Settings::default();
        let today: Date = "2026-09-06".parse().unwrap();
        // a: brand new. b: forward learned long ago (due), reverse never seen.
        let decks = vec![loaded(
            "a:::1\nb:::2\n",
            vec![row("2026-01-01T10:00:00Z", "b", Goal::Forward, Grade::Good)],
        )];
        let p = plan(
            &decks,
            &model,
            &clock,
            today,
            &settings,
            NewPolicy::Daily,
            0,
        );
        assert_eq!(p.due.len(), 1);
        assert_eq!(p.due[0].goal, Goal::Forward);
        // a forward is new; a reverse is not eligible (forward unseen);
        // b reverse is eligible but deferred because b forward is in session.
        assert_eq!(p.new.len(), 1);
        assert_eq!(p.new[0].goal, Goal::Forward);
        assert_eq!(p.deferred_siblings, 1);
        assert_eq!(p.new_limit, 1);
        assert_eq!(p.total_goals, 4);
        assert_eq!(p.learned, 1);

        let p = plan(
            &decks,
            &model,
            &clock,
            today,
            &settings,
            NewPolicy::Explicit(0),
            0,
        );
        assert_eq!(p.new_limit, 0);
        assert_eq!(p.new_waiting(), 1);

        let p = plan(
            &decks,
            &model,
            &clock,
            today,
            &settings,
            NewPolicy::Daily,
            10,
        );
        assert_eq!(p.new_limit, 0, "daily cap reached");
    }

    #[test]
    fn due_sorted_by_retrievability_and_next_due_reported() {
        let clock = Clock::utc();
        let model = Model::new(None, 0.9).unwrap();
        let settings = Settings::default();
        let today: Date = "2026-09-06".parse().unwrap();
        let decks = vec![loaded(
            "old::1\nolder::2\nfresh::3\n",
            vec![
                row("2026-08-01T10:00:00Z", "old", Goal::Forward, Grade::Good),
                row("2026-06-01T10:00:00Z", "older", Goal::Forward, Grade::Good),
                row("2026-09-06T10:00:00Z", "fresh", Goal::Forward, Grade::Good),
            ],
        )];
        let p = plan(
            &decks,
            &model,
            &clock,
            today,
            &settings,
            NewPolicy::Daily,
            0,
        );
        assert_eq!(p.due.len(), 2);
        assert_eq!(decks[0].deck.cards[p.due[0].card].front, "older");
        assert!(p.next_due.is_some());
        assert_eq!(introduced_today(&decks, &clock, today), 1);
    }

    #[test]
    fn backlog_throttles_new_intake() {
        let clock = Clock::utc();
        let model = Model::new(None, 0.9).unwrap();
        let settings = Settings {
            session_minutes: 1,
            ..Settings::default()
        };
        let today: Date = "2026-09-06".parse().unwrap();
        let mut text = String::from("brandnew::x\n");
        let mut rows = Vec::new();
        for i in 0..40 {
            text.push_str(&format!("c{i}::y\n"));
            rows.push(row(
                "2026-01-01T10:00:00Z",
                &format!("c{i}"),
                Goal::Forward,
                Grade::Good,
            ));
        }
        let decks = vec![loaded(&text, rows)];
        let p = plan(
            &decks,
            &model,
            &clock,
            today,
            &settings,
            NewPolicy::Daily,
            0,
        );
        assert!(p.throttled);
        assert_eq!(p.new_limit, 0);
        let p = plan(
            &decks,
            &model,
            &clock,
            today,
            &settings,
            NewPolicy::Explicit(1),
            0,
        );
        assert!(!p.throttled);
        assert_eq!(p.new_limit, 1);
    }

    #[test]
    fn average_duration_handles_large_log_values() {
        let mut rows = vec![
            row("2026-01-01T10:00:00Z", "a", Goal::Forward, Grade::Good),
            row("2026-01-02T10:00:00Z", "a", Goal::Forward, Grade::Good),
        ];
        for row in &mut rows {
            if let RowKind::Review { elapsed_ms, .. } = &mut row.kind {
                *elapsed_ms = u64::MAX;
            }
        }
        assert_eq!(average_review_ms(&[loaded("a::answer\n", rows)]), 60_000);
    }
}
