//! Deck files: one card per line in the Obsidian `front::back` convention.
//! Everything that is not a card line is ignored, so a deck can be an
//! ordinary Markdown note. Lines starting with `#` (headings, comments) and
//! fenced code blocks are never cards, so `# 食::to eat` comments a card out.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;

use crate::text::{front_key, nfc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// Normalized front: trimmed, NFC. This is the card's identity.
    pub front: String,
    /// Trimmed, NFC.
    pub back: String,
    /// `:::` lines also produce a reverse (back → front) goal.
    pub reverse: bool,
    /// 1-based line number in the deck file.
    pub line: usize,
}

impl Card {
    /// The side shown as the prompt for a goal.
    pub fn prompt(&self, goal: crate::types::Goal) -> &str {
        match goal {
            crate::types::Goal::Forward => &self.front,
            crate::types::Goal::Reverse => &self.back,
        }
    }

    /// The side expected as the answer for a goal.
    pub fn answer(&self, goal: crate::types::Goal) -> &str {
        match goal {
            crate::types::Goal::Forward => &self.back,
            crate::types::Goal::Reverse => &self.front,
        }
    }
}

/// A non-fatal problem in a deck or log file, reported with its location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

#[derive(Debug)]
pub struct Deck {
    pub name: String,
    pub cards: Vec<Card>,
    pub warnings: Vec<Warning>,
}

/// Split a line into (front, back, reverse) if it is a card line.
/// Splits on the first `:::` if present, otherwise the first `::`.
pub fn split_card(line: &str) -> Option<(&str, &str, bool)> {
    if let Some(i) = line.find(":::") {
        Some((&line[..i], &line[i + 3..], true))
    } else {
        line.find("::").map(|i| (&line[..i], &line[i + 2..], false))
    }
}

/// Would a line beginning with this front be ignored by the parser? Fronts
/// that start with `#` or a code fence can never become cards, so `add` and
/// `rename` refuse them instead of writing a line that silently disappears.
pub fn front_is_ignored(front: &str) -> bool {
    let f = front.trim_start();
    f.starts_with('#') || f.starts_with("```")
}

impl Deck {
    /// `file` is how the deck is named in messages, e.g. `decks/cantonese.md`.
    pub fn parse(name: &str, file: &str, text: &str) -> Deck {
        let mut cards: Vec<Card> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut warnings = Vec::new();
        let mut in_fence = false;

        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            if raw.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence || front_is_ignored(raw) {
                continue;
            }
            let Some((front_raw, back_raw, reverse)) = split_card(raw) else {
                continue;
            };
            let front = front_key(front_raw);
            let back = nfc(back_raw.trim());
            if front.is_empty() {
                warnings.push(Warning {
                    file: file.to_string(),
                    line: line_no,
                    message: "card line has an empty front; skipped".into(),
                });
                continue;
            }
            if back.is_empty() {
                warnings.push(Warning {
                    file: file.to_string(),
                    line: line_no,
                    message: format!("\"{front}\" has an empty back; skipped"),
                });
                continue;
            }
            if let Some(&first_line) = seen.get(&front) {
                warnings.push(Warning {
                    file: file.to_string(),
                    line: line_no,
                    message: format!(
                        "duplicate front \"{front}\" (first seen on line {first_line}); using line \
                         {first_line}. Merge the backs or disambiguate the front."
                    ),
                });
                continue;
            }
            seen.insert(front.clone(), line_no);
            cards.push(Card {
                front,
                back,
                reverse,
                line: line_no,
            });
        }

        Deck {
            name: name.to_string(),
            cards,
            warnings,
        }
    }

    pub fn load(name: &str, path: &Path, file: &str) -> io::Result<Deck> {
        let text = std::fs::read_to_string(path)?;
        Ok(Deck::parse(name, file, &text))
    }

    pub fn find(&self, front: &str) -> Option<&Card> {
        self.cards.iter().find(|c| c.front == front)
    }

    pub fn has(&self, front: &str) -> bool {
        self.find(front).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Deck {
        Deck::parse("t", "decks/t.md", text)
    }

    #[test]
    fn parses_forward_and_reverse_cards() {
        let d = parse("# heading\n\n食::to eat\n飲 ::: to drink\nprose line\n");
        assert_eq!(d.cards.len(), 2);
        assert_eq!(
            d.cards[0],
            Card {
                front: "食".into(),
                back: "to eat".into(),
                reverse: false,
                line: 3
            }
        );
        assert_eq!(
            d.cards[1],
            Card {
                front: "飲".into(),
                back: "to drink".into(),
                reverse: true,
                line: 4
            }
        );
        assert!(d.warnings.is_empty());
    }

    #[test]
    fn splits_on_first_triple_then_first_double() {
        assert_eq!(
            split_card("Vec::new():::constructor"),
            Some(("Vec::new()", "constructor", true))
        );
        assert_eq!(split_card("a::b::c"), Some(("a", "b::c", false)));
        assert_eq!(split_card("no card here"), None);
    }

    #[test]
    fn warns_on_duplicates_and_empty_sides() {
        let d = parse("a::1\na::2\n::x\ny::\n");
        assert_eq!(d.cards.len(), 1);
        assert_eq!(d.warnings.len(), 3);
        assert!(d.warnings[0].message.contains("duplicate"));
        assert_eq!(d.warnings[0].line, 2);
        assert!(d.warnings[1].message.contains("empty front"));
        assert!(d.warnings[2].message.contains("empty back"));
    }

    #[test]
    fn ignores_code_fences_and_hash_lines() {
        let d = parse("```\nstd::io\n```\n# heading::not a card\n  # commented::out\nreal::card\n");
        assert_eq!(d.cards.len(), 1);
        assert_eq!(d.cards[0].front, "real");
    }

    #[test]
    fn comment_like_fronts_are_refused_up_front() {
        assert!(front_is_ignored("#tag"));
        assert!(front_is_ignored("  # spaced"));
        assert!(front_is_ignored("```rust"));
        assert!(!front_is_ignored("C# language"));
        assert!(!front_is_ignored("plain"));
    }

    #[test]
    fn normalizes_fronts() {
        let d = parse("  e\u{301}  ::x\n");
        assert_eq!(d.cards[0].front, "é");
        assert!(d.has("é"));
    }
}
