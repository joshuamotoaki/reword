//! The data directory: where decks and logs live, and how they are found.

use std::path::{Path, PathBuf};

use crate::deck::{Deck, Warning};
use crate::error::{Error, Result, bail};
use crate::history::{self, Ledger, Row};

pub const DEFAULT_DIR_NAME: &str = "reword";
pub const ENV_DIR: &str = "REWORD_DIR";

#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    /// `--dir` beats `$REWORD_DIR` beats `~/reword`.
    pub fn locate(flag: Option<&Path>) -> Store {
        let root = match flag {
            Some(p) => p.to_path_buf(),
            None => match std::env::var_os(ENV_DIR).filter(|v| !v.is_empty()) {
                Some(v) => PathBuf::from(v),
                None => std::env::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(DEFAULT_DIR_NAME),
            },
        };
        Store { root }
    }

    pub fn exists(&self) -> bool {
        self.decks_dir().is_dir()
    }

    pub fn require(&self) -> Result<()> {
        if self.exists() {
            Ok(())
        } else {
            Err(
                Error::new(format!("no data directory at {}", self.display()))
                    .hint("Run `reword init` to create it, or pass --dir / set $REWORD_DIR."),
            )
        }
    }

    pub fn decks_dir(&self) -> PathBuf {
        self.root.join("decks")
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn params_path(&self) -> PathBuf {
        self.root.join("params.toml")
    }

    pub fn deck_path(&self, name: &str) -> PathBuf {
        self.decks_dir().join(format!("{name}.md"))
    }

    pub fn log_path(&self, name: &str) -> PathBuf {
        self.decks_dir().join(format!("{name}.log"))
    }

    pub fn deck_label(&self, name: &str) -> String {
        format!("decks/{name}.md")
    }

    pub fn log_label(&self, name: &str) -> String {
        format!("decks/{name}.log")
    }

    /// Root with `~` for the home directory, for messages.
    pub fn display(&self) -> String {
        display_path(&self.root)
    }

    /// Sorted deck names: the stems of `decks/*.md`.
    pub fn list_decks(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let dir = match std::fs::read_dir(self.decks_dir()) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(e) => return Err(e.into()),
        };
        for entry in dir {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem.starts_with('.') {
                continue;
            }
            names.push(stem.to_string());
        }
        names.sort();
        Ok(names)
    }

    /// Logs without a deck file beside them.
    pub fn stray_logs(&self) -> Result<Vec<String>> {
        let decks = self.list_decks()?;
        let mut strays = Vec::new();
        let dir = match std::fs::read_dir(self.decks_dir()) {
            Ok(d) => d,
            Err(_) => return Ok(strays),
        };
        for entry in dir {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !decks.iter().any(|d| d == stem) {
                strays.push(stem.to_string());
            }
        }
        strays.sort();
        Ok(strays)
    }

    /// Turn a user-supplied deck argument (`cantonese`, `cantonese.md`,
    /// `decks/cantonese.md`) into a deck name, or explain what exists.
    pub fn resolve_deck(&self, arg: &str) -> Result<String> {
        let name = deck_name_from_arg(arg);
        if self.deck_path(&name).is_file() {
            return Ok(name);
        }
        let decks = self.list_decks()?;
        let hint = if decks.is_empty() {
            format!("There are no decks yet. Create one with `reword add {name} FRONT BACK`.")
        } else {
            let mut close: Vec<&String> = decks
                .iter()
                .filter(|d| {
                    d.to_lowercase().contains(&name.to_lowercase())
                        || name.to_lowercase().contains(&d.to_lowercase())
                })
                .collect();
            close.truncate(3);
            if close.is_empty() {
                format!("Decks: {}", decks.join(", "))
            } else {
                format!(
                    "Did you mean {}? Decks: {}",
                    close
                        .iter()
                        .map(|d| d.as_str())
                        .collect::<Vec<_>>()
                        .join(" or "),
                    decks.join(", ")
                )
            }
        };
        Err(Error::new(format!("no deck named \"{name}\"")).hint(hint))
    }

    pub fn validate_new_deck_name(name: &str) -> Result<()> {
        if name.is_empty() {
            bail!("deck name is empty");
        }
        if name.starts_with('.') || name.contains('/') || name.contains('\\') || name.contains('\0')
        {
            bail!("\"{name}\" is not a valid deck name (no slashes, no leading dot)");
        }
        Ok(())
    }

    pub fn load_deck(&self, name: &str) -> Result<Deck> {
        let path = self.deck_path(name);
        Deck::load(name, &path, &self.deck_label(name))
            .map_err(|e| Error::new(format!("{}: {e}", self.deck_label(name))))
    }

    pub fn load_rows(&self, name: &str) -> Result<(Vec<Row>, Vec<Warning>)> {
        history::load(&self.log_path(name), &self.log_label(name))
            .map_err(|e| Error::new(format!("{}: {e}", self.log_label(name))))
    }

    pub fn load_ledger(&self, name: &str) -> Result<(Ledger, Vec<Warning>)> {
        let (rows, warnings) = self.load_rows(name)?;
        Ok((Ledger::replay(&rows), warnings))
    }

    pub fn append_row(&self, name: &str, row: &Row) -> Result<()> {
        history::append(&self.log_path(name), row)
            .map_err(|e| Error::new(format!("{}: {e}", self.log_label(name))))
    }

    /// A deck plus its replayed history, as every command sees it.
    pub fn load_all(&self, names: &[String]) -> Result<Vec<LoadedDeck>> {
        names.iter().map(|n| self.load(n)).collect()
    }

    pub fn load(&self, name: &str) -> Result<LoadedDeck> {
        let deck = self.load_deck(name)?;
        let (ledger, log_warnings) = self.load_ledger(name)?;
        let mut warnings = deck.warnings.clone();
        warnings.extend(log_warnings);
        Ok(LoadedDeck {
            deck,
            ledger,
            warnings,
        })
    }
}

