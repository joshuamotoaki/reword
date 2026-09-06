//! Unicode-aware text helpers: card keys, typed-answer matching, log escaping.

use unicode_normalization::UnicodeNormalization;
use unicode_width::UnicodeWidthStr;

pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// The identity of a card within a deck: trimmed and NFC-normalized front.
pub fn front_key(s: &str) -> String {
    nfc(s.trim())
}

/// Normalization used when checking a typed answer: NFC, trimmed, internal
/// whitespace collapsed, case-insensitive. Deliberately nothing else.
pub fn normalize_answer(s: &str) -> String {
    nfc(s)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Alternatives on an answer side are separated by " / ".
pub fn alternatives(answer: &str) -> impl Iterator<Item = &str> {
    answer.split(" / ")
}

/// Does a typed answer match the expected side, or any of its alternatives?
pub fn answer_matches(typed: &str, expected: &str) -> bool {
    let typed = normalize_answer(typed);
    if typed.is_empty() {
        return false;
    }
    if normalize_answer(expected) == typed {
        return true;
    }
    alternatives(expected).any(|alt| normalize_answer(alt) == typed)
}

/// Log fields are tab-separated, so tabs, newlines and backslashes inside a
/// field are escaped. Fronts and answers almost never contain them; this
/// keeps the format lossless when they do.
pub fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

pub fn unescape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Display width in terminal cells (CJK counts double).
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Greedy word wrap to `cells` display cells. Text without spaces (CJK)
/// breaks between characters.
pub fn wrap(s: &str, cells: usize) -> Vec<String> {
    let cells = cells.max(1);
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        let mut line_w = 0;
        for word in para.split(' ') {
            let ww = width(word);
            if ww > cells {
                // Too long for any line: break it by character.
                for ch in word.chars() {
                    let cw = width(&ch.to_string());
                    if line_w + cw > cells && !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                        line_w = 0;
                    }
                    line.push(ch);
                    line_w += cw;
                }
                continue;
            }
            let sep = if line.is_empty() { 0 } else { 1 };
            if line_w + sep + ww > cells {
                lines.push(std::mem::take(&mut line));
                line_w = 0;
            } else if sep == 1 {
                line.push(' ');
                line_w += 1;
            }
            line.push_str(word);
            line_w += ww;
        }
        lines.push(line);
    }
    lines
}

/// Pad on the right to `cells` display cells.
pub fn pad_right(s: &str, cells: usize) -> String {
    let w = width(s);
    if w >= cells {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(cells - w))
    }
}

/// Pad on the left to `cells` display cells.
pub fn pad_left(s: &str, cells: usize) -> String {
    let w = width(s);
    if w >= cells {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(cells - w))
    }
}

/// "1 card comes due" / "3 cards come due".
pub fn cards_come_due(n: usize) -> String {
    if n == 1 {
        "1 card comes due".into()
    } else {
        format!("{n} cards come due")
    }
}

/// Plural helper: `plural(1, "card")` → "1 card", `plural(2, "card")` → "2 cards".
pub fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_matching_is_forgiving_about_case_and_space() {
        assert!(answer_matches("  To   EAT ", "to eat"));
        assert!(answer_matches("thank you", "excuse me / thank you"));
        assert!(answer_matches(
            "excuse me / thank you",
            "excuse me / thank you"
        ));
        assert!(!answer_matches("", "to eat"));
        assert!(!answer_matches("eat", "to eat"));
    }

    #[test]
    fn nfc_equalizes_composed_and_decomposed() {
        let composed = "é";
        let decomposed = "e\u{301}";
        assert_eq!(front_key(composed), front_key(decomposed));
        assert!(answer_matches(decomposed, composed));
    }

    #[test]
    fn escaping_round_trips() {
        let s = "a\tb\nc\\d";
        assert_eq!(unescape_field(&escape_field(s)), s);
        assert!(!escape_field(s).contains('\t'));
    }

    #[test]
    fn cjk_width() {
        assert_eq!(width("食"), 2);
        assert_eq!(pad_right("食", 4), "食  ");
    }
}
