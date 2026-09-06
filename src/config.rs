//! `config.toml` and `params.toml`. Everything is optional.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Mode;

pub const DEFAULT_RETENTION: f32 = 0.90;
pub const DEFAULT_SESSION_MINUTES: u32 = 10;
pub const DEFAULT_NEW_PER_DAY: u32 = 10;

#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    desired_retention: Option<f32>,
    session_minutes: Option<u32>,
    new_per_day: Option<u32>,
    mode: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct ParamsFile {
    parameters: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub desired_retention: f32,
    pub session_minutes: u32,
    pub new_per_day: u32,
    pub mode: Option<Mode>,
    /// FSRS parameters from `params.toml`, if `reword optimize` has run.
    pub params: Option<Vec<f32>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            desired_retention: DEFAULT_RETENTION,
            session_minutes: DEFAULT_SESSION_MINUTES,
            new_per_day: DEFAULT_NEW_PER_DAY,
            mode: None,
            params: None,
        }
    }
}

impl Settings {
    pub fn load(store: &Store) -> Result<Settings> {
        let mut settings = Settings::default();

        let config_path = store.config_path();
        if config_path.is_file() {
            let text = std::fs::read_to_string(&config_path)?;
            let file: FileConfig = toml::from_str(&text).map_err(|e| {
                Error::new(format!("config.toml: {}", e.message()))
                    .hint("Keys: desired_retention, session_minutes, new_per_day, mode.")
            })?;
            if let Some(r) = file.desired_retention {
                if !(0.5..=0.99).contains(&r) {
                    return Err(Error::new(format!(
                        "config.toml: desired_retention = {r} is outside 0.5–0.99"
                    )));
                }
                settings.desired_retention = r;
            }
            if let Some(m) = file.session_minutes {
                if m == 0 {
                    return Err(Error::new(
                        "config.toml: session_minutes must be at least 1",
                    ));
                }
                settings.session_minutes = m;
            }
            if let Some(n) = file.new_per_day {
                settings.new_per_day = n;
            }
            if let Some(m) = file.mode {
                settings.mode = Some(Mode::parse(&m).ok_or_else(|| {
                    Error::new(format!(
                        "config.toml: mode = \"{m}\" is not \"recall\" or \"typed\""
                    ))
                })?);
            }
        }

        let params_path = store.params_path();
        if params_path.is_file() {
            let text = std::fs::read_to_string(&params_path)?;
            let file: ParamsFile = toml::from_str(&text)
                .map_err(|e| Error::new(format!("params.toml: {}", e.message())).hint("Expected `parameters = [ ... ]`. Delete the file to fall back to the defaults."))?;
            settings.params = Some(file.parameters);
        }

        Ok(settings)
    }
}

/// Written by `reword init`. Every line is a comment; the defaults apply.
pub const CONFIG_TEMPLATE: &str = "\
# reword settings. Every key is optional; these are the defaults.
# Flags beat this file: `reword review -m 5 --typed`.

# FSRS target: the probability of recall at which a card comes due.
# desired_retention = 0.90

# Minutes per session unless -m is given.
# session_minutes = 10

# Cap on new cards introduced per day across all decks. --new N overrides it.
# new_per_day = 10

# Set to \"recall\" or \"typed\" to stop `review` asking each time.
# mode = \"recall\"
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_defaults_and_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            root: dir.path().to_path_buf(),
        };
        let s = Settings::load(&store).unwrap();
        assert_eq!(s.session_minutes, 10);
        std::fs::write(
            store.config_path(),
            "session_minutes = 5\nmode = \"typed\"\n",
        )
        .unwrap();
        std::fs::write(store.params_path(), "parameters = [0.4, 0.9]\n").unwrap();
        let s = Settings::load(&store).unwrap();
        assert_eq!(s.session_minutes, 5);
        assert_eq!(s.mode, Some(Mode::Typed));
        assert_eq!(s.params.unwrap().len(), 2);
    }

    #[test]
    fn rejects_typos() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            root: dir.path().to_path_buf(),
        };
        std::fs::write(store.config_path(), "session_minute = 5\n").unwrap();
        assert!(Settings::load(&store).is_err());
        std::fs::write(store.config_path(), "mode = \"both\"\n").unwrap();
        assert!(Settings::load(&store).is_err());
    }

    #[test]
    fn template_parses() {
        let file: FileConfig = toml::from_str(CONFIG_TEMPLATE).unwrap();
        assert!(file.mode.is_none());
    }
}
