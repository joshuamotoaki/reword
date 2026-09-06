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
use crate::term::{self, Key, Style, Term};
use crate::text::{self, answer_matches, plural};
use crate::types::{Goal, Grade, Mode};

/// A card graded Again comes back after this many other cards.
const RELEARN_GAP: usize = 5;
/// One new card per this many reviews while due cards remain.
const NEW_EVERY: usize = 5;

pub struct Options {
    pub mode: Mode,
    pub minutes: u32,
    /// No time budget; when the queue empties, start another round.
    pub endless: bool,
    /// Deck label for the header.
    pub label: String,
    /// One-line notes for the ticker at session start.
    pub notes: Vec<String>,
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

    fn withdraw_all(&mut self, item: &Item) {
        self.due.retain(|i| !same(i, item));
        self.new.retain(|i| !same(i, item));
        self.withdraw(item);
    }

    /// A new endless round: learned cards by retrievability, leftovers as new.
    fn refill(&mut self, due: Vec<Item>, new: Vec<Item>) {
        self.due = due.into();
        self.new = new.into();
        self.relearn.clear();
        self.new_limit = self.new.len();
        self.new_taken = 0;
        self.since_new = 0;
    }
}

struct Done {
    item: Item,
    origin: Origin,
    stage: Stage,
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
    endless: bool,
    multi: bool,
    queue: Queue,
    /// Every card this session can loop over (`--endless` re-queues these).
    pool: Vec<Item>,
    start: Instant,
    budget: Duration,
    done: Vec<Done>,
    session_events: HashMap<(usize, String, Goal), Vec<ReviewEvent>>,
    summary: Summary,
    label: String,
    /// What happened to the previous card, shown under the header.
    ticker: String,
    screen: term::Screen,
}

/// Left margin of the card body.
const MARGIN: usize = 5;

