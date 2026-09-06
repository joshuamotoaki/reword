//! The review loop: one mode per session, every grade appended immediately,
//! undo as an appended row, time-bounded with an optional continue.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::clock::{Clock, interval_label};
use crate::error::Result;
use crate::history::{ReviewEvent, Row, RowKind};
use crate::memory::Model;
use crate::planner::{Item, Plan};
use crate::store::{LoadedDeck, Store};
use crate::term::{self, Key, Term};
use crate::text::{answer_matches, plural};
use crate::types::{Goal, Grade, Mode};

/// A card graded Again comes back after this many other cards.
const RELEARN_GAP: usize = 5;
/// One new card per this many reviews while due cards remain.
const NEW_EVERY: usize = 5;

pub struct Options {
    pub mode: Mode,
    pub minutes: u32,
}

#[derive(Default, Debug)]
pub struct Summary {
    pub reviews: usize,
    pub correct: usize,
    pub skipped: usize,
    pub new_seen: usize,
    pub secs: u64,
    pub remaining: usize,
    pub quit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Due,
    New,
    Relearn(usize),
}

#[derive(Clone, Debug)]
enum Stage {
    Fresh,
    Revealed,
    Checked { typed: String, correct: bool },
}

enum Step {
    Graded {
        grade: Grade,
        answer: Option<String>,
        overridden: bool,
        stage: Stage,
    },
    Skipped,
    Quit,
    Undo,
}

struct Queue {
    due: VecDeque<Item>,
    new: VecDeque<Item>,
    relearn: VecDeque<(Item, usize)>,
    new_limit: usize,
    new_taken: usize,
    shown: usize,
    since_new: usize,
}

type Counters = (usize, usize, usize);

fn same(a: &Item, b: &Item) -> bool {
    a.deck == b.deck && a.card == b.card && a.goal == b.goal
}

impl Queue {
    fn new_allowance(&self) -> usize {
        self.new_limit
            .saturating_sub(self.new_taken)
            .min(self.new.len())
    }

    fn remaining(&self) -> usize {
        self.due.len() + self.relearn.len() + self.new_allowance()
    }

    fn pick(&mut self) -> Option<(Item, Origin)> {
        if let Some(pos) = self
            .relearn
            .iter()
            .position(|(_, ready)| *ready <= self.shown)
        {
            let (item, ready) = self.relearn.remove(pos).unwrap();
            return Some((item, Origin::Relearn(ready)));
        }
        let new_ok = self.new_allowance() > 0;
        if new_ok && (self.due.is_empty() || self.since_new >= NEW_EVERY) {
            return self.new.pop_front().map(|i| (i, Origin::New));
        }
        if let Some(item) = self.due.pop_front() {
            return Some((item, Origin::Due));
        }
        if let Some((item, ready)) = self.relearn.pop_front() {
            return Some((item, Origin::Relearn(ready)));
        }
        None
    }

    fn account(&mut self, origin: Origin) {
        self.shown += 1;
        match origin {
            Origin::New => {
                self.new_taken += 1;
                self.since_new = 0;
            }
            _ => self.since_new += 1,
        }
    }

    fn counters(&self) -> Counters {
        (self.shown, self.new_taken, self.since_new)
    }

    fn restore(&mut self, c: Counters) {
        (self.shown, self.new_taken, self.since_new) = c;
    }

    fn push_front(&mut self, item: Item, origin: Origin) {
        match origin {
            Origin::Due => self.due.push_front(item),
            Origin::New => self.new.push_front(item),
            Origin::Relearn(ready) => self.relearn.push_front((item, ready)),
        }
    }

    fn requeue(&mut self, item: Item) {
        self.relearn.push_back((item, self.shown + RELEARN_GAP));
    }

    fn withdraw(&mut self, item: &Item) {
        if let Some(pos) = self.relearn.iter().position(|(i, _)| same(i, item)) {
            self.relearn.remove(pos);
        }
    }
}

struct Done {
    item: Item,
    origin: Origin,
    stage: Stage,
    requeued: bool,
    counters: Counters,
    graded: Option<Grade>,
}

struct Session<'a> {
    store: &'a Store,
    decks: &'a [LoadedDeck],
    model: &'a Model,
    clock: &'a Clock,
    term: &'a Term,
    mode: Mode,
    minutes: u32,
    multi: bool,
    queue: Queue,
    pool: usize,
    start: Instant,
    budget: Duration,
    done: Vec<Done>,
    session_events: HashMap<(usize, String, Goal), Vec<ReviewEvent>>,
    summary: Summary,
}

