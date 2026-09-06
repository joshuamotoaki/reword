//! Tab completion: print a snippet, install it, or enable it on first use.

use std::path::{Path, PathBuf};

use clap_complete::Shell;
use clap_complete::env::Shells;

use crate::error::{Error, Result};
use crate::out;
use crate::term::Term;

/// Print a script you can `source`, or write that hook into the shell config.
pub fn run(shell: Option<Shell>, install: bool) -> Result<i32> {
    let shell = match shell {
        Some(s) => s,
        None => detect_shell()?,
    };
    if install {
        match install_hook(shell)? {
            Outcome::Installed => match shell {
                Shell::Fish => {
                    out::println(&format!("Wrote {}", dest_label(shell)?));
                    out::println("Open a new shell, or run: exec $SHELL");
                }
                _ => {
                    out::println(&format!("Appended to {}.", dest_label(shell)?));
                    out::println(&format!(
                        "Reload your shell, or run: source {}",
                        rc_or_file(shell)?.display()
                    ));
                }
            },
            Outcome::Already => {
                out::println(&format!(
                    "Completions are already in {}.",
                    dest_label(shell)?
                ));
            }
        }
        Ok(0)
    } else {
        print!("{}", registration(shell)?);
        Ok(0)
    }
}

/// Wire up tab completion for this shell the first time `reword` is used
/// interactively. Scripts, `--no-input`, and `--quiet` leave the config alone.
pub fn ensure(term: &Term) {
    if !term.interactive {
        return;
    }
    let Ok(shell) = detect_shell() else {
        return;
    };
    if matches!(install_hook(shell), Ok(Outcome::Installed)) {
        out::note(
            term,
            "Tab completion enabled. Open a new shell, or source your shell config.",
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Installed,
    Already,
}

fn registration(shell: Shell) -> Result<String> {
    let name = shell.to_string();
    let shells = Shells::builtins();
    let completer = shells.completer(&name).ok_or_else(|| {
        Error::new(format!("no completion script for {name}"))
            .hint("Supported: bash, zsh, fish, elvish, powershell.")
    })?;
    let mut buf = Vec::new();
    completer
        .write_registration("COMPLETE", "reword", "reword", "reword", &mut buf)
        .map_err(Error::from)?;
    let script = String::from_utf8(buf).map_err(|e| Error::new(e.to_string()))?;
    if shell == Shell::Zsh {
        Ok(format!(
            "(( $+functions[compdef] )) || {{ autoload -Uz compinit && compinit }}\n{script}"
        ))
    } else {
        Ok(script)
    }
}

fn detect_shell() -> Result<Shell> {
    let path = std::env::var("SHELL").unwrap_or_default();
    let name = Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let name = if name == "pwsh" { "powershell" } else { name };
    name.parse().map_err(|_| {
        Error::new("could not tell which shell you use")
            .hint("Pass one: reword completions zsh --install")
    })
}

fn install_hook(shell: Shell) -> Result<Outcome> {
    match shell {
        Shell::Fish => write_if_changed(&fish_path()?, &registration(shell)?),
        _ => append_rc(&rc_path(shell)?, &hook_line(shell)),
    }
}

fn hook_line(shell: Shell) -> String {
    match shell {
        Shell::Zsh => "source <(reword completions zsh)\n".into(),
        Shell::Bash => "eval \"$(reword completions bash)\"\n".into(),
        Shell::Elvish => "eval (reword completions elvish | slurp)\n".into(),
        Shell::PowerShell => "Invoke-Expression (& reword completions powershell)\n".into(),
        Shell::Fish => String::new(),
        _ => String::new(),
    }
}

fn rc_path(shell: Shell) -> Result<PathBuf> {
    let home = home()?;
    Ok(match shell {
        Shell::Zsh => std::env::var_os("ZDOTDIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or(home)
            .join(".zshrc"),
        Shell::Bash => home.join(".bashrc"),
        Shell::Elvish => home.join(".config/elvish/rc.elv"),
        Shell::PowerShell => home.join(".config/powershell/Microsoft.PowerShell_profile.ps1"),
        Shell::Fish => fish_path()?,
        _ => {
            return Err(Error::new(format!(
                "do not know where {shell} keeps its config"
            )));
        }
    })
}

fn rc_or_file(shell: Shell) -> Result<PathBuf> {
    match shell {
        Shell::Fish => fish_path(),
        _ => rc_path(shell),
    }
}

fn dest_label(shell: Shell) -> Result<String> {
    Ok(rc_or_file(shell)?.display().to_string())
}

fn fish_path() -> Result<PathBuf> {
    Ok(home()?.join(".config/fish/completions/reword.fish"))
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(std::env::home_dir)
        .ok_or_else(|| Error::new("HOME is not set"))
}

fn write_if_changed(path: &Path, contents: &str) -> Result<Outcome> {
    if path.is_file() && std::fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return Ok(Outcome::Already);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(Outcome::Installed)
}

fn append_rc(path: &Path, line: &str) -> Result<Outcome> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    if current.contains("reword completions") {
        return Ok(Outcome::Already);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = current;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str("# reword tab completion\n");
    text.push_str(line);
    std::fs::write(path, text)?;
    Ok(Outcome::Installed)
}
