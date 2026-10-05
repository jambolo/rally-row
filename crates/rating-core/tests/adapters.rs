use rating_core::{
    LeagueConfig, adapter_for, parse_documents, parse_source,
    tuning::{self, Split},
};
use test_support::{NFLVERSE_HEADER, history_file, league_config};

fn nflverse() -> LeagueConfig {
    league_config("nfl")
}

fn csv() -> String {
    format!(
        "{NFLVERSE_HEADER}a,2002,REG,1,2002-09-08,13:00,SF,10,SEA,20,Home\n\
         sb,2002,SB,21,2003-01-26,18:25,OAK,21,TB,48,Neutral\n\
         b,2003,REG,1,2003-09-07,13:00,SF,17,SEA,24,Home\n\
         sb2,2003,SB,21,2004-02-01,18:25,CAR,,NE,,Neutral\n"
    )
}

#[test]
fn unknown_kind_is_rejected() {
    let error = adapter_for("bogus").err().expect("bogus kind must be rejected");
    assert_eq!(error.to_string(), "Unknown source adapter");
}

#[test]
fn registry_resolves_both_kinds() {
    assert_eq!(adapter_for("nflverse-csv").unwrap().kind(), "nflverse-csv");
    assert_eq!(adapter_for("canonical-json").unwrap().kind(), "canonical-json");
    assert!(adapter_for("nflverse-csv").unwrap().strict_source_ids());
    assert!(!adapter_for("canonical-json").unwrap().strict_source_ids());
}

#[test]
fn history_urls_are_the_configured_source_url() {
    let cfg = nflverse();
    for kind in ["nflverse-csv", "canonical-json"] {
        assert_eq!(
            adapter_for(kind).unwrap().history_urls(&cfg, 2002, 2025),
            vec![cfg.source.url.clone()]
        );
    }
}

#[test]
fn nflverse_season_incomplete_requires_a_completed_super_bowl() {
    let games = parse_source(&csv(), &nflverse()).unwrap();
    let adapter = adapter_for("nflverse-csv").unwrap();
    assert_eq!(adapter.season_incomplete(&games, 2002), None);
    assert_eq!(
        adapter.season_incomplete(&games, 2003),
        Some("no completed Super Bowl".to_owned())
    );
    assert_eq!(
        adapter.season_incomplete(&games, 2004),
        Some("no completed Super Bowl".to_owned())
    );
}

#[test]
fn canonical_season_incomplete_is_always_none() {
    let games = parse_source(&csv(), &nflverse()).unwrap();
    let adapter = adapter_for("canonical-json").unwrap();
    for season in [2002, 2003, 2004] {
        assert_eq!(adapter.season_incomplete(&games, season), None);
    }
}

#[test]
fn parse_requires_exactly_one_document() {
    let cfg = nflverse();
    let mut canonical = nflverse();
    canonical.source.kind = "canonical-json".into();
    let envelope = r#"{"schema_version":2,"league":"nfl","games":[]}"#.to_owned();
    for (config, document) in [(&cfg, csv()), (&canonical, envelope)] {
        assert!(parse_documents(&[], config, |_| {}).is_err());
        assert!(parse_documents(&[document.clone(), document.clone()], config, |_| {}).is_err());
        assert!(parse_documents(&[document], config, |_| {}).is_ok());
    }
}

#[test]
fn parse_documents_matches_parse_source() {
    let cfg = nflverse();
    assert_eq!(
        parse_documents(&[csv()], &cfg, |_| {}).unwrap(),
        parse_source(&csv(), &cfg).unwrap()
    );
}

#[test]
fn tuning_requires_a_completed_super_bowl_per_season() {
    let mut cfg = nflverse();
    cfg.history_start = 2020;
    let rows: String = (2020..=2023)
        .map(|y| {
            format!(
                "r{y},{y},REG,1,{y}-09-13,13:00,SF,10,SEA,20,Home\nsb{y},{y},SB,21,{}-02-07,18:30,KC,9,TB,31,Neutral\n",
                y + 1
            )
        })
        .collect();
    let history = |csv: &str| {
        history_file(
            &cfg,
            2020..=2023,
            parse_source(&format!("{NFLVERSE_HEADER}{csv}"), &cfg).unwrap(),
        )
    };
    let split = Split {
        warmup_start: 2020,
        tune_start: 2021,
        tune_end: 2022,
        test_end: 2023,
    };
    tuning::validate(&history(&rows), &cfg, split).unwrap();
    let without = rows.replace("sb2022,2022,SB", "sb2022,2022,CON");
    assert_eq!(
        tuning::validate(&history(&without), &cfg, split).unwrap_err().to_string(),
        "Season 2022 has no completed Super Bowl; refresh history before tuning"
    );
}
