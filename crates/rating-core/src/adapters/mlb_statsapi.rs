//! MLB Stats API schedule responses: one document per season, finality from the status text,
//! one game per `gamePk`, and World Series completeness.

use super::SourceAdapter;
use crate::{Game, LeagueConfig, Outcome};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{cmp::Ordering, collections::BTreeMap};

pub struct MlbStatsApi;

#[derive(Deserialize)]
struct Schedule {
    #[serde(default)]
    dates: Vec<ScheduleDate>,
}

#[derive(Deserialize)]
struct ScheduleDate {
    #[serde(default)]
    games: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    game_pk: u64,
    game_type: String,
    season: String,
    game_date: String,
    status: Status,
    teams: Teams,
    is_tie: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    detailed_state: String,
}

#[derive(Deserialize)]
struct Teams {
    home: Side,
    away: Side,
}

#[derive(Deserialize)]
struct Side {
    team: TeamRef,
    score: Option<u32>,
}

#[derive(Deserialize)]
struct TeamRef {
    id: u32,
}

/// One schedule listing that survived the status filter.
struct Listing {
    game_pk: u64,
    game_type: String,
    phase: &'static str,
    round: u32,
    season: i32,
    start: DateTime<Utc>,
    home: u32,
    away: u32,
    /// `(home score, away score, isTie == Some(true))` for final listings.
    final_score: Option<(u32, u32, bool)>,
}

/// `(phase, round)` for a game-type code; `None` for excluded types (spring, exhibition, All-Star).
fn phase_and_round(code: &str) -> Result<Option<(&'static str, u32)>> {
    Ok(Some(match code {
        "R" => ("regular", 1),
        "F" => ("postseason", 2),
        "D" => ("postseason", 3),
        "L" => ("postseason", 4),
        "W" => ("postseason", 5),
        "S" | "E" | "A" => return Ok(None),
        other => bail!("Unknown game type: {other}"),
    }))
}

/// Applies the per-entry rules; `None` drops the entry (excluded type, early season, not a game, or unusable final).
fn listing(entry: Entry, cfg: &LeagueConfig, warn: &mut dyn FnMut(String)) -> Result<Option<Listing>> {
    let game_pk = entry.game_pk;
    let Some((phase, round)) = phase_and_round(&entry.game_type)? else {
        return Ok(None);
    };
    let Ok(season) = entry.season.parse::<i32>() else {
        bail!("Invalid season {:?} for game {game_pk}", entry.season);
    };
    if season < cfg.history_start {
        return Ok(None);
    }
    // `abstractGameState` is "Final" for postponed and cancelled listings, so only the detailed text decides.
    let base = entry.status.detailed_state.split(':').next().unwrap_or_default().trim();
    let final_score = match base {
        "Postponed" | "Cancelled" | "Suspended" => return Ok(None),
        "Final" | "Completed Early" => {
            let (Some(home), Some(away)) = (entry.teams.home.score, entry.teams.away.score) else {
                warn(format!("Game {game_pk}: final status without scores; ignored"));
                return Ok(None);
            };
            Some((home, away, entry.is_tie == Some(true)))
        }
        _ => None,
    };
    let start = DateTime::parse_from_rfc3339(&entry.game_date)
        .with_context(|| format!("Invalid gameDate {:?} for game {game_pk}", entry.game_date))?
        .with_timezone(&Utc);
    Ok(Some(Listing {
        game_pk,
        game_type: entry.game_type,
        phase,
        round,
        season,
        start,
        home: entry.teams.home.team.id,
        away: entry.teams.away.team.id,
        final_score,
    }))
}

/// Collapses every listing of one `gamePk` into a single game.
fn game(listings: &[Listing], cfg: &LeagueConfig) -> Result<Game> {
    let first = &listings[0];
    let game_pk = first.game_pk;
    ensure!(
        listings
            .iter()
            .all(|l| l.game_type == first.game_type && l.season == first.season && l.home == first.home && l.away == first.away),
        "Inconsistent entries for game {game_pk}"
    );
    let finals: Vec<&Listing> = listings.iter().filter(|l| l.final_score.is_some()).collect();
    ensure!(
        finals.iter().all(|l| l.final_score == finals[0].final_score),
        "Conflicting final results for game {game_pk}"
    );
    // `max_by_key` keeps the last of equal maxima, i.e. the later listing in input order.
    let latest = if finals.is_empty() {
        listings.iter().max_by_key(|l| l.start)
    } else {
        finals.into_iter().max_by_key(|l| l.start)
    }
    .expect("a gamePk group is never empty");
    let result = match latest.final_score {
        None => None,
        Some((home, away, is_tie)) => Some(match (is_tie, home.cmp(&away)) {
            (true, Ordering::Equal) => Outcome::Tie,
            (false, Ordering::Greater) => Outcome::HomeWin,
            (false, Ordering::Less) => Outcome::AwayWin,
            _ => bail!("Inconsistent tie flag for game {game_pk}"),
        }),
    };
    let (home_source_id, away_source_id) = (latest.home.to_string(), latest.away.to_string());
    Ok(Game {
        id: game_pk.to_string(),
        league: cfg.id.clone(),
        season: latest.season,
        start_time_utc: latest.start,
        phase: latest.phase.into(),
        round_label: latest.game_type.clone(),
        round: latest.round,
        home_team: cfg.team_id(&home_source_id),
        away_team: cfg.team_id(&away_source_id),
        home_source_id,
        away_source_id,
        // Special venues, reversed home/away listings, and relocated games all stay home games.
        neutral: false,
        result,
    })
}

impl SourceAdapter for MlbStatsApi {
    fn kind(&self) -> &'static str {
        "mlb-statsapi"
    }
    fn version(&self) -> u32 {
        1
    }
    fn history_urls(&self, cfg: &LeagueConfig, from_season: i32, through_season: i32) -> Vec<String> {
        (from_season..=through_season)
            .map(|season| cfg.source.url.replace("{season}", &season.to_string()))
            .collect()
    }
    fn parse(&self, documents: &[String], cfg: &LeagueConfig, warn: &mut dyn FnMut(String)) -> Result<Vec<Game>> {
        let mut groups: BTreeMap<u64, Vec<Listing>> = BTreeMap::new();
        for text in documents {
            let schedule: Schedule = serde_json::from_str(text).context("Malformed MLB Stats API schedule document")?;
            for entry in schedule.dates.into_iter().flat_map(|d| d.games) {
                if let Some(l) = listing(entry, cfg, warn)? {
                    groups.entry(l.game_pk).or_default().push(l);
                }
            }
        }
        groups.values().map(|listings| game(listings, cfg)).collect()
    }
    fn strict_source_ids(&self) -> bool {
        true
    }
    fn season_incomplete(&self, games: &[Game], season: i32) -> Option<String> {
        let complete = games
            .iter()
            .any(|g| g.season == season && g.round_label == "W" && g.result.is_some());
        if complete {
            None
        } else {
            Some("no completed World Series".into())
        }
    }
    fn validate_config(&self, cfg: &LeagueConfig) -> Result<()> {
        ensure!(
            cfg.source.url.contains("{season}"),
            "Source URL needs a {{season}} placeholder"
        );
        // Numeric source ids are not display text, so the first one cannot stand in for an abbreviation.
        ensure!(
            cfg.teams.iter().flat_map(|t| &t.eras).all(|e| e.abbreviation.is_some()),
            "Every team era needs an abbreviation"
        );
        Ok(())
    }
}