pub fn run(
    store: &Store,
    decks: &[LoadedDeck],
    plan: Plan,
    model: &Model,
    clock: &Clock,
    term: &Term,
    opts: Options,
) -> Result<Summary> {
    let queue = Queue {
        due: plan.due.into(),
        new: plan.new.into(),
        relearn: VecDeque::new(),
        new_limit: plan.new_limit,
        new_taken: 0,
        shown: 0,
        since_new: 0,
    };
    let pool = queue.remaining();
    let mut s = Session {
        store,
        decks,
        model,
        clock,
        term,
        mode: opts.mode,
        minutes: opts.minutes,
        multi: decks.len() > 1,
        queue,
        pool,
        start: Instant::now(),
        budget: Duration::from_secs(u64::from(opts.minutes) * 60),
        done: Vec::new(),
        session_events: HashMap::new(),
        summary: Summary::default(),
    };

    if s.mode == Mode::Typed {
        println!(
            "{}",
            term.out
                .dim("Type the answer and press enter. An empty line reveals it.")
        );
    }

    let mut pending: Option<(Item, Origin, Stage)> = None;
    loop {
        let (item, origin, stage) = match pending.take() {
            Some(p) => p,
            None => {
                if s.time_up()? {
                    break;
                }
                match s.queue.pick() {
                    Some((item, origin)) => (item, origin, Stage::Fresh),
                    None => break,
                }
            }
        };
        let counters = s.queue.counters();
        s.queue.account(origin);
        let can_undo = !s.done.is_empty();
        let started = Instant::now();
        println!();
        let step = s.present(&item, &stage, can_undo)?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match step {
            Step::Graded {
                grade,
                answer,
                overridden,
                stage,
            } => {
                s.record(&item, grade, elapsed_ms, answer, overridden)?;
                let requeued = grade == Grade::Again;
                if requeued {
                    s.queue.requeue(item.clone());
                }
                if origin == Origin::New {
                    s.summary.new_seen += 1;
                }
                s.print_result(&item, grade);
                s.done.push(Done {
                    item,
                    origin,
                    stage,
                    requeued,
                    counters,
                    graded: Some(grade),
                });
            }
            Step::Skipped => {
                s.skip(&item, elapsed_ms)?;
                s.print_skip();
                s.done.push(Done {
                    item,
                    origin,
                    stage: Stage::Fresh,
                    requeued: false,
                    counters,
                    graded: None,
                });
            }
            Step::Quit => {
                s.queue.push_front(item, origin);
                s.queue.restore(counters);
                s.summary.quit = true;
                break;
            }
            Step::Undo => {
                let d = s.done.pop().expect("undo only offered with history");
                s.undo(&d)?;
                s.queue.push_front(item, origin);
                if d.requeued {
                    s.queue.withdraw(&d.item);
                }
                if d.origin == Origin::New && d.graded.is_some() {
                    s.summary.new_seen -= 1;
                }
                s.queue.restore(d.counters);
                println!("{}", term.out.dim("↶ undo"));
                pending = Some((d.item, d.origin, d.stage));
            }
        }
    }
    s.finish()
}

