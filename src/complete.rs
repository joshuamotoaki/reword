//! Tab-completion candidates: decks, headings, and card fronts.

use std::ffi::OsStr;
use std::path::PathBuf;

use clap_complete::CompletionCandidate;

use crate::deck;
use crate::store::Store;

/// Existing deck names, for `review`, `add`, `edit`, `rename`, `stats`.
pub fn decks(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = current.to_str() else {
        return Vec::new();
    };
    match store().list_decks() {
        Ok(names) => names
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            .map(CompletionCandidate::new)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Heading titles in the selected decks (or every deck), for `--under`.
pub fn headings(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = current.to_str() else {
        return Vec::new();
    };
    let store = store();
    let names = match selected_decks(&store) {
        Ok(n) => n,
        Err(_) => return Vec::new(),
    };
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for name in names {
        let Ok(d) = store.load_deck(&name) else {
            continue;
        };
        for h in deck::collect_headings(&d.cards) {
            if h.starts_with(prefix) && !seen.iter().any(|s| s == &h) {
                seen.push(h.clone());
                out.push(CompletionCandidate::new(h));
            }
        }
    }
    out
}

/// Fronts in the `rename` deck, for the old-front argument.
pub fn fronts(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(prefix) = current.to_str() else {
        return Vec::new();
    };
    let Some(deck) = first_positional_after("rename") else {
        return Vec::new();
    };
    let store = store();
    let Ok(name) = store.resolve_deck(&deck) else {
        return Vec::new();
    };
    let Ok(d) = store.load_deck(&name) else {
        return Vec::new();
    };
    d.cards
        .into_iter()
        .filter(|c| c.front.starts_with(prefix))
        .map(|c| CompletionCandidate::new(c.front))
        .collect()
}

fn store() -> Store {
    Store::locate(dir_flag().as_deref())
}

fn dir_flag() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--dir" {
            return args.get(i + 1).map(PathBuf::from);
        }
        if let Some(v) = args[i].strip_prefix("--dir=") {
            return Some(PathBuf::from(v));
        }
        i += 1;
    }
    None
}

fn selected_decks(store: &Store) -> crate::error::Result<Vec<String>> {
    let named = positionals_after("review");
    if named.is_empty() {
        return store.list_decks();
    }
    Ok(named)
}

fn first_positional_after(command: &str) -> Option<String> {
    positionals_after(command).into_iter().next()
}

/// Non-flag words after `command` on the line being completed.
fn positionals_after(command: &str) -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    let Some(start) = args.iter().position(|a| a == command) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut skip_value = false;
    for a in args.into_iter().skip(start + 1) {
        if skip_value {
            skip_value = false;
            continue;
        }
        if a == "--" {
            continue;
        }
        if takes_value(&a) {
            skip_value = !a.contains('=');
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        out.push(a);
    }
    out
}

fn takes_value(arg: &str) -> bool {
    matches!(
        arg.split('=').next(),
        Some("--dir" | "--under" | "--minutes" | "-m" | "--cards" | "-n" | "--new")
    )
}
