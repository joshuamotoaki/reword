//! The memory model: FSRS-6 state derived from replayed history.

use fsrs::{
    ComputeParametersInput, FSRS, FSRSItem, FSRSReview, MemoryState, ModelEvaluation,
    check_and_fill_parameters, compute_parameters, current_retrievability,
};
use jiff::{Timestamp, civil::Date};

use crate::clock::{Clock, add_days, days_between};
use crate::error::{Error, Result};
use crate::history::{Ledger, ReviewEvent};

pub struct Model {
    fsrs: FSRS,
    decay: f32,
    pub params: Vec<f32>,
    pub custom: bool,
    pub desired_retention: f32,
}

/// What we know about one (card, goal) after replaying its reviews.
#[derive(Clone, Debug)]
pub struct CardMemory {
    pub state: MemoryState,
    /// Study day of the most recent review.
    pub last_day: Date,
}

impl Model {
    pub fn new(params: Option<&[f32]>, desired_retention: f32) -> Result<Model> {
        let raw: &[f32] = params.unwrap_or(&[]);
        let filled = check_and_fill_parameters(raw).map_err(|_| {
            Error::new(format!(
                "params.toml has {} parameters; FSRS expects 17, 19 or 21",
                raw.len()
            ))
            .hint("Delete params.toml to use the defaults, or run `reword optimize` again.")
        })?;
        let fsrs = FSRS::new(raw).map_err(|e| Error::from(e).hint("Check params.toml."))?;
        let decay = filled
            .get(20)
            .copied()
            .unwrap_or(fsrs::FSRS6_DEFAULT_DECAY)
            .clamp(0.1, 0.8);
        Ok(Model {
            fsrs,
            decay,
            params: filled,
            custom: !raw.is_empty(),
            desired_retention,
        })
    }

    /// Reviews → FSRS item. `delta_t` is whole study days since the previous
    /// review; 0 for the first review and for same-day repeats.
    pub fn item(events: &[ReviewEvent], clock: &Clock) -> FSRSItem {
        let mut reviews = Vec::with_capacity(events.len());
        let mut prev_day: Option<Date> = None;
        for ev in events {
            let day = clock.study_day(ev.ts);
            let delta_t = prev_day
                .map(|p| days_between(p, day).max(0) as u32)
                .unwrap_or(0);
            reviews.push(FSRSReview {
                rating: ev.grade.rating(),
                delta_t,
            });
            prev_day = Some(day);
        }
        FSRSItem { reviews }
    }

    pub fn memory(&self, events: &[ReviewEvent], clock: &Clock) -> Option<CardMemory> {
        let last = events.last()?;
        let item = Self::item(events, clock);
        let state = self.fsrs.memory_state(item, None).ok()?;
        Some(CardMemory {
            state,
            last_day: clock.study_day(last.ts),
        })
    }

    pub fn retrievability(&self, m: &CardMemory, today: Date) -> f32 {
        let days = days_between(m.last_day, today).max(0) as f32;
        current_retrievability(m.state, days, self.decay)
    }

    /// Whole days after the last review at which the card comes due, given
    /// the current desired retention. Never less than one day: same-day
    /// repeats are the session's job.
    pub fn scheduled_days(&self, m: &CardMemory) -> i32 {
        let interval = self
            .fsrs
            .next_interval(Some(m.state.stability), self.desired_retention, 3);
        interval.round().max(1.0) as i32
    }

    pub fn due_day(&self, m: &CardMemory) -> Date {
        add_days(m.last_day, self.scheduled_days(m))
    }

    pub fn is_due(&self, m: &CardMemory, today: Date) -> bool {
        today >= self.due_day(m)
    }

    /// Training items in the shape the optimizer wants: one item per review
    /// after the first, each carrying the full prefix, keeping only items
    /// whose current review is a long-term one (delta_t > 0).
    pub fn training_items(ledgers: &[&Ledger], clock: &Clock) -> Vec<FSRSItem> {
        let mut stamped: Vec<(Timestamp, FSRSItem)> = Vec::new();
        for ledger in ledgers {
            for (_, _, events) in ledger.iter() {
                let full = Self::item(events, clock);
                for (idx, review) in full.reviews.iter().enumerate().skip(1) {
                    if review.delta_t == 0 {
                        continue;
                    }
                    stamped.push((
                        events[idx].ts,
                        FSRSItem {
                            reviews: full.reviews[..=idx].to_vec(),
                        },
                    ));
                }
            }
        }
        stamped.sort_by_key(|(ts, _)| *ts);
        stamped.into_iter().map(|(_, item)| item).collect()
    }

