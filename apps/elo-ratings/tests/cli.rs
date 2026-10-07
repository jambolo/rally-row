use std::process::{Command, Output};
use test_support::{
    CONFIG_DIR,
    cli::{assert_contains, assert_requires_league},
};

const BIN: &str = env!("CARGO_BIN_EXE_elo-ratings");

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

#[test]
fn cli_requires_league() {
    assert_requires_league(BIN);
}

#[test]
fn cli_rejects_unsafe_league_ids() {
    let output = run(&["--league", "NFL/../x"]);
    assert!(!output.status.success());
    assert_contains(&String::from_utf8_lossy(&output.stderr), "Unsafe league id");
}

#[test]
fn cli_reads_the_league_config_from_config_dir() {
    let data_dir = concat!(env!("CARGO_TARGET_TMPDIR"), "/elo-ratings-cli-without-history");
    let output = run(&["--league", "nfl", "--config-dir", CONFIG_DIR, "--data-dir", data_dir]);
    assert!(!output.status.success());
    assert_contains(
        &String::from_utf8_lossy(&output.stderr),
        "Read history.json; run history-importer first",
    );
}
