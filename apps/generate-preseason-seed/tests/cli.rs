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
fn cli_saves_the_replay_audit_apart_from_the_seed() {
    let tool = ToolDir::new(BIN, &[]);
    let (cfg, history) = evaluation_fixture();
    let config_bytes = tool.write_config(&cfg);
    let history_bytes = tool.write_history(&history);
    let season = history.through_season + 1;
    let output = tool.run(&["--target-season", &season.to_string()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_contains(&stdout, &format!("elo-{season}.json, and their replay to "));
    assert_contains(&stdout, &format!("elo-audit-{season}.json"));
    let read =
        |name: String| -> Value { serde_json::from_slice(&fs::read(tool.path().join("data/nfl").join(name)).unwrap()).unwrap() };
    let seed = read(format!("elo-{season}.json"));
    let audit = read(format!("elo-audit-{season}.json"));
    assert!(seed.get("audit").is_none());
    assert_eq!(audit["schema_version"], 1);
    for field in [
        "league",
        "target_season",
        "through_season",
        "generated_at",
        "history_sha256",
        "config_sha256",
    ] {
        assert_eq!(audit[field], seed[field], "{field}");
    }
    assert_eq!(audit["config_sha256"], digest(&config_bytes));
    assert_eq!(audit["history_sha256"], digest(&history_bytes));
    let games = audit["games"].as_array().unwrap();
    assert_eq!(games.len(), seed["completed_games"].as_u64().unwrap() as usize);
    assert_eq!(games[0]["game_id"], history.games[0].id.as_str());
}