    pub fn evaluate(&self, items: Vec<FSRSItem>) -> Result<ModelEvaluation> {
        Ok(self.fsrs.evaluate(items, |_| true)?)
    }

    pub fn optimize(items: Vec<FSRSItem>) -> Result<Vec<f32>> {
        Ok(compute_parameters(ComputeParametersInput {
            train_set: items,
            ..Default::default()
        })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_ts;
    use crate::types::{Grade, Mode};

    fn ev(t: &str, grade: Grade) -> ReviewEvent {
        ReviewEvent {
            ts: parse_ts(t).unwrap(),
            grade,
            mode: Mode::Recall,
            elapsed_ms: 1000,
        }
    }

    #[test]
    fn new_card_has_no_memory_and_grades_order_intervals() {
        let model = Model::new(None, 0.9).unwrap();
        let clock = Clock::utc();
        assert!(model.memory(&[], &clock).is_none());
        let days = |g: Grade| {
            model.scheduled_days(
                &model
                    .memory(&[ev("2026-09-06T10:00:00Z", g)], &clock)
                    .unwrap(),
            )
        };
        assert!(days(Grade::Again) >= 1);
        assert!(days(Grade::Again) <= days(Grade::Good));
        assert!(days(Grade::Good) < days(Grade::Easy));
    }

    #[test]
    fn same_day_repeat_uses_zero_delta_and_history_grows_stability() {
        let model = Model::new(None, 0.9).unwrap();
        let clock = Clock::utc();
        let events = vec![
            ev("2026-09-01T10:00:00Z", Grade::Again),
            ev("2026-09-01T10:05:00Z", Grade::Good),
            ev("2026-09-03T10:00:00Z", Grade::Good),
        ];
        let item = Model::item(&events, &clock);
        assert_eq!(
            item.reviews.iter().map(|r| r.delta_t).collect::<Vec<_>>(),
            vec![0, 0, 2]
        );
        let m = model.memory(&events, &clock).unwrap();
        let m2 = model.memory(&events[..2], &clock).unwrap();
        assert!(m.state.stability > m2.state.stability);
        let today: Date = "2026-09-03".parse().unwrap();
        assert!(!model.is_due(&m, today), "just reviewed today");
        assert!((model.retrievability(&m, today) - 1.0).abs() < 1e-5);
        let far: Date = "2027-09-03".parse().unwrap();
        assert!(model.is_due(&m, far));
        assert!(model.retrievability(&m, far) < 0.9);
    }

    #[test]
    fn training_items_keep_long_term_targets_only() {
        let clock = Clock::utc();
        let rows = vec![
            crate::history::Row {
                ts: parse_ts("2026-09-01T10:00:00Z").unwrap(),
                front: "a".into(),
                kind: crate::history::RowKind::Review {
                    goal: crate::types::Goal::Forward,
                    mode: Mode::Recall,
                    grade: Grade::Good,
                    elapsed_ms: 1,
                    answer: None,
                    overridden: false,
                },
            },
            crate::history::Row {
                ts: parse_ts("2026-09-01T10:01:00Z").unwrap(),
                front: "a".into(),
                kind: crate::history::RowKind::Review {
                    goal: crate::types::Goal::Forward,
                    mode: Mode::Recall,
                    grade: Grade::Good,
                    elapsed_ms: 1,
                    answer: None,
                    overridden: false,
                },
            },
            crate::history::Row {
                ts: parse_ts("2026-09-04T10:00:00Z").unwrap(),
                front: "a".into(),
                kind: crate::history::RowKind::Review {
                    goal: crate::types::Goal::Forward,
                    mode: Mode::Recall,
                    grade: Grade::Good,
                    elapsed_ms: 1,
                    answer: None,
                    overridden: false,
                },
            },
        ];
        let ledger = Ledger::replay(&rows);
        let items = Model::training_items(&[&ledger], &clock);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].reviews.len(), 3);
    }

    #[test]
    fn bad_params_are_reported() {
        assert!(Model::new(Some(&[1.0, 2.0]), 0.9).is_err());
    }
}
