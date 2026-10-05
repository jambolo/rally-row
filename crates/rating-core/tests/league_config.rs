use rating_core::load_league_config;
use std::{fs, path::Path};
use test_support::CONFIG_DIR;

#[test]
fn loads_the_league_file_from_the_config_dir() {
    let (cfg, bytes) = load_league_config(Path::new(CONFIG_DIR), "nfl").unwrap();
    assert_eq!(cfg.id, "nfl");
    assert_eq!(bytes, fs::read(Path::new(CONFIG_DIR).join("nfl.json")).unwrap());
}

#[test]
fn rejects_unsafe_league_ids_before_reading_files() {
    let missing = Path::new("no-such-config-dir");
    for league in ["", "NFL/../x", "../nfl", "nfl.json", "n f l", "nfl\\x"] {
        let error = load_league_config(missing, league).unwrap_err();
        assert_eq!(error.to_string(), "Unsafe league id", "{league:?}");
    }
}

#[test]
fn rejects_a_config_whose_id_differs_from_the_league() {
    let dir = tempfile::tempdir().unwrap();
    fs::copy(Path::new(CONFIG_DIR).join("nfl.json"), dir.path().join("other.json")).unwrap();
    let error = load_league_config(dir.path(), "other").unwrap_err().to_string();
    assert_eq!(error, "Config id nfl does not match requested league other");
}

#[test]
fn reports_a_missing_league_file() {
    let dir = tempfile::tempdir().unwrap();
    let error = format!("{:#}", load_league_config(dir.path(), "absent").unwrap_err());
    assert!(error.contains("Read config") && error.contains("absent.json"), "{error}");
}
