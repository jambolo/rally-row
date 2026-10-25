//! Source adapters: every source-kind-specific rule lives behind this registry.

use crate::{Game, LeagueConfig};
use anyhow::{Context, Result};

mod canonical_json;
mod mlb_statsapi;
mod nflverse_csv;

pub trait SourceAdapter: Sync {
    /// Registry key; equals `LeagueConfig.source.kind`.
    fn kind(&self) -> &'static str;
    /// Parser revision recorded in `history.json`. Bump it whenever a change to `parse` changes the games it produces, so
    /// saved histories are downloaded again instead of kept.
    fn version(&self) -> u32;
    /// URLs whose bodies, in order, cover seasons `from_season..=through_season`.
    /// Whole-file sources return `vec![cfg.source.url.clone()]`; per-season sources return one URL per season.
    fn history_urls(&self, cfg: &LeagueConfig, from_season: i32, through_season: i32) -> Vec<String>;
    /// Parse downloaded (or `--input`) documents into normalized but unvalidated games.
    /// Finality/dedup/game-type mapping are adapter concerns; unfinished games get `result: None`.
    fn parse(&self, documents: &[String], cfg: &LeagueConfig, warn: &mut dyn FnMut(String)) -> Result<Vec<Game>>;
    /// Whether `validate_games` requires each source id to be listed in its franchise era's `source_ids`.
    fn strict_source_ids(&self) -> bool;
    /// `Some(reason)` when `season` lacks the league's completion marker in `games`, e.g. "no completed Super Bowl".
    fn season_incomplete(&self, games: &[Game], season: i32) -> Option<String>;
    /// Adapter-specific configuration checks; called by `LeagueConfig::validate`.
    fn validate_config(&self, _cfg: &LeagueConfig) -> Result<()> {
        Ok(())
    }
}

static ADAPTERS: &[&dyn SourceAdapter] = &[
    &nflverse_csv::NflverseCsv,
    &canonical_json::CanonicalJson,
    &mlb_statsapi::MlbStatsApi,
];

/// Registry lookup; error text exactly "Unknown source adapter".
pub fn adapter_for(kind: &str) -> Result<&'static dyn SourceAdapter> {
    ADAPTERS
        .iter()
        .copied()
        .find(|adapter| adapter.kind() == kind)
        .context("Unknown source adapter")
}
