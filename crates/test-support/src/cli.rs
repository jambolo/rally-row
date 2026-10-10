//! Running a command-line tool against fixture files, and the checks every tool's tests share.

use chrono::{DateTime, Utc};
use rating_core::{EloTuneSettings, GameFile, HISTORY_SCHEMA_VERSION, LeagueConfig, digest};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

/// Asserts that `text` contains `expected`, showing `text` when it does not.
pub fn assert_contains(text: &str, expected: &str) {
    assert!(text.contains(expected), "expected {expected:?} in:\n{text}");
}

/// Clap rejects a run without `--league`, exiting with status 2.
pub fn assert_requires_league(bin: &str) {
    let output = Command::new(bin).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_contains(&String::from_utf8_lossy(&output.stderr), "--league <LEAGUE>");
}

/// A temporary working directory for a tool run as `<bin> --league nfl --config-dir . --data-dir data`.
pub struct ToolDir {
    bin: &'static str,
    json_args: &'static [&'static str],
    dir: TempDir,
}

impl ToolDir {
    /// `json_args` are the flags that make the tool print its JSON report, if it needs any.
    pub fn new(bin: &'static str, json_args: &'static [&'static str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("data/nfl")).unwrap();
        Self { bin, json_args, dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Writes `nfl.json` and returns its bytes.
    pub fn write_config(&self, cfg: &LeagueConfig) -> Vec<u8> {
        let bytes = serde_json::to_vec(cfg).unwrap();
        fs::write(self.path().join("nfl.json"), &bytes).unwrap();
        bytes
    }

    /// Writes `data/nfl/history.json` and returns its bytes.
    pub fn write_history(&self, history: &impl Serialize) -> Vec<u8> {
        let bytes = serde_json::to_vec(history).unwrap();
        fs::write(self.path().join("data/nfl/history.json"), &bytes).unwrap();
        bytes
    }

    pub fn run(&self, args: &[&str]) -> Output {
        Command::new(self.bin)
            .current_dir(self.path())
            .args(["--league", "nfl", "--config-dir", ".", "--data-dir", "data"])
            .args(args)
            .output()
            .unwrap()
    }

    /// Runs with the JSON flags, requires success, and parses the report printed to stdout.
    pub fn report(&self, args: &[&str]) -> Value {
        let output = self.run(&[self.json_args, args].concat());
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// Runs, requires failure, and returns stderr.
    pub fn error(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// Runs with the JSON flags and `--report-dir reports`, and checks that the report has a UTC `run_at` no earlier
    /// than `started` and is saved as `reports/<report_name>-nfl-<UTC date>.json` with the printed content. Returns the
    /// report and stderr.
    fn saved_report(&self, report_name: &str, started: DateTime<Utc>) -> (Value, String) {
        let output = self.run(&[self.json_args, &["--report-dir", "reports"]].concat());
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{stderr}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let run_at = run_at(&report);
        assert!(run_at >= started && run_at <= Utc::now());
        let path = self
            .path()
            .join("reports")
            .join(format!("{report_name}-nfl-{}.json", run_at.format("%Y-%m-%d")));
        let saved: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved, report);
        (report, stderr)
    }
}

/// The report's `run_at`, which must be written in UTC.
fn run_at(report: &Value) -> DateTime<Utc> {
    let run_at = DateTime::parse_from_rfc3339(report["run_at"].as_str().unwrap()).unwrap();
    assert_eq!(run_at.offset().local_minus_utc(), 0);
    run_at.with_timezone(&Utc)
}

/// The report without its `run_at`, for comparing runs.
pub fn without_run_at(report: &Value) -> Value {
    let mut report = report.clone();
    report.as_object_mut().unwrap().remove("run_at");
    report
}

/// Runs the tool twice on `cfg` and `history`, saving reports, and checks what every report-writing tool promises:
/// each report is saved under `<report_name>` with the printed content and a UTC `run_at`, it carries the input hashes,
/// the runs agree apart from `run_at`, and the inputs and an existing seed stay untouched. Returns the first report
/// and its stderr.
pub fn assert_read_only_and_repeatable(
    tool: &ToolDir,
    report_name: &str,
    cfg: &LeagueConfig,
    history: &GameFile,
) -> (Value, String) {
    let config_bytes = tool.write_config(cfg);
    let history_bytes = tool.write_history(history);
    let seed = tool.path().join(format!("data/nfl/elo-{}.json", history.through_season + 1));
    fs::write(&seed, "untouched seed").unwrap();
    let (report, stderr) = tool.saved_report(report_name, Utc::now());
    assert_eq!(report["config_sha256"], digest(&config_bytes));
    assert_eq!(report["history_sha256"], digest(&history_bytes));
    let (repeated, _) = tool.saved_report(report_name, run_at(&report));
    assert_eq!(without_run_at(&report), without_run_at(&repeated));
    assert_eq!(fs::read(tool.path().join("nfl.json")).unwrap(), config_bytes);
    assert_eq!(fs::read(tool.path().join("data/nfl/history.json")).unwrap(), history_bytes);
    assert_eq!(fs::read_to_string(seed).unwrap(), "untouched seed");
    assert_eq!(fs::read_dir(tool.path().join("data/nfl")).unwrap().count(), 2);
    (report, stderr)
}

/// `--tune-start`, `--tune-end` and `--test-end` set the split whether or not the config has an `elo_tune` split, and
/// with neither a config split nor the flags, the error names `elo_tune.tune_start`. `cfg` has no `bayes_tune` split.
pub fn assert_split_flags_override_config(tool: &ToolDir, mut cfg: LeagueConfig, history: &GameFile) {
    tool.write_history(history);
    let test_end = history.through_season.to_string();
    let configured = EloTuneSettings {
        tune_start: 2010,
        tune_end: 2022,
        test_end: 2025,
    };
    for defaults in [Some(configured), None] {
        cfg.elo_tune = defaults;
        tool.write_config(&cfg);
        if cfg.elo_tune.is_none() {
            assert_contains(&tool.error(&[]), "elo_tune.tune_start");
        }
        let report = tool.report(&["--tune-start", "2003", "--tune-end", "2004", "--test-end", &test_end]);
        assert_eq!(
            report["split"],
            json!({"warmup_start": history.from_season, "tune_start": 2003, "tune_end": 2004, "test_end": history.through_season})
        );
    }
}

/// Rejects history in an old schema or without `start_time_utc`, a split with no held-out season, a split past the end
/// of history, a missing tuning season, and an unreported game, each with an actionable error. `cfg` sets the
/// `elo_tune` split, and `history` ends with its last held-out season.
pub fn assert_rejects_unusable_history(tool: &ToolDir, cfg: &LeagueConfig, history: &GameFile) {
    let split = cfg.elo_tune.as_ref().unwrap();
    tool.write_config(cfg);
    let mut obsolete = serde_json::to_value(history).unwrap();
    for version in [1, 2] {
        obsolete["schema_version"] = json!(version);
        tool.write_history(&obsolete);
        assert_contains(&tool.error(&[]), "rerun import-history");
    }
    obsolete["schema_version"] = json!(HISTORY_SCHEMA_VERSION);
    obsolete["games"][0].as_object_mut().unwrap().remove("start_time_utc");
    tool.write_history(&obsolete);
    let error = tool.error(&[]);
    assert_contains(&error, "start_time_utc");
    assert_contains(&error, "rerun import-history");
    tool.write_history(history);
    assert_contains(&tool.error(&["--test-end", &split.tune_end.to_string()]), "held-out seasons");
    let past_history = (history.through_season + 1).to_string();
    assert_contains(&tool.error(&["--test-end", &past_history]), "History must cover");
    let test_end = history.through_season.to_string();
    let mut missing = history.clone();
    missing.games.retain(|g| g.season != split.tune_start);
    tool.write_history(&missing);
    assert_contains(
        &tool.error(&["--test-end", &test_end]),
        &format!("Missing completed historical season {}", split.tune_start),
    );
    let mut unfinished = history.clone();
    unfinished.games[0].result = None;
    tool.write_history(&unfinished);
    assert_contains(&tool.error(&["--test-end", &test_end]), "unreported game");
}

/// A tuner report's `search.evaluated_ranges` covers the baseline and starting grid, and `selected_on_boundary` lists
/// exactly the parameters, in `names` order, whose selected value equals an evaluated extreme; the boundary note
/// appears exactly when any do.
pub fn assert_boundary_report(report: &Value, names: [&str; 3]) {
    let search = &report["search"];
    let mut on_boundary = Vec::new();
    for name in names {
        let range = &search["evaluated_ranges"][name];
        let (lo, hi) = (range[0].as_f64().unwrap(), range[1].as_f64().unwrap());
        let selected = report["selected_parameters"][name].as_f64().unwrap();
        let baseline = report["baseline_parameters"][name].as_f64().unwrap();
        let grid = search["coarse_grid"][name]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap());
        for value in grid.chain([selected, baseline]) {
            assert!(lo <= value && value <= hi, "{name}: {value} outside [{lo}, {hi}]");
        }
        if selected == lo || selected == hi {
            on_boundary.push(name);
        }
    }
    assert_eq!(search["selected_on_boundary"], json!(on_boundary));
    let noted = report["notes"].as_array().unwrap().iter().any(|n| {
        n.as_str()
            .unwrap()
            .starts_with("Selected parameters lie on a searched boundary (")
    });
    assert_eq!(noted, !on_boundary.is_empty());
}
