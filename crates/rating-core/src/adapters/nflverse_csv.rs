//! nflverse `games.csv`: one whole-history file, NFL game-type codes, Eastern local times.

use super::SourceAdapter;
use crate::{Game, LeagueConfig, Outcome, SourceGame};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

pub struct NflverseCsv;

#[derive(Deserialize)]
struct CsvGame {
    game_id: String,
    season: i32,
    game_type: String,
    week: u32,
    #[serde(default)]
    gameday: String,
    #[serde(default)]
    gametime: Option<String>,
    away_team: String,
    home_team: String,
    away_score: Option<u32>,
    home_score: Option<u32>,
    location: String,
}

impl SourceAdapter for NflverseCsv {
    fn kind(&self) -> &'static str {
        "nflverse-csv"
    }
    fn version(&self) -> u32 {
        1
    }
    fn history_urls(&self, cfg: &LeagueConfig, _from_season: i32, _through_season: i32) -> Vec<String> {
        vec![cfg.source.url.clone()]
    }
    fn parse(&self, documents: &[String], cfg: &LeagueConfig, warn: &mut dyn FnMut(String)) -> Result<Vec<Game>> {
        let [text] = documents else {
            bail!(
                "The nflverse-csv adapter expects exactly one document, got {}",
                documents.len()
            );
        };
        let mut games = Vec::new();
        let mut reader = csv::Reader::from_reader(text.as_bytes());
        for row in reader.deserialize::<CsvGame>() {
            let r = row.context("Malformed nflverse CSV row")?;
            if r.season < cfg.history_start {
                continue;
            }
            let phase = match r.game_type.as_str() {
                "REG" => "regular",
                "WC" | "DIV" | "CON" | "SB" => "postseason",
                "PRE" | "PRO" | "PB" => continue,
                other => bail!("Unknown game type: {other}"),
            };
            let result = match (r.home_score, r.away_score) {
                (None, None) => None,
                (Some(h), Some(a)) => Some(if h > a {
                    Outcome::HomeWin
                } else if h < a {
                    Outcome::AwayWin
                } else {
                    Outcome::Tie
                }),
                _ => bail!("Only one score present for {}", r.game_id),
            };
            ensure!(
                ["Home", "Neutral"].contains(&r.location.as_str()),
                "Unknown location for {}",
                r.game_id
            );
            games.push(SourceGame {
                id: r.game_id,
                league: cfg.id.clone(),
                season: r.season,
                date: r.gameday,
                time: r.gametime,
                timezone: "America/New_York".into(),
                phase: phase.into(),
                round_label: r.game_type,
                round: r.week,
                home_team: cfg.team_id(&r.home_team),
                away_team: cfg.team_id(&r.away_team),
                home_source_id: r.home_team,
                away_source_id: r.away_team,
                neutral: r.location == "Neutral",
                result,
            });
        }
        games.into_iter().map(|g| g.normalize(warn)).collect()
    }
    fn strict_source_ids(&self) -> bool {
        true
    }
    fn season_incomplete(&self, games: &[Game], season: i32) -> Option<String> {
        let complete = games
            .iter()
            .any(|g| g.season == season && g.round_label == "SB" && g.result.is_some());
        if complete {
            None
        } else {
            Some("no completed Super Bowl".into())
        }
    }
}