impl Session<'_> {
    fn card(&self, item: &Item) -> &crate::deck::Card {
        &self.decks[item.deck].deck.cards[item.card]
    }

    fn key(&self, item: &Item) -> (usize, String, Goal) {
        (item.deck, self.card(item).front.clone(), item.goal)
    }

    fn events(&self, item: &Item) -> Vec<ReviewEvent> {
        let card = self.card(item);
        let mut evs = self.decks[item.deck]
            .ledger
            .reviews(&card.front, item.goal)
            .to_vec();
        if let Some(extra) = self.session_events.get(&self.key(item)) {
            evs.extend(extra.iter().cloned());
        }
        evs
    }

    fn print_prompt(&self, item: &Item) {
        let card = self.card(item);
        let tag = if self.multi {
            format!("   {}", self.term.out.dim(self.decks[item.deck].name()))
        } else {
            String::new()
        };
        println!("  {}{tag}", card.prompt(item.goal));
    }

    fn present(&self, item: &Item, stage: &Stage, can_undo: bool) -> Result<Step> {
        match self.mode {
            Mode::Recall => self.present_recall(item, stage, can_undo),
            Mode::Typed => self.present_typed(item, stage, can_undo),
        }
    }

    fn wait_key(&self, hint: &str, can_undo: bool) -> Result<Key> {
        let hint = if can_undo {
            hint.to_string()
        } else {
            hint.replace("  [←] undo", "")
        };
        term::show_hint(self.term, &hint);
        let key = term::read_key()?;
        term::clear_line();
        Ok(key)
    }

    fn present_recall(&self, item: &Item, stage: &Stage, can_undo: bool) -> Result<Step> {
        self.print_prompt(item);
        let answer = self.card(item).answer(item.goal);
        if matches!(stage, Stage::Fresh) {
            loop {
                match self.wait_key("[space] reveal  [s] skip  [←] undo  [q] quit", can_undo)? {
                    Key::Space | Key::Enter => break,
                    Key::Char('s') => return Ok(Step::Skipped),
                    Key::Char('q') | Key::Esc | Key::CtrlC | Key::CtrlD => return Ok(Step::Quit),
                    Key::Left | Key::Backspace | Key::Char('u') if can_undo => {
                        return Ok(Step::Undo);
                    }
                    _ => {}
                }
            }
        }
        println!("  {answer}");
        loop {
            let grade = match self
                .wait_key("[space] good  [a] again  [h] hard  [e] easy", can_undo)?
            {
                Key::Space | Key::Enter | Key::Char('g') | Key::Char('3') => Grade::Good,
                Key::Char('a') | Key::Char('1') => Grade::Again,
                Key::Char('h') | Key::Char('2') => Grade::Hard,
                Key::Char('e') | Key::Char('4') => Grade::Easy,
                Key::Char('s') => return Ok(Step::Skipped),
                Key::Char('q') | Key::Esc | Key::CtrlC | Key::CtrlD => return Ok(Step::Quit),
                Key::Left | Key::Backspace | Key::Char('u') if can_undo => return Ok(Step::Undo),
                Key::Char('?') => {
                    println!(
                        "{}",
                        self.term.out.dim(
                            "    also: [s] skip  [←] undo  [q] quit  [1-4] again/hard/good/easy"
                        )
                    );
                    continue;
                }
                _ => continue,
            };
            return Ok(Step::Graded {
                grade,
                answer: None,
                overridden: false,
                stage: Stage::Revealed,
            });
        }
    }

    fn present_typed(&self, item: &Item, stage: &Stage, can_undo: bool) -> Result<Step> {
        self.print_prompt(item);
        let expected = self.card(item).answer(item.goal);
        let fresh = matches!(stage, Stage::Fresh);
        let stage = match stage {
            Stage::Fresh => {
                let Some(line) = term::read_line("  > ")? else {
                    return Ok(Step::Quit);
                };
                if line.trim().is_empty() {
                    Stage::Revealed
                } else {
                    Stage::Checked {
                        correct: answer_matches(&line, expected),
                        typed: line,
                    }
                }
            }
            Stage::Checked { typed, correct } => {
                println!("  > {typed}");
                Stage::Checked {
                    typed: typed.clone(),
                    correct: *correct,
                }
            }
            Stage::Revealed => Stage::Revealed,
        };
        match &stage {
            Stage::Fresh => unreachable!(),
            Stage::Revealed => {
                println!("  {expected}");
                loop {
                    match self.wait_key("[enter] again  [s] skip  [←] undo  [q] quit", can_undo)?
                    {
                        Key::Enter | Key::Space | Key::Char('a') => {
                            return Ok(Step::Graded {
                                grade: Grade::Again,
                                answer: None,
                                overridden: false,
                                stage,
                            });
                        }
                        Key::Char('s') => return Ok(Step::Skipped),
                        Key::Char('q') | Key::Esc | Key::CtrlC | Key::CtrlD => {
                            return Ok(Step::Quit);
                        }
                        Key::Left | Key::Backspace | Key::Char('u') if can_undo => {
                            return Ok(Step::Undo);
                        }
                        _ => {}
                    }
                }
            }
            Stage::Checked {
                typed,
                correct: true,
            } => {
                if fresh {
                    return Ok(Step::Graded {
                        grade: Grade::Good,
                        answer: Some(typed.clone()),
                        overridden: false,
                        stage,
                    });
                }
                println!("{} {expected}", self.term.out.green("✓"));
                loop {
                    match self.wait_key(
                        "[enter] good  [a] again  [s] skip  [←] undo  [q] quit",
                        can_undo,
                    )? {
                        Key::Enter | Key::Space | Key::Char('g') => {
                            return Ok(Step::Graded {
                                grade: Grade::Good,
                                answer: Some(typed.clone()),
                                overridden: false,
                                stage,
                            });
                        }
                        Key::Char('a') => {
                            return Ok(Step::Graded {
                                grade: Grade::Again,
                                answer: Some(typed.clone()),
                                overridden: false,
                                stage,
                            });
                        }
                        Key::Char('s') => return Ok(Step::Skipped),
                        Key::Char('q') | Key::Esc | Key::CtrlC | Key::CtrlD => {
                            return Ok(Step::Quit);
                        }
                        Key::Left | Key::Backspace | Key::Char('u') if can_undo => {
                            return Ok(Step::Undo);
                        }
                        _ => {}
                    }
                }
            }
            Stage::Checked {
                typed,
                correct: false,
            } => {
                println!("{} {expected}", self.term.out.red("✗"));
                loop {
                    match self.wait_key(
                        "[enter] again  [o] typo, count it  [s] skip  [←] undo  [q] quit",
                        can_undo,
                    )? {
                        Key::Enter | Key::Space | Key::Char('a') => {
                            return Ok(Step::Graded {
                                grade: Grade::Again,
                                answer: Some(typed.clone()),
                                overridden: false,
                                stage,
                            });
                        }
                        Key::Char('o') => {
                            return Ok(Step::Graded {
                                grade: Grade::Good,
                                answer: Some(typed.clone()),
                                overridden: true,
                                stage,
                            });
                        }
                        Key::Char('s') => return Ok(Step::Skipped),
                        Key::Char('q') | Key::Esc | Key::CtrlC | Key::CtrlD => {
                            return Ok(Step::Quit);
                        }
                        Key::Left | Key::Backspace | Key::Char('u') if can_undo => {
                            return Ok(Step::Undo);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn record(
        &mut self,
        item: &Item,
        grade: Grade,
        elapsed_ms: u64,
        answer: Option<String>,
        overridden: bool,
    ) -> Result<()> {
        let ts = self.clock.now();
        let deck = &self.decks[item.deck];
        let front = self.card(item).front.clone();
        let row = Row {
            ts,
            front,
            kind: RowKind::Review {
                goal: item.goal,
                mode: self.mode,
                grade,
                elapsed_ms,
                answer,
                overridden,
            },
        };
        self.store.append_row(deck.name(), &row)?;
        self.session_events
            .entry(self.key(item))
            .or_default()
            .push(ReviewEvent {
                ts,
                grade,
                mode: self.mode,
                elapsed_ms,
            });
        self.summary.reviews += 1;
        if grade.is_success() {
            self.summary.correct += 1;
        }
        Ok(())
    }

    fn skip(&mut self, item: &Item, elapsed_ms: u64) -> Result<()> {
        let deck = &self.decks[item.deck];
        let row = Row {
            ts: self.clock.now(),
            front: self.card(item).front.clone(),
            kind: RowKind::Skip {
                goal: item.goal,
                mode: self.mode,
                elapsed_ms,
            },
        };
        self.store.append_row(deck.name(), &row)?;
        self.summary.skipped += 1;
        Ok(())
    }

    fn undo(&mut self, d: &Done) -> Result<()> {
        let deck = &self.decks[d.item.deck];
        let row = Row {
            ts: self.clock.now(),
            front: self.card(&d.item).front.clone(),
            kind: RowKind::Undo {
                goal: d.item.goal,
                mode: self.mode,
            },
        };
        self.store.append_row(deck.name(), &row)?;
        match d.graded {
            Some(grade) => {
                if let Some(evs) = self.session_events.get_mut(&self.key(&d.item)) {
                    evs.pop();
                }
                self.summary.reviews -= 1;
                if grade.is_success() {
                    self.summary.correct -= 1;
                }
            }
            None => self.summary.skipped -= 1,
        }
        Ok(())
    }

    fn progress(&self) -> String {
        let done = self.pool.saturating_sub(self.queue.remaining());
        let left = self.budget.saturating_sub(self.start.elapsed());
        let mins = left.as_secs().div_ceil(60);
        let time = if mins == 0 {
            "time's up".to_string()
        } else {
            format!("{mins} min left")
        };
        format!("{done}/{} · {time}", self.pool)
    }

    fn print_result(&self, item: &Item, grade: Grade) {
        let label = if grade == Grade::Again {
            "again soon".to_string()
        } else {
            let evs = self.events(item);
            self.model
                .memory(&evs, self.clock)
                .map(|m| interval_label(self.model.scheduled_days(&m) as f32))
                .unwrap_or_default()
        };
        let sym = if grade.is_success() {
            self.term.out.green("✓")
        } else {
            self.term.out.red("✗")
        };
        println!(
            "{sym} {}",
            self.term.out.dim(&format!("{label} · {}", self.progress()))
        );
    }

    fn print_skip(&self) {
        println!(
            "{}",
            self.term
                .out
                .dim(&format!("→ skipped · {}", self.progress()))
        );
    }

    fn time_up(&mut self) -> Result<bool> {
        if self.start.elapsed() < self.budget {
            return Ok(false);
        }
        let remaining = self.queue.remaining();
        if remaining == 0 {
            return Ok(true);
        }
        println!();
        println!(
            "{}",
            self.term
                .out
                .bold(&format!("Time's up. {}", self.summary_line()))
        );
        let prompt = format!(
            "{} remain. Continue for {} more? [y/N] ",
            plural(remaining, "card"),
            plural(self.minutes as usize, "minute")
        );
        print!("{prompt}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let key = term::read_key()?;
        term::clear_line();
        if matches!(key, Key::Char('y') | Key::Char('Y')) {
            self.budget += Duration::from_secs(u64::from(self.minutes) * 60);
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn summary_line(&self) -> String {
        let s = &self.summary;
        if s.reviews == 0 {
            return if s.skipped > 0 {
                format!("{} skipped, nothing graded.", plural(s.skipped, "card"))
            } else {
                "Nothing reviewed.".to_string()
            };
        }
        let secs = self.start.elapsed().as_secs();
        let took = if secs < 60 {
            format!("{secs} s")
        } else {
            plural(((secs + 30) / 60).max(1) as usize, "minute")
        };
        let pct = (s.correct as f64 * 100.0 / s.reviews as f64).round() as u64;
        let mut parts = vec![
            format!("{} in {took}", plural(s.reviews, "review")),
            format!("{} correct ({pct}%)", s.correct),
        ];
        if s.new_seen > 0 {
            parts.push(format!("{} new", s.new_seen));
        }
        if s.skipped > 0 {
            parts.push(format!("{} skipped", s.skipped));
        }
        parts.join(" · ")
    }

    fn finish(mut self) -> Result<Summary> {
        self.summary.secs = self.start.elapsed().as_secs();
        self.summary.remaining = self.queue.remaining();
        println!();
        println!("{}", self.term.out.bold(&self.summary_line()));
        Ok(self.summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: usize) -> Item {
        Item {
            deck: 0,
            card: n,
            goal: Goal::Forward,
            retrievability: 0.0,
        }
    }

    fn queue(due: usize, new: usize, limit: usize) -> Queue {
        Queue {
            due: (0..due).map(item).collect(),
            new: (100..100 + new).map(item).collect(),
            relearn: VecDeque::new(),
            new_limit: limit,
            new_taken: 0,
            shown: 0,
            since_new: 0,
        }
    }

    fn drain(q: &mut Queue) -> Vec<usize> {
        let mut out = Vec::new();
        while let Some((i, o)) = q.pick() {
            q.account(o);
            out.push(i.card);
        }
        out
    }

    #[test]
    fn new_cards_interleave_one_per_five_reviews() {
        let mut q = queue(12, 3, 2);
        assert_eq!(q.remaining(), 14);
        let order = drain(&mut q);
        assert_eq!(order, vec![0, 1, 2, 3, 4, 100, 5, 6, 7, 8, 9, 101, 10, 11]);
    }

    #[test]
    fn new_cards_fill_when_nothing_is_due() {
        let mut q = queue(0, 3, 3);
        assert_eq!(drain(&mut q), vec![100, 101, 102]);
    }

    #[test]
    fn relearn_returns_after_gap_or_at_the_end() {
        let mut q = queue(8, 0, 0);
        let (first, o) = q.pick().unwrap();
        q.account(o);
        q.requeue(first.clone());
        assert_eq!(q.remaining(), 8);
        let order = drain(&mut q);
        assert_eq!(order, vec![1, 2, 3, 4, 5, 0, 6, 7]);

        let mut q = queue(2, 0, 0);
        let (first, o) = q.pick().unwrap();
        q.account(o);
        q.requeue(first);
        assert_eq!(drain(&mut q), vec![1, 0]);
    }

    #[test]
    fn undo_restores_counters_and_withdraws_relearn() {
        let mut q = queue(3, 1, 1);
        let before = q.counters();
        let (a, o) = q.pick().unwrap();
        q.account(o);
        q.requeue(a.clone());
        q.withdraw(&a);
        q.push_front(a, o);
        q.restore(before);
        assert_eq!(q.remaining(), 4);
        assert_eq!(drain(&mut q), vec![0, 1, 2, 100]);
    }
}
