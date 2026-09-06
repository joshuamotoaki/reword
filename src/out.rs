//! stdout is for data, stderr is for messages. A closed pipe is not an error.

use std::io::{self, Write};

use crate::term::Term;

pub fn println(s: &str) {
    let mut out = io::stdout().lock();
    if let Err(e) = writeln!(out, "{s}")
        && e.kind() == io::ErrorKind::BrokenPipe
    {
        std::process::exit(0);
    }
}

pub fn json(value: &serde_json::Value) {
    println(&serde_json::to_string_pretty(value).unwrap_or_default());
}

pub fn warn(term: &Term, msg: &str) {
    if !term.quiet {
        eprintln!("{} {msg}", term.err.yellow("warning:"));
    }
}

pub fn note(term: &Term, msg: &str) {
    if !term.quiet {
        eprintln!("{msg}");
    }
}