#[derive(Debug)]
pub struct LoadedDeck {
    pub deck: Deck,
    pub ledger: Ledger,
    pub warnings: Vec<Warning>,
}

impl LoadedDeck {
    pub fn name(&self) -> &str {
        &self.deck.name
    }
}

pub fn deck_name_from_arg(arg: &str) -> String {
    let p = Path::new(arg);
    let stem = if p.extension().and_then(|e| e.to_str()) == Some("md") {
        p.file_stem().and_then(|s| s.to_str()).unwrap_or(arg)
    } else {
        p.file_name().and_then(|s| s.to_str()).unwrap_or(arg)
    };
    stem.to_string()
}

pub fn display_path(path: &Path) -> String {
    if let Some(home) = std::env::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_arg_forms() {
        assert_eq!(deck_name_from_arg("cantonese"), "cantonese");
        assert_eq!(deck_name_from_arg("cantonese.md"), "cantonese");
        assert_eq!(deck_name_from_arg("decks/cantonese.md"), "cantonese");
        assert_eq!(deck_name_from_arg("/x/y/decks/cantonese.md"), "cantonese");
    }

    #[test]
    fn lists_decks_and_strays() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            root: dir.path().to_path_buf(),
        };
        std::fs::create_dir_all(store.decks_dir()).unwrap();
        std::fs::write(store.deck_path("b"), "x::y\n").unwrap();
        std::fs::write(store.deck_path("a"), "x::y\n").unwrap();
        std::fs::write(store.log_path("a"), "").unwrap();
        std::fs::write(store.log_path("gone"), "").unwrap();
        assert_eq!(store.list_decks().unwrap(), vec!["a", "b"]);
        assert_eq!(store.stray_logs().unwrap(), vec!["gone"]);
        assert!(store.resolve_deck("a.md").is_ok());
        let err = store.resolve_deck("zzz").unwrap_err();
        assert!(err.hint.unwrap().contains("Decks: a, b"));
    }
}
