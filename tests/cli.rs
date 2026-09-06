//! End-to-end tests against the built binary, each in its own data dir.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn reword(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_reword"))
        .args(args)
        .env("REWORD_DIR", dir)
        .env("NO_COLOR", "1")
        .env_remove("EDITOR")
        .env_remove("VISUAL")
        .output()
        .expect("run reword")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn fresh() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("reword");
    (tmp, dir)
}

#[test]
fn bare_invocation_without_data_dir_points_at_init() {
    let (_tmp, dir) = fresh();
    let o = reword(&dir, &[]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("reword init"));
    let o = reword(&dir, &["--json"]);
    assert!(stdout(&o).contains("\"initialized\": false"));
}

#[test]
fn commands_that_need_data_fail_with_a_hint() {
    let (_tmp, dir) = fresh();
    let o = reword(&dir, &["decks"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("error: no data directory"));
    assert!(stderr(&o).contains("reword init"));
}

#[test]
fn usage_errors_exit_two() {
    let (_tmp, dir) = fresh();
    let o = reword(&dir, &["reveiw"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("review"),
        "clap suggests the right command"
    );
}

#[test]
fn init_is_idempotent_and_creates_the_layout() {
    let (_tmp, dir) = fresh();
    let o = reword(&dir, &["init"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(dir.join("config.toml").is_file());
    assert!(dir.join("decks/example.md").is_file());
    assert_eq!(
        std::fs::read_to_string(dir.join(".gitattributes")).unwrap(),
        "*.log merge=union\n"
    );
    let o = reword(&dir, &["init"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("already set up"));
    let o = reword(&dir, &[]);
    assert!(stdout(&o).contains("example"));
    assert!(stdout(&o).contains("3 cards"));
}

#[test]
fn add_creates_decks_and_rejects_duplicates_without_input() {
    let (_tmp, dir) = fresh();
    reword(&dir, &["init"]);
    std::fs::remove_file(dir.join("decks/example.md")).unwrap();

    let o = reword(&dir, &["add", "cantonese", "食", "to eat"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "creating a deck asks, and there is no terminal"
    );
    assert!(stderr(&o).contains("no deck named"));

    std::fs::write(dir.join("decks/cantonese.md"), "# Cantonese\n").unwrap();
    let o = reword(&dir, &["add", "cantonese", "食", "to eat"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let o = reword(&dir, &["add", "飲", "to drink", "--reverse"]);
    assert!(o.status.success(), "only deck is used: {}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(dir.join("decks/cantonese.md")).unwrap(),
        "# Cantonese\n食::to eat\n飲:::to drink\n"
    );

    let o = reword(&dir, &["add", "cantonese", " 食 ", "again"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("already has \"食\" on line 2"));

    let o = reword(&dir, &["add", "cantonese", "a::b", "c"]);
    assert_eq!(o.status.code(), Some(1));

    let o = reword(&dir, &["add", "cantonese"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("no front given"));
}

#[test]
fn decks_check_and_status_report_counts_and_problems() {
    let (_tmp, dir) = fresh();
    reword(&dir, &["init"]);
    std::fs::remove_file(dir.join("decks/example.md")).unwrap();
    std::fs::write(dir.join("decks/a.md"), "x::1\ny:::2\nx::dup\n::empty\n").unwrap();
    std::fs::write(
        dir.join("decks/a.log"),
        "2026-09-01T10:00:00Z\tgone\tforward\trecall\tgood\t100\nbroken line\n",
    )
    .unwrap();
    std::fs::write(dir.join("decks/stray.log"), "").unwrap();

    let o = reword(&dir, &["decks", "--json"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["decks"][0]["name"], "a");
    assert_eq!(v["decks"][0]["cards"], 2);
    assert_eq!(v["decks"][0]["goals"], 3);
    assert_eq!(
        v["decks"][0]["new"], 2,
        "y's reverse side waits for its forward side"
    );
    assert_eq!(v["stray_logs"][0], "stray");
    assert!(stderr(&o).contains("decks/a.md:3: duplicate front \"x\""));
    assert!(stderr(&o).contains("decks/a.log:2:"));

    let o = reword(&dir, &["check"]);
    assert_eq!(o.status.code(), Some(1));
    let text = stdout(&o);
    assert!(text.contains("decks/a.md:3: duplicate"));
    assert!(text.contains("decks/a.md:4: card line has an empty front"));
    assert!(text.contains("decks/a.log:2:"));
    assert!(text.contains("history for \"gone\""));
    assert!(text.contains("decks/stray.log: no deck file"));
    assert!(text.contains("4 problems, 1 note"));

    let o = reword(&dir, &["--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["due"], 0);
    assert_eq!(v["new"], 2);
}

#[test]
fn rename_carries_history_and_rewrites_the_line() {
    let (_tmp, dir) = fresh();
    reword(&dir, &["init"]);
    std::fs::write(dir.join("decks/a.md"), "  old  :: 1\nother::2\n").unwrap();
    std::fs::write(
        dir.join("decks/a.log"),
        "2026-09-01T10:00:00Z\told\tforward\trecall\tgood\t100\n",
    )
    .unwrap();

    let o = reword(&dir, &["rename", "a", "old", "new"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("1 review carried over"));
    assert_eq!(
        std::fs::read_to_string(dir.join("decks/a.md")).unwrap(),
        "  new  :: 1\nother::2\n"
    );
    let log = std::fs::read_to_string(dir.join("decks/a.log")).unwrap();
    assert!(log.lines().last().unwrap().ends_with("\told\trename\tnew"));

    let o = reword(&dir, &["check"]);
    assert!(o.status.success(), "no orphan after rename: {}", stdout(&o));

    // Rename after the file was already edited: only the log row is needed.
    std::fs::write(dir.join("decks/a.md"), "newer::1\nother::2\n").unwrap();
    let o = reword(&dir, &["rename", "a", "new", "newer"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let o = reword(&dir, &["check"]);
    assert!(o.status.success(), "{}", stdout(&o));
    let o = reword(&dir, &["--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["decks"][0]["learned"], 1);
    assert_eq!(v["decks"][0]["due"], 1);

    let o = reword(&dir, &["rename", "a", "nothing", "x"]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn review_refuses_without_a_terminal_and_reports_nothing_due() {
    let (_tmp, dir) = fresh();
    reword(&dir, &["init"]);
    let o = reword(&dir, &["review"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("interactive terminal"));
    let o = reword(&dir, &["stats", "nope"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("no deck named \"nope\""));
    assert!(stderr(&o).contains("Decks: example"));
}

#[test]
fn stats_and_optimize_speak_plainly_with_little_history() {
    let (_tmp, dir) = fresh();
    reword(&dir, &["init"]);
    std::fs::write(dir.join("decks/example.log"), "2026-09-01T10:00:00Z\tfront\tforward\trecall\tgood\t100\n2026-09-03T10:00:00Z\tfront\tforward\trecall\tgood\t100\n").unwrap();
    let o = reword(&dir, &["stats", "--json"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["reviews"]["total"], 2);
    assert_eq!(v["learned"], 1);
    assert_eq!(v["fsrs_parameters"]["source"], "default");
    let o = reword(&dir, &["stats"]);
    assert!(stdout(&o).contains("Due in the next 14 days"));
    let o = reword(&dir, &["optimize"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("need 400"));
}

#[test]
fn config_errors_are_specific() {
    let (_tmp, dir) = fresh();
    reword(&dir, &["init"]);
    std::fs::write(dir.join("config.toml"), "session_minute = 5\n").unwrap();
    let o = reword(&dir, &["decks"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("config.toml"));
    assert!(stderr(&o).contains("session_minutes"));
}

#[test]
fn completions_and_help_work() {
    let (_tmp, dir) = fresh();
    let o = reword(&dir, &["completions", "zsh"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("_reword"));
    let o = reword(&dir, &["help", "review"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("--typed"));
    let o = reword(&dir, &["--version"]);
    assert!(stdout(&o).starts_with("reword "));
}

#[test]
fn edit_without_editor_names_the_file() {
    let (_tmp, dir) = fresh();
    reword(&dir, &["init"]);
    let o = reword(&dir, &["edit"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("$EDITOR"));
    assert!(stderr(&o).contains("example.md"));
}
