//! Per-deck append-only review logs, and the replay that turns them into
//! per-card review histories. History is the source of truth; memory state
//! is derived from it.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use jiff::Timestamp;

use crate::clock::{format_ts, parse_ts};
use crate::deck::Warning;
use crate::text::{escape_field, unescape_field};
use crate::types::{Goal, Grade, Mode};

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub ts: Timestamp,
    pub front: String,
    pub kind: RowKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    Review {
        goal: Goal,
        mode: Mode,
        grade: Grade,
        elapsed_ms: u64,
        answer: Option<String>,
        overridden: bool,
    },
    /// A postponed card. Never fed to FSRS.
    Skip {
        goal: Goal,
        mode: Mode,
        elapsed_ms: u64,
    },
    /// Cancels the most recent review or skip row for the same card and goal.
    Undo { goal: Goal, mode: Mode },
    /// Carries history from `front` to `to`.
    Rename { to: String },
}

impl Row {
    pub fn to_line(&self) -> String {
        let ts = format_ts(self.ts);
        let front = escape_field(&self.front);
        match &self.kind {
            RowKind::Review {
                goal,
                mode,
                grade,
                elapsed_ms,
                answer,
                overridden,
            } => {
                let mut cols = vec![
                    ts,
                    front,
                    goal.as_str().into(),
                    mode.as_str().into(),
                    grade.as_str().into(),
                    elapsed_ms.to_string(),
                ];
                if answer.is_some() || *overridden {
                    cols.push(escape_field(answer.as_deref().unwrap_or("")));
                }
                if *overridden {
                    cols.push("override".into());
                }
                cols.join("\t")
            }
            RowKind::Skip {
                goal,
                mode,
                elapsed_ms,
            } => [
                ts,
                front,
                goal.as_str().into(),
                mode.as_str().into(),
                "skip".into(),
                elapsed_ms.to_string(),
            ]
            .join("\t"),
            RowKind::Undo { goal, mode } => [
                ts,
                front,
                goal.as_str().into(),
                mode.as_str().into(),
                "undo".into(),
            ]
            .join("\t"),
            RowKind::Rename { to } => [ts, front, "rename".into(), escape_field(to)].join("\t"),
        }
    }

    pub fn parse(line: &str) -> Result<Row, String> {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 3 {
            return Err("expected at least 3 tab-separated columns".into());
        }
        let ts = parse_ts(cols[0]).ok_or_else(|| format!("bad timestamp \"{}\"", cols[0]))?;
        let front = unescape_field(cols[1]);
        if front.is_empty() {
            return Err("empty front".into());
        }
        if cols[2] == "rename" {
            let to = cols.get(3).map(|s| unescape_field(s)).unwrap_or_default();
            if to.is_empty() {
                return Err("rename row without a new front".into());
            }
            return Ok(Row {
                ts,
                front,
                kind: RowKind::Rename { to },
            });
        }
        let goal = Goal::parse(cols[2]).ok_or_else(|| format!("unknown goal \"{}\"", cols[2]))?;
        let mode_s = cols.get(3).copied().unwrap_or("");
        let mode = Mode::parse(mode_s).ok_or_else(|| format!("unknown mode \"{mode_s}\""))?;
        let grade_s = cols.get(4).copied().unwrap_or("");
        let elapsed = |s: Option<&&str>| -> Result<u64, String> {
            match s {
                None | Some(&"") => Ok(0),
                Some(v) => v.parse().map_err(|_| format!("bad elapsed_ms \"{v}\"")),
            }
        };
        let kind = match grade_s {
            "undo" => RowKind::Undo { goal, mode },
            "skip" => RowKind::Skip {
                goal,
                mode,
                elapsed_ms: elapsed(cols.get(5))?,
            },
            g => {
                let grade = Grade::parse(g).ok_or_else(|| format!("unknown grade \"{g}\""))?;
                let answer = cols
                    .get(6)
                    .filter(|s| !s.is_empty())
                    .map(|s| unescape_field(s));
                let overridden = cols.get(7) == Some(&"override");
                RowKind::Review {
                    goal,
                    mode,
                    grade,
                    elapsed_ms: elapsed(cols.get(5))?,
                    answer,
                    overridden,
                }
            }
        };
        Ok(Row { ts, front, kind })
    }
}

/// Read a log. A missing file is an empty log. Malformed lines become
/// warnings and are skipped; they are never fatal.
pub fn load(path: &Path, file: &str) -> io::Result<(Vec<Row>, Vec<Warning>)> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(e) => return Err(e),
    };
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    for (idx, line) in BufReader::new(f).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match Row::parse(&line) {
            Ok(row) => rows.push(row),
            Err(message) => warnings.push(Warning {
                file: file.to_string(),
                line: idx + 1,
                message: format!("{message}; line skipped"),
            }),
        }
    }
    Ok((rows, warnings))
}