pub fn run(
    store: &Store,
    decks: &[LoadedDeck],
    plan: Plan,
    model: &Model,
    clock: &Clock,
    term: &Term,
    opts: Options,
) -> Result<Summary> {
    let mut notes = vec![{
        let noun = if opts.endless { "learned" } else { "due" };
        let mut parts = vec![format!("{} {noun}", plan.due.len())];
        let new = plan.new_limit.min(plan.new.len());
        if new > 0 {
            parts.push(format!("{new} new"));
        }
        parts.join(", ")
    }];
    notes.extend(opts.notes);
    if opts.mode == Mode::Typed {
        notes.push("enter checks · empty line reveals".into());
    }
    let pool: Vec<Item> = plan.due.iter().chain(plan.new.iter()).cloned().collect();
    let queue = Queue {
        due: plan.due.into(),
        new: plan.new.into(),
        relearn: VecDeque::new(),
        new_limit: plan.new_limit,
        new_taken: 0,
        shown: 0,
        since_new: 0,
    };
    let screen = term::Screen::enter()?;
    let mut s = Session {
        store,
        decks,
        model,
        clock,
        term,
        mode: opts.mode,
        minutes: opts.minutes,
        endless: opts.endless,
        multi: decks.len() > 1,
        queue,
        pool,
        start: Instant::now(),
        budget: Duration::from_secs(u64::from(opts.minutes) * 60),
        done: Vec::new(),
        session_events: HashMap::new(),
        summary: Summary::default(),
        label: opts.label,
        ticker: term.out.dim(&notes.join(" · ")),
        screen,
    };

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
                    None if s.endless && s.refill() => match s.queue.pick() {
                        Some((item, origin)) => (item, origin, Stage::Fresh),
                        None => break,
                    },
                    None => break,
                }
            }
        };
        let counters = s.queue.counters();
        s.queue.account(origin);
        let can_undo = !s.done.is_empty();
        let started = Instant::now();
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
                if grade == Grade::Again {
                    s.queue.requeue(item.clone());
                }
                if origin == Origin::New {
                    s.summary.new_seen += 1;
                }
                s.set_result(&item, grade);
                s.done.push(Done {
                    item,
                    origin,
                    stage,
                    counters,
                    graded: Some(grade),
                });
            }
            Step::Skipped => {
                s.skip(&item, elapsed_ms)?;
                s.set_skip(&item);
                s.done.push(Done {
                    item,
                    origin,
                    stage: Stage::Fresh,
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
                s.queue.withdraw_all(&d.item);
                if d.origin == Origin::New && d.graded.is_some() {
                    s.summary.new_seen -= 1;
                }
                s.queue.restore(d.counters);
                s.ticker = term.out.dim("↶ undo");
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

    fn draw(&self, body: &[String], footer: &str, cursor_at: Option<(usize, usize)>) {
        self.screen.draw(
            self.term,
            &term::Frame {
                left: format!("{} · {}", self.label, self.mode),
                right: self.progress(),
                ticker: &self.ticker,
                body,
                footer,
                cursor_at,
            },
        );
    }

    /// Wrap `s` into body lines with the left margin. `lead` (already
    /// styled, `lead_cells` wide) goes before the first line.
    fn lines(
        &self,
        s: &str,
        lead: &str,
        lead_cells: usize,
        paint: fn(Style, &str) -> String,
    ) -> Vec<String> {
        let avail = self
            .term
            .width()
            .saturating_sub(MARGIN + lead_cells + 1)
            .max(8);
        text::wrap(s, avail)
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let head = if i == 0 {
                    lead.to_string()
                } else {
                    " ".repeat(lead_cells)
                };
                format!("{}{head}{}", " ".repeat(MARGIN), paint(self.term.out, l))
            })
            .collect()
    }

    fn prompt_lines(&self, item: &Item) -> Vec<String> {
        let mut lines = self.lines(self.card(item).prompt(item.goal), "", 0, Style::bold);
        if self.multi
            && let Some(first) = lines.first_mut()
        {
            first.push_str(&format!(
                "   {}",
                self.term.out.dim(self.decks[item.deck].name())
            ));
        }
        lines
    }

    fn hints(&self, pairs: &[(&str, &str)], can_undo: bool) -> String {
        let pairs: Vec<(&str, &str)> = pairs
            .iter()
            .copied()
            .filter(|(k, _)| can_undo || *k != "←")
            .collect();
        term::hints(self.term, &pairs)
    }

    fn present(&self, item: &Item, stage: &Stage, can_undo: bool) -> Result<Step> {
        match self.mode {
            Mode::Recall => self.present_recall(item, stage, can_undo),
            Mode::Typed => self.present_typed(item, stage, can_undo),
        }
    }

    fn present_recall(&self, item: &Item, stage: &Stage, can_undo: bool) -> Result<Step> {
        let answer = self.card(item).answer(item.goal);
        let mut body = self.prompt_lines(item);
        body.push(String::new());
        if matches!(stage, Stage::Fresh) {
            let footer = self.hints(
                &[
                    ("space", "reveal"),
                    ("s", "skip"),
                    ("←", "undo"),
                    ("q", "quit"),
                ],
                can_undo,
            );
            self.draw(&body, &footer, None);
            loop {
                match term::read_key()? {
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
        body.extend(self.lines(answer, "", 0, Style::cyan));
        let footer = self.hints(
            &[
                ("space", "good"),
                ("a", "again"),
                ("h", "hard"),
                ("e", "easy"),
                ("s", "skip"),
                ("←", "undo"),
                ("q", "quit"),
            ],
            can_undo,
        );
        self.draw(&body, &footer, None);
        loop {
            let grade = match term::read_key()? {
                Key::Space | Key::Enter | Key::Char('g') | Key::Char('3') => Grade::Good,
                Key::Char('a') | Key::Char('1') => Grade::Again,
                Key::Char('h') | Key::Char('2') => Grade::Hard,
                Key::Char('e') | Key::Char('4') => Grade::Easy,
                Key::Char('s') => return Ok(Step::Skipped),
                Key::Char('q') | Key::Esc | Key::CtrlC | Key::CtrlD => return Ok(Step::Quit),
                Key::Left | Key::Backspace | Key::Char('u') if can_undo => return Ok(Step::Undo),
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

    fn input_line(&self, typed: &str) -> String {
        format!("{}{} {typed}", " ".repeat(MARGIN), self.term.out.dim("›"))
    }

    fn present_typed(&self, item: &Item, stage: &Stage, can_undo: bool) -> Result<Step> {
        let expected = self.card(item).answer(item.goal);
        let fresh = matches!(stage, Stage::Fresh);
        let mut body = self.prompt_lines(item);
        let input_row = body.len();
        let stage = match stage {
            Stage::Fresh => {
                body.push(self.input_line(""));
                let footer = self.hints(
                    &[
                        ("enter", "check"),
                        ("empty line", "reveal"),
                        ("ctrl-d", "quit"),
                    ],
                    false,
                );
                self.draw(&body, &footer, Some((input_row, MARGIN + 2)));
                let Some(line) = term::read_line("")? else {
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
            Stage::Checked { typed, correct } => Stage::Checked {
                typed: typed.clone(),
                correct: *correct,
            },
            Stage::Revealed => Stage::Revealed,
        };
        body.truncate(input_row);
        let style = self.term.out;
        match &stage {
            Stage::Fresh => unreachable!(),
            Stage::Revealed => {
                body.push(self.input_line(""));
                body.extend(self.lines(expected, "", 0, Style::cyan));
                let footer = self.hints(
                    &[
                        ("enter", "again"),
                        ("s", "skip"),
                        ("←", "undo"),
                        ("q", "quit"),
                    ],
                    can_undo,
                );
                self.draw(&body, &footer, None);
                loop {
                    match term::read_key()? {
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
                body.push(self.input_line(typed));
                body.extend(self.lines(
                    expected,
                    &format!("{} ", style.green("✓")),
                    2,
                    Style::cyan,
                ));
                let footer = self.hints(
                    &[
                        ("enter", "good"),
                        ("a", "again"),
                        ("s", "skip"),
                        ("←", "undo"),
                        ("q", "quit"),
                    ],
                    can_undo,
                );
                self.draw(&body, &footer, None);
                loop {
                    match term::read_key()? {
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
                body.push(self.input_line(typed));
                body.extend(self.lines(expected, &format!("{} ", style.red("✗")), 2, Style::cyan));
                let footer = self.hints(
                    &[
                        ("enter", "again"),
                        ("o", "typo, count it"),
                        ("s", "skip"),
                        ("←", "undo"),
                        ("q", "quit"),
                    ],
                    can_undo,
                );
                self.draw(&body, &footer, None);
                loop {
                    match term::read_key()? {
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

    /// "done/total": cards graded or skipped so far over what the session
    /// still holds. A card graded Again rejoins the queue, so the total grows
    /// rather than the count going backwards.
    fn progress(&self) -> String {
        let done = self.summary.reviews + self.summary.skipped;
        if self.endless {
            let mins = self.start.elapsed().as_secs() / 60;
            return format!("{done} done · {mins} min");
        }
        let total = done + self.queue.remaining();
        let left = self.budget.saturating_sub(self.start.elapsed());
        let mins = left.as_secs().div_ceil(60);
        let time = if mins == 0 {
            "time's up".to_string()
        } else {
            format!("{mins} min left")
        };
        format!("{done}/{total} · {time}")
    }

    fn set_result(&mut self, item: &Item, grade: Grade) {
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
        let name = text::truncate(self.card(item).prompt(item.goal), 24);
        self.ticker = format!("{sym} {}", self.term.out.dim(&format!("{name} · {label}")));
    }

    fn set_skip(&mut self, item: &Item) {
        let name = text::truncate(self.card(item).prompt(item.goal), 24);
        self.ticker = format!(
            "{} {}",
            self.term.out.yellow("→"),
            self.term.out.dim(&format!("{name} · skipped"))
        );
    }

    /// Re-queue the session's cards, weakest first, so `--endless` can loop.
    fn refill(&mut self) -> bool {
        if self.pool.is_empty() {
            return false;
        }
        let today = self.clock.today();
        let mut due = Vec::new();
        let mut new = Vec::new();
        for item in &self.pool {
            match self.model.memory(&self.events(item), self.clock) {
                Some(m) => due.push((item.clone(), self.model.retrievability(&m, today))),
                None => new.push(item.clone()),
            }
        }
        due.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        self.queue
            .refill(due.into_iter().map(|(item, _)| item).collect(), new);
        self.ticker = self.term.out.dim("another round");
        true
    }

    fn time_up(&mut self) -> Result<bool> {
        if self.endless || self.start.elapsed() < self.budget {
            return Ok(false);
        }
        let remaining = self.queue.remaining();
        if remaining == 0 {
            return Ok(true);
        }
        let style = self.term.out;
        let body = vec![
            format!(
                "{}{}",
                " ".repeat(MARGIN),
                style.bold(&format!("Time's up. {}", self.summary_line()))
            ),
            String::new(),
            format!(
                "{}{} remain. Continue for {} more?",
                " ".repeat(MARGIN),
                plural(remaining, "card"),
                plural(self.minutes as usize, "minute")
            ),
        ];
        let footer = self.hints(&[("y", "continue"), ("any other key", "finish")], false);
        self.draw(&body, &footer, None);
        let key = term::read_key()?;
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
        self.screen.leave();
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

    #[test]
    fn refill_replays_the_pool() {
        let mut q = queue(2, 1, 1);
        assert_eq!(drain(&mut q), vec![0, 1, 100]);
        assert_eq!(q.remaining(), 0);
        q.refill(vec![item(1), item(0)], vec![item(100)]);
        assert_eq!(q.remaining(), 3);
        assert_eq!(drain(&mut q), vec![1, 0, 100]);
    }

    #[test]
    fn withdraw_all_clears_every_bucket() {
        let mut q = queue(2, 1, 1);
        let (first, o) = q.pick().unwrap();
        q.account(o);
        q.requeue(first.clone());
        q.due.push_back(first.clone());
        q.withdraw_all(&first);
        assert_eq!(drain(&mut q), vec![1, 100]);
    }
}
