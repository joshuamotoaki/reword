//! Small vocabulary shared by every module: goals, modes, grades.

use std::fmt;

/// Which direction a card is asked in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Goal {
    /// front → back
    Forward,
    /// back → front (only for `:::` lines)
    Reverse,
}

impl Goal {
    pub fn as_str(self) -> &'static str {
        match self {
            Goal::Forward => "forward",
            Goal::Reverse => "reverse",
        }
    }

    pub fn parse(s: &str) -> Option<Goal> {
        match s {
            "forward" => Some(Goal::Forward),
            "reverse" => Some(Goal::Reverse),
            _ => None,
        }
    }

    /// Slot in a per-goal pair: forward first, reverse second.
    pub fn index(self) -> usize {
        match self {
            Goal::Forward => 0,
            Goal::Reverse => 1,
        }
    }
}

impl fmt::Display for Goal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a session is run. Never mixed within one session.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Mode {
    Recall,
    Typed,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Recall => "recall",
            Mode::Typed => "typed",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "recall" => Some(Mode::Recall),
            "typed" => Some(Mode::Typed),
            _ => None,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A memory observation. Maps 1:1 onto FSRS ratings.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Grade {
    Again,
    Hard,
    Good,
    Easy,
}

impl Grade {
    pub fn as_str(self) -> &'static str {
        match self {
            Grade::Again => "again",
            Grade::Hard => "hard",
            Grade::Good => "good",
            Grade::Easy => "easy",
        }
    }

    pub fn parse(s: &str) -> Option<Grade> {
        match s {
            "again" => Some(Grade::Again),
            "hard" => Some(Grade::Hard),
            "good" => Some(Grade::Good),
            "easy" => Some(Grade::Easy),
            _ => None,
        }
    }

    /// FSRS rating: again=1, hard=2, good=3, easy=4.
    pub fn rating(self) -> u32 {
        match self {
            Grade::Again => 1,
            Grade::Hard => 2,
            Grade::Good => 3,
            Grade::Easy => 4,
        }
    }

    pub fn is_success(self) -> bool {
        !matches!(self, Grade::Again)
    }
}

impl fmt::Display for Grade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
