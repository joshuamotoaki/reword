mod cli;
mod clock;
mod commands;
mod config;
mod deck;
mod error;
mod history;
mod memory;
mod out;
mod planner;
mod session;
mod store;
mod term;
mod text;
mod types;
mod viz;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::commands::Ctx;

fn main() {
    let cli = Cli::parse();
    let term = term::Term::detect(cli.no_color, cli.no_input, cli.quiet);
    let ctx = Ctx {
        store: store::Store::locate(cli.dir.as_deref()),
        term,
        clock: clock::Clock::system(),
        json: cli.json,
    };

    let result = match cli.command {
        None => commands::status::run(&ctx),
        Some(Command::Init) => commands::init::run(&ctx),
        Some(Command::Review(args)) => commands::review::run(&ctx, args),
        Some(Command::Add(args)) => commands::add::run(&ctx, args),
        Some(Command::Edit { deck }) => commands::edit::run(&ctx, deck.as_deref()),
        Some(Command::Decks) => commands::decks::run(&ctx),
        Some(Command::Rename { deck, old, new }) => commands::rename::run(&ctx, &deck, &old, &new),
        Some(Command::Check) => commands::check::run(&ctx),
        Some(Command::Stats { decks }) => commands::stats::run(&ctx, &decks),
        Some(Command::Optimize) => commands::optimize::run(&ctx),
        Some(Command::Completions { shell }) => commands::completions::run(shell),
    };

    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("{} {}", term.err.red("error:"), e.message);
            if let Some(hint) = e.hint {
                eprintln!("  {hint}");
            }
            std::process::exit(1);
        }
    }
}
