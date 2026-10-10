use rating_core::digest;
use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};
use test_support::{
    CONFIG_DIR,
    cli::{ToolDir, assert_contains, assert_requires_league},
    evaluation_fixture,
};

const BIN: &str = env!("CARGO_BIN_EXE_generate-preseason-seed");

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
    let data_dir = concat!(env!("CARGO_TARGET_TMPDIR"), "/generate-preseason-seed-cli-without-history");
    let output = run(&["--league", "nfl", "--config-dir", CONFIG_DIR, "--data-dir", data_dir]);
    assert!(!output.status.success());
    assert_contains(
        &String::from_utf8_lossy(&output.stderr),
        "Read history.json; run import-history first",
    );
}

#[test]
fn cli_saves_only_the_seed_with_its_input_hashes() {
    let tool = ToolDir::new(BIN, &[]);
    let (cfg, history) = evaluation_fixture();
    let config_bytes = tool.write_config(&cfg);
    let history_bytes = tool.write_history(&history);
    let season = history.through_season + 1;
    let output = tool.run(&["--target-season", &season.to_string()]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_contains(
        &String::from_utf8_lossy(&output.stdout),
        "from 25 completed games (5 ties) to ",
    );
    let dir = tool.path().join("data/nfl");
    let seed: Value = serde_json::from_slice(&fs::read(dir.join(format!("elo-{season}.json"))).unwrap()).unwrap();
    assert_eq!(seed["config_sha256"], digest(&config_bytes));
    assert_eq!(seed["history_sha256"], digest(&history_bytes));
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [format!("elo-{season}.json"), "elo.lock".into(), "history.json".into()]
    );
}
