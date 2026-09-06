//! Terminal plumbing: TTY detection, colors, single-key reads, line prompts.

use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, queue};

use crate::text;

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
    pub fn cyan(self, s: &str) -> String {
        self.paint("36", s)
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

    pub fn size(&self) -> (usize, usize) {
        terminal::size()
            .map(|(w, h)| (w as usize, h as usize))
            .unwrap_or((80, 24))
    }
}

/// Whether the alternate screen is active, so a Ctrl-C during cooked-mode
/// input can restore the terminal before exiting.
static ALT_ACTIVE: AtomicBool = AtomicBool::new(false);
static CTRLC: Once = Once::new();

fn restore_terminal() {
    let mut out = io::stdout();
    if ALT_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = execute!(out, cursor::Show, LeaveAlternateScreen);
    }
    let _ = terminal::disable_raw_mode();
}

/// The review screen: the whole session is drawn on the terminal's
/// alternate screen, one frame per state, so nothing piles up and the
/// scrollback is untouched. Rows are fixed: header, rule, ticker, blank,
/// then the body from `BODY_ROW`, with the footer on the last row.
pub struct Screen {
    active: bool,
}

/// Row where the card body starts (0-based).
pub const BODY_ROW: usize = 4;

impl Screen {
    pub fn enter() -> io::Result<Screen> {
        CTRLC.call_once(|| {
            let _ = ctrlc::set_handler(|| {
                restore_terminal();
                std::process::exit(130);
            });
        });
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
        ALT_ACTIVE.store(true, Ordering::SeqCst);
        Ok(Screen { active: true })
    }

    pub fn leave(&mut self) {
        if self.active {
            self.active = false;
            restore_terminal();
        }
    }

    /// Draw one frame.
    pub fn draw(&self, term: &Term, frame: &Frame) {
        let (w, h) = term.size();
        let style = term.out;
        let mut out = io::stdout().lock();
        let _ = queue!(out, terminal::Clear(terminal::ClearType::All));
        let gap = w
            .saturating_sub(2 + text::width(&frame.left) + text::width(&frame.right))
            .max(1);
        let _ = queue!(out, cursor::MoveTo(0, 0));
        let _ = write!(
            out,
            " {}{}{}",
            style.bold(&frame.left),
            " ".repeat(gap),
            style.dim(&frame.right)
        );
        let _ = queue!(out, cursor::MoveTo(0, 1));
        let _ = write!(out, " {}", style.dim(&"─".repeat(w.saturating_sub(2))));
        let _ = queue!(out, cursor::MoveTo(0, 2));
        let _ = write!(out, " {}", frame.ticker);
        for (i, line) in frame.body.iter().enumerate() {
            let row = BODY_ROW + i;
            if row + 1 >= h {
                break;
            }
            let _ = queue!(out, cursor::MoveTo(0, row as u16));
            let _ = write!(out, "{line}");
        }
        let _ = queue!(out, cursor::MoveTo(0, h.saturating_sub(1) as u16));
        let _ = write!(out, " {}", frame.footer);
        match frame.cursor_at {
            Some((line, col)) => {
                let _ = queue!(
                    out,
                    cursor::MoveTo(col as u16, (BODY_ROW + line) as u16),
                    cursor::Show
                );
            }
            None => {
                let _ = queue!(out, cursor::Hide);
            }
        }
        let _ = out.flush();
    }
}

/// One review frame. `left` and `right` are plain header text; `ticker`,
/// `body`, and `footer` are already styled. `cursor_at` is a (body line,
/// column) to leave a visible cursor at for line input.
pub struct Frame<'a> {
    pub left: String,
    pub right: String,
    pub ticker: &'a str,
    pub body: &'a [String],
    pub footer: &'a str,
    pub cursor_at: Option<(usize, usize)>,
}

impl Drop for Screen {
    fn drop(&mut self) {
        self.leave();
    }
}

/// A footer keymap: the key in normal weight, the action dim.
pub fn hints(term: &Term, pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k} {}", term.out.dim(v)))
        .collect::<Vec<_>>()
        .join("   ")
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
