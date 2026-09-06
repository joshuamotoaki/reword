//! `reword completions SHELL`.

use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::Cli;
use crate::error::Result;

pub fn run(shell: Shell) -> Result<i32> {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "reword", &mut std::io::stdout());
    Ok(0)
}