/// Append one row. Creates the file if needed and repairs a missing trailing
/// newline so a hand-edited log never swallows the next row.
pub fn append(path: &Path, row: &Row) -> io::Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    if f.metadata()?.len() > 0 {
        f.seek(SeekFrom::End(-1))?;
        let mut last = [0u8; 1];
        f.read_exact(&mut last)?;
        if last[0] != b'\n' {
            f.write_all(b"\n")?;
        }
    }
    f.write_all(row.to_line().as_bytes())?;
    f.write_all(b"\n")?;
    f.flush()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReviewEvent {
    pub ts: Timestamp,
    pub grade: Grade,
    pub mode: Mode,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug)]
enum Entry {
    Review(ReviewEvent),
    Skip { ts: Timestamp },
}

impl Entry {
    fn ts(&self) -> Timestamp {
        match self {
            Entry::Review(r) => r.ts,
            Entry::Skip { ts } => *ts,
        }
    }
}

/// The replayed history of one deck: surviving reviews per front, one slot
/// per goal (see `Goal::index`). Fronts with no surviving review have no entry.
#[derive(Default, Debug, Clone)]
pub struct Ledger {
    reviews: HashMap<String, [Vec<ReviewEvent>; 2]>,
    /// Time of the most recent review or skip. Rename and undo rows are
    /// bookkeeping, not study, and do not count.
    pub last_activity: Option<Timestamp>,
}

impl Ledger {
    pub fn replay(rows: &[Row]) -> Ledger {
        // Logs merged from two devices (git's union driver) can interleave out
        // of order; time order is the order that matters.
        let mut rows: Vec<&Row> = rows.iter().collect();
        rows.sort_by_key(|r| r.ts);

        let mut entries: HashMap<(String, Goal), Vec<Entry>> = HashMap::new();
        let mut last_activity = None;
        for row in &rows {
            match &row.kind {
                RowKind::Review {
                    goal,
                    mode,
                    grade,
                    elapsed_ms,
                    ..
                } => {
                    last_activity = Some(row.ts);
                    entries
                        .entry((row.front.clone(), *goal))
                        .or_default()
                        .push(Entry::Review(ReviewEvent {
                            ts: row.ts,
                            grade: *grade,
                            mode: *mode,
                            elapsed_ms: *elapsed_ms,
                        }));
                }
                RowKind::Skip { goal, .. } => {
                    last_activity = Some(row.ts);
                    entries
                        .entry((row.front.clone(), *goal))
                        .or_default()
                        .push(Entry::Skip { ts: row.ts });
                }
                RowKind::Undo { goal, .. } => {
                    if let Some(v) = entries.get_mut(&(row.front.clone(), *goal)) {
                        v.pop();
                    }
                }
                RowKind::Rename { to } => {
                    for goal in [Goal::Forward, Goal::Reverse] {
                        if let Some(moved) = entries.remove(&(row.front.clone(), goal)) {
                            let target = entries.entry((to.clone(), goal)).or_default();
                            target.extend(moved);
                            target.sort_by_key(|e| e.ts());
                        }
                    }
                }
            }
        }

        let mut reviews: HashMap<String, [Vec<ReviewEvent>; 2]> = HashMap::new();
        for ((front, goal), list) in entries {
            let evs: Vec<ReviewEvent> = list
                .into_iter()
                .filter_map(|e| match e {
                    Entry::Review(r) => Some(r),
                    Entry::Skip { .. } => None,
                })
                .collect();
            if !evs.is_empty() {
                reviews.entry(front).or_default()[goal.index()] = evs;
            }
        }
        Ledger {
            reviews,
            last_activity,
        }
    }

    pub fn reviews(&self, front: &str, goal: Goal) -> &[ReviewEvent] {
        self.reviews
            .get(front)
            .map_or(&[], |slots| slots[goal.index()].as_slice())
    }

