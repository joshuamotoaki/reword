//! Command-line surface. `reword` alone prints status and concise help.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;

const AFTER_HELP: &str = "\
Examples:
  reword init                       create ~/reword with an example deck
  reword add cantonese 食 'to eat'   append a card
  reword review                     review every deck for the configured minutes
  reword review cantonese -m 5      one deck, five minutes
  reword review -n 20               twenty cards, no time limit
  reword review cantonese --under food
  reword review --typed --no-new    typed answers, no new cards
  reword edit cantonese             open the deck in $EDITOR

Deck files are plain text, one card per line: `front::back`. A `:::` line is
asked in both directions. Edit them freely; history is keyed on the front.";

#[derive(Parser, Debug)]
#[command(
    name = "reword",
    version,
    about = "Plain-text flashcards with FSRS spaced repetition",
    after_help = AFTER_HELP,
    args_conflicts_with_subcommands = false
)]
pub struct Cli {
    /// Data directory [default: ~/reword]
    #[arg(long, global = true, value_name = "PATH", env = "REWORD_DIR")]
    pub dir: Option<PathBuf>,

    /// Machine-readable output where a command prints data
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress warnings and confirmations
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Disable colors (NO_COLOR=1 does the same)
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Never prompt; fail with a hint instead
    #[arg(long, global = true)]
    pub no_input: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create the data directory with an example deck
    Init,
    /// Start a review session
    Review(ReviewArgs),
    /// Append a card to a deck
    Add(AddArgs),
    /// Open a deck file in $EDITOR, then check it
    Edit {
        /// Deck name (the only deck is used if there is just one)
        deck: Option<String>,
    },
    /// List decks with card and due counts
    Decks,
    /// Change a card's front and carry its history along
    Rename {
        deck: String,
        /// Current front, as written in the deck or the log
        old: String,
        /// New front
        new: String,
    },
    /// Validate decks and logs: duplicates, orphaned histories, malformed lines
    Check,
    /// Retention, reviews per day, and a 14-day forecast
    Stats {
        /// Decks to include (default: all)
        decks: Vec<String>,
    },
    /// Fit FSRS parameters to your history and write params.toml
    Optimize,
    /// Print shell completions (bash, zsh, fish, elvish, powershell)
    Completions { shell: Shell },
}

#[derive(Args, Debug)]
pub struct ReviewArgs {
    /// Decks to review (default: all)
    pub decks: Vec<String>,

    /// Typed session: type each answer, checked automatically
    #[arg(long, conflicts_with = "recall")]
    pub typed: bool,

    /// Recall session: reveal the answer, then grade yourself
    #[arg(long)]
    pub recall: bool,

    /// Time budget in minutes
    #[arg(short = 'm', long, value_name = "N")]
    pub minutes: Option<u32>,

    /// Stop after this many cards (graded or skipped)
    #[arg(short = 'n', long, value_name = "N")]
    pub cards: Option<usize>,

    /// Only cards under this Markdown heading
    #[arg(long, value_name = "HEADING")]
    pub under: Option<String>,

    /// New cards to introduce this session, ignoring the daily cap
    #[arg(long, value_name = "N", conflicts_with = "no_new")]
    pub new: Option<usize>,

    /// No new cards this session
    #[arg(long)]
    pub no_new: bool,

    /// Keep going until you quit: no time limit, no daily cap on new
    /// cards, and once nothing is due, the cards closest to being forgotten
    #[arg(long, conflicts_with_all = ["minutes", "cards"])]
    pub endless: bool,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Deck name; created if it does not exist (asks first)
    pub deck: Option<String>,
    /// Front of the card
    pub front: Option<String>,
    /// Back of the card; " / " separates alternatives for typed mode
    pub back: Option<String>,
    /// Also ask back → front (writes a `:::` line)
    #[arg(short, long)]
    pub reverse: bool,
}
