//! Canonical JSON envelope `{schema_version: 2, league, games: [SourceGame]}` for any league.

use super::SourceAdapter;
use crate::{Game, LeagueConfig, SourceGame};
use anyhow::{Result, bail, ensure};
use serde::Deserialize;

pub struct CanonicalJson;

#[derive(Deserialize)]
struct Input {
    schema_version: u32,
    league: String,
    games: Vec<SourceGame>,
}

impl SourceAdapter for CanonicalJson {
    fn kind(&self) -> &'static str {
        "canonical-json"
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
                "The canonical-json adapter expects exactly one document, got {}",
                documents.len()
            );
        };
        let input: Input = serde_json::from_str(text)?;
        ensure!(
            input.schema_version == 2 && input.league == cfg.id,
            "Incompatible source envelope; canonical JSON requires schema version 2"
        );
        let mut games = input.games;
        for g in &mut games {
            g.home_team = cfg.team_id(&g.home_team);
            g.away_team = cfg.team_id(&g.away_team);
        }
        games.into_iter().map(|g| g.normalize(warn)).collect()
    }
    fn strict_source_ids(&self) -> bool {
        false
    }
    fn season_incomplete(&self, _games: &[Game], _season: i32) -> Option<String> {
        None
    }
}