    /// Every (front, goal) with surviving reviews, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Goal, &[ReviewEvent])> {
        self.reviews.iter().flat_map(|(front, slots)| {
            [Goal::Forward, Goal::Reverse]
                .into_iter()
                .filter_map(move |goal| {
                    let evs = &slots[goal.index()];
                    (!evs.is_empty()).then_some((front.as_str(), goal, evs.as_slice()))
                })
        })
    }

    /// Fronts with surviving reviews, for orphan detection. Matches what
    /// `iter` reports, so `check` and the rename prompt agree.
    pub fn fronts(&self) -> impl Iterator<Item = &str> {
        self.reviews.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        parse_ts(s).unwrap()
    }

    fn review(t: &str, front: &str, grade: Grade) -> Row {
        Row {
            ts: ts(t),
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

    #[test]
    fn rows_round_trip() {
        let rows = vec![
            review("2026-09-06T02:11:09Z", "食", Grade::Good),
            Row {
                ts: ts("2026-09-06T02:11:31Z"),
                front: "唔該".into(),
                kind: RowKind::Review {
                    goal: Goal::Forward,
                    mode: Mode::Typed,
                    grade: Grade::Good,
                    elapsed_ms: 9800,
                    answer: Some("excuse\tme".into()),
                    overridden: true,
                },
            },
            Row {
                ts: ts("2026-09-06T02:12:00Z"),
                front: "a".into(),
                kind: RowKind::Skip {
                    goal: Goal::Reverse,
                    mode: Mode::Recall,
                    elapsed_ms: 5,
                },
            },
            Row {
                ts: ts("2026-09-06T02:12:01Z"),
                front: "a".into(),
                kind: RowKind::Undo {
                    goal: Goal::Reverse,
                    mode: Mode::Recall,
                },
            },
            Row {
                ts: ts("2026-09-06T02:12:02Z"),
                front: "a".into(),
                kind: RowKind::Rename { to: "b".into() },
            },
        ];
        for row in rows {
            let line = row.to_line();
            assert!(!line.contains('\n'));
            assert_eq!(Row::parse(&line).unwrap(), row, "line: {line}");
        }
        assert_eq!(
            review("2026-09-06T02:11:09Z", "食", Grade::Good).to_line(),
            "2026-09-06T02:11:09Z\t食\tforward\trecall\tgood\t1000"
        );
    }

    #[test]
    fn malformed_rows_are_errors() {
        assert!(Row::parse("nonsense").is_err());
        assert!(Row::parse("2026-09-06T02:11:09Z\t食\tsideways\trecall\tgood\t1").is_err());
        assert!(Row::parse("2026-09-06T02:11:09Z\t食\tforward\trecall\tmeh\t1").is_err());
    }

    #[test]
    fn undo_drops_the_previous_row_for_that_card() {
        let rows = vec![
            review("2026-09-06T02:11:09Z", "食", Grade::Good),
            review("2026-09-06T02:11:19Z", "食", Grade::Again),
            Row {
                ts: ts("2026-09-06T02:11:20Z"),
                front: "食".into(),
                kind: RowKind::Undo {
                    goal: Goal::Forward,
                    mode: Mode::Recall,
                },
            },
        ];
        let ledger = Ledger::replay(&rows);
        let evs = ledger.reviews("食", Goal::Forward);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].grade, Grade::Good);
    }

    #[test]
    fn rename_carries_history_and_merges() {
        let rows = vec![
            review("2026-09-06T02:11:09Z", "old", Grade::Good),
            review("2026-09-07T02:11:09Z", "new", Grade::Again),
            Row {
                ts: ts("2026-09-08T00:00:00Z"),
                front: "old".into(),
                kind: RowKind::Rename { to: "new".into() },
            },
        ];
        let ledger = Ledger::replay(&rows);
        assert!(ledger.reviews("old", Goal::Forward).is_empty());
        let evs = ledger.reviews("new", Goal::Forward);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].grade, Grade::Good);
        assert_eq!(evs[1].grade, Grade::Again);
        assert_eq!(ledger.fronts().collect::<Vec<_>>(), vec!["new"]);
        assert_eq!(ledger.iter().count(), 1);
        assert_eq!(
            ledger.last_activity,
            Some(ts("2026-09-07T02:11:09Z")),
            "a rename row is not study activity"
        );
    }

    #[test]
    fn skip_only_and_fully_undone_fronts_have_no_history() {
        let rows = vec![
            Row {
                ts: ts("2026-09-06T02:12:00Z"),
                front: "skipped".into(),
                kind: RowKind::Skip {
                    goal: Goal::Forward,
                    mode: Mode::Recall,
                    elapsed_ms: 5,
                },
            },
            review("2026-09-06T02:13:00Z", "undone", Grade::Good),
            Row {
                ts: ts("2026-09-06T02:13:01Z"),
                front: "undone".into(),
                kind: RowKind::Undo {
                    goal: Goal::Forward,
                    mode: Mode::Recall,
                },
            },
        ];
        let ledger = Ledger::replay(&rows);
        assert_eq!(ledger.fronts().count(), 0);
        assert_eq!(ledger.iter().count(), 0);
        assert_eq!(ledger.last_activity, Some(ts("2026-09-06T02:13:00Z")));
    }

    #[test]
    fn replay_sorts_by_time() {
        let rows = vec![
            review("2026-09-07T02:11:09Z", "x", Grade::Again),
            review("2026-09-06T02:11:09Z", "x", Grade::Good),
        ];
        let ledger = Ledger::replay(&rows);
        let evs = ledger.reviews("x", Goal::Forward);
        assert_eq!(evs[0].grade, Grade::Good);
    }

    #[test]
    fn append_and_load_with_missing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.log");
        std::fs::write(&path, "garbage line").unwrap();
        append(&path, &review("2026-09-06T02:11:09Z", "食", Grade::Good)).unwrap();
        let (rows, warnings) = load(&path, "decks/t.log").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].line, 1);
    }
}
