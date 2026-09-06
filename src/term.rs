//! Terminal plumbing: TTY detection, colors, single-key reads, line prompts.

use std::io::{self, BufRead, IsTerminal, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub on: bool,
}

impl Style {
    fn paint(self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    pub fn dim(self, s: &str) -> String {
        self.paint("2", s)
    }
    pub fn bold(self, s: &str) -> String {
        self.paint("1", s)
    }
    pub fn green(self, s: &str) -> String {
        self.paint("32", s)
    }
    pub fn red(self, s: &str) -> String {
        self.paint("31", s)
    }
    pub fn yellow(self, s: &str) -> String {
        self.paint("33", s)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Term {
    /// Style for stdout.
    pub out: Style,
    /// Style for stderr.
    pub err: Style,
    /// stdin and stdout are terminals and `--no-input` is not set.
    pub interactive: bool,
    pub quiet: bool,
}

impl Term {
    pub fn detect(no_color: bool, no_input: bool, quiet: bool) -> Term {
        let env_no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
        let allow = !no_color && !env_no_color && !dumb;
        let stdout_tty = io::stdout().is_terminal();
        let stdin_tty = io::stdin().is_terminal();
        Term {
            out: Style {
                on: allow && stdout_tty,
            },
            err: Style {
                on: allow && io::stderr().is_terminal(),
            },
            interactive: stdin_tty && stdout_tty && !no_input,
            quiet,
        }
    }

    pub fn width(&self) -> usize {
        terminal::size().map(|(w, _)| w as usize).unwrap_or(80)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Space,
    Enter,
    Char(char),
    Left,
    Right,
    Up,
    Down,
    Backspace,
    Esc,
    CtrlC,
    CtrlD,
    Other,
}

struct RawGuard;

impl RawGuard {
    fn enable() -> io::Result<RawGuard> {
        terminal::enable_raw_mode()?;
        Ok(RawGuard)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// Read one key press. Raw mode is enabled only for the duration of the read,
/// so cooked-mode line input (and IME composition) works everywhere else.
pub fn read_key() -> io::Result<Key> {
    let _guard = RawGuard::enable()?;
    loop {
        if let Event::Key(k) = event::read()? {
            if k.kind == KeyEventKind::Release {
                continue;
            }
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            return Ok(match k.code {
                KeyCode::Char('c') if ctrl => Key::CtrlC,
                KeyCode::Char('d') if ctrl => Key::CtrlD,
                KeyCode::Char(' ') => Key::Space,
                KeyCode::Char(c) => Key::Char(c),
                KeyCode::Enter => Key::Enter,
                KeyCode::Left => Key::Left,
                KeyCode::Right => Key::Right,
                KeyCode::Up => Key::Up,
                KeyCode::Down => Key::Down,
                KeyCode::Backspace => Key::Backspace,
                KeyCode::Esc => Key::Esc,
                _ => Key::Other,
            });
        }
    }
}

/// Column where transient key hints start, when the terminal is wide enough.
pub const HINT_COL: usize = 38;

/// Print a dim key hint on the current line without a newline.
pub fn show_hint(term: &Term, hint: &str) {
    let width = term.width();
    let col = if HINT_COL + crate::text::width(hint) < width {
        HINT_COL
    } else {
        2
    };
    let mut out = io::stdout().lock();
    let _ = write!(out, "{}{}", " ".repeat(col), term.out.dim(hint));
    let _ = out.flush();
}

/// Erase the current line (the hint) and return to column 0.
pub fn clear_line() {
    let mut out = io::stdout().lock();
    let _ = write!(out, "\r\x1b[2K");
    let _ = out.flush();
}

/// Cooked-mode line input. `None` on end of input.
pub fn read_line(prompt: &str) -> io::Result<Option<String>> {
    {
        let mut out = io::stdout().lock();
        write!(out, "{prompt}")?;
        out.flush()?;
    }
    let mut line = String::new();
    let n = io::stdin().lock().read_line(&mut line)?;
    if n == 0 {
        println!();
        return Ok(None);
    }
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(Some(line))
}

/// Ask a yes/no question on the terminal. End of input counts as no.
pub fn confirm(prompt: &str, default_yes: bool) -> io::Result<bool> {
    let suffix = if default_yes { "[Y/n] " } else { "[y/N] " };
    loop {
        let Some(line) = read_line(&format!("{prompt} {suffix}"))? else {
            return Ok(false);
        };
        let answer = line.trim().to_lowercase();
        match answer.as_str() {
            "" => return Ok(default_yes),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => continue,
        }
    }
}

/// Ask for a line of text; empty answers are asked again. `None` on EOF.
pub fn ask(prompt: &str) -> io::Result<Option<String>> {
    loop {
        let Some(line) = read_line(prompt)? else {
            return Ok(None);
        };
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
    }
}
