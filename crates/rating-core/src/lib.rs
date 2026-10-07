use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub mod adapters;
pub mod bayesian;
mod elo;
pub mod scoring;
mod time;
pub mod tuning;
pub mod walk_forward;
pub use adapters::{SourceAdapter, adapter_for};
pub use elo::{EloReplay, apply_game, expected_home, regress_rating, replay_elo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub kind: String,
    pub url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TeamEra {
    pub from_season: i32,
    pub through_season: Option<i32>,
    pub name: String,
    /// Franchise market/region; individual stadium moves are venue metadata.
    pub location: String,
    /// Display abbreviation; when absent, the first source id serves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abbreviation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub division: Option<String>,
    pub source_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub location: String,
    pub eras: Vec<TeamEra>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EloSettings {
    pub initial: f64,
    pub scale: f64,
    pub k: f64,
    pub home_advantage: f64,
    pub offseason_regression: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BayesianSettings {
    pub prior_sd_elo: f64,
    pub tie_prior_games: f64,
    pub tie_prior_rate: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuningSettings {
    pub tune_start: i32,
    pub tune_end: i32,
    pub test_end: i32,
}
pub use TuningSettings as EloTuneSettings;
/// Starting Elo search grid for `elo-tune`; each list is non-empty, finite, and strictly ascending.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EloGrid {
    pub k: Vec<f64>,
    pub home_advantage: Vec<f64>,
    pub offseason_regression: Vec<f64>,
}
/// Starting Bayesian search grid for `bayes-tune`; each list is non-empty, finite, and strictly ascending.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BayesianGrid {
    pub prior_sd_elo: Vec<f64>,
    pub tie_prior_games: Vec<f64>,
    pub tie_prior_rate: Vec<f64>,
}
/// Optional per-league tuner search grids; an absent grid means the tuner's default grid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TuningGrids {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elo: Option<EloGrid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bayesian: Option<BayesianGrid>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleUnit {
    Round,
    Date,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleFilter {
    pub unit: ScheduleUnit,
    pub label: String,
    pub all_label: String,
}
/// League display vocabulary; shared UI code renders only these strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisplayVocabulary {
    pub start_time_label: String,
    pub round_name: Option<String>,
    pub schedule_filter: ScheduleFilter,
    pub postseason_label: String,
    pub postseason_round_labels: BTreeMap<String, String>,
    pub postseason_tie_note: String,
}
/// Inclusive "MM-DD" range that may cross the year boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonthDayWindow {
    pub start: String,
    pub end: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LeagueWindows {
    pub season: MonthDayWindow,
    pub postseason: MonthDayWindow,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Division {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Conference {
    pub id: String,
    pub name: String,
    pub divisions: Vec<Division>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SeriesHome {
    HigherSeed,
    BetterRecord,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PostseasonRound {
    pub name: String,
    pub short: String,
    pub round_label: String,
    pub pattern: String,
    pub home: SeriesHome,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TiebreakKind {
    HeadToHead,
    HeadToHeadSweep,
    DivisionRecord,
    ConferenceRecord,
    CommonGames,
    StrengthOfVictory,
    StrengthOfSchedule,
    LastHalfConference,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TiebreakRule {
    pub rule: TiebreakKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_games: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tiebreakers {
    pub one_per_division: bool,
    pub division: Vec<TiebreakRule>,
    pub conference: Vec<TiebreakRule>,
}
/// The league's current playoff format; division membership lives on team eras.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PostseasonFormat {
    pub conferences: Vec<Conference>,
    pub teams_per_conference: u32,
    pub division_winners_first: bool,
    pub reseed: bool,
    pub rounds: Vec<PostseasonRound>,
    pub tiebreakers: Tiebreakers,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeagueConfig {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub history_start: i32,
    pub season_rollover_month: u32,
    pub source: Source,
    pub teams: Vec<Team>,
    pub aliases: BTreeMap<String, String>,
    pub ties_allowed_in: Vec<String>,
    pub display: DisplayVocabulary,
    pub windows: LeagueWindows,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postseason: Option<PostseasonFormat>,
    pub elo: EloSettings,
    pub bayesian: BayesianSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elo_tune: Option<EloTuneSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bayes_tune: Option<TuningSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tuning_grids: Option<TuningGrids>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    HomeWin,
    AwayWin,
    Tie,
}

impl Outcome {
    pub const ALL: [Outcome; 3] = [Outcome::HomeWin, Outcome::AwayWin, Outcome::Tie];

    /// Fractional home score: win 1, tie 1/2, loss 0.
    pub fn home_score(&self) -> f64 {
        match self {
            Outcome::HomeWin => 1.0,
            Outcome::AwayWin => 0.0,
            Outcome::Tie => 0.5,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Game {
    pub id: String,
    pub league: String,
    pub season: i32,
    /// Authoritative game start instant, converted by the source adapter before storage.
    pub start_time_utc: DateTime<Utc>,
    pub phase: String,
    pub round_label: String,
    pub round: u32,
    pub home_team: String,
    pub away_team: String,
    pub home_source_id: String,
    pub away_source_id: String,
    pub neutral: bool,
    pub result: Option<Outcome>,
}
pub const HISTORY_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameFile {
    pub schema_version: u32,
    pub league: String,
    pub fetched_at: String,
    pub source_url: String,
    pub from_season: i32,
    pub through_season: i32,
    pub teams: Vec<Team>,
    pub games: Vec<Game>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Rating {
    pub team: String,
    pub elo: f64,
    pub games: usize,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Audit {
    pub game_id: String,
    pub season: i32,
    pub home_before: f64,
    pub away_before: f64,
    pub expected_home_score: f64,
    pub observed_home_score: f64,
    pub home_after: f64,
    pub away_after: f64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct EloSeed {
    pub schema_version: u32,
    pub league: String,
    pub target_season: i32,
    pub through_season: i32,
    pub generated_at: String,
    pub history_sha256: String,
    pub config_sha256: String,
    pub settings: EloSettings,
    pub completed_games: usize,
    pub tied_games: usize,
    pub tie_weight: f64,
    pub ratings: Vec<Rating>,
    pub audit: Vec<Audit>,
}

pub fn digest(bytes: &[u8]) -> String {
    // sha2 0.11 returns a hybrid_array::Array, which no longer implements LowerHex.
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(hex, "{byte:02x}").expect("Writing to a String cannot fail");
    }
    hex
}

pub fn load_config(path: &Path) -> Result<(LeagueConfig, Vec<u8>)> {
    let bytes = fs::read(path).with_context(|| format!("Read config {}", path.display()))?;
    let config: LeagueConfig = serde_json::from_slice(&bytes)?;
    config.validate()?;
    Ok((config, bytes))
}

/// League ids name files and storage keys, so they are restricted to ASCII letters, digits, and `-`.
fn is_safe_league_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Load `<config_dir>/<league>.json` and require its `id` to be `league`.
pub fn load_league_config(config_dir: &Path, league: &str) -> Result<(LeagueConfig, Vec<u8>)> {
    ensure!(is_safe_league_id(league), "Unsafe league id");
    let (config, bytes) = load_config(&config_dir.join(format!("{league}.json")))?;
    ensure!(
        config.id == league,
        "Config id {} does not match requested league {league}",
        config.id
    );
    Ok((config, bytes))
}

/// Load `<data_dir>/<league>/history.json`, returning the parsed file and its bytes.
pub fn load_history(data_dir: &Path, cfg: &LeagueConfig) -> Result<(GameFile, Vec<u8>)> {
    let bytes = fs::read(data_dir.join(&cfg.id).join("history.json")).context("Read history.json; run history-importer first")?;
    let history = serde_json::from_slice(&bytes)
        .context("Parse history.json; start_time_utc, numeric round, and round_label are required; rerun history-importer")?;
    Ok((history, bytes))
}

const DAYS_IN_MONTH: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Day of year (1-365) in a non-leap year; `None` unless `value` is a valid "MM-DD".
fn month_day_ordinal(value: &str) -> Option<u32> {
    let b = value.as_bytes();
    if b.len() != 5 || b[2] != b'-' || ![b[0], b[1], b[3], b[4]].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let month: usize = value[..2].parse().ok()?;
    let day: u32 = value[3..].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=DAYS_IN_MONTH[month - 1]).contains(&day) {
        return None;
    }
    Some(DAYS_IN_MONTH[..month - 1].iter().sum::<u32>() + day)
}

impl DisplayVocabulary {
    pub fn validate(&self) -> Result<()> {
        let f = &self.schedule_filter;
        let labels = [
            &self.start_time_label,
            &f.label,
            &f.all_label,
            &self.postseason_label,
            &self.postseason_tie_note,
        ];
        ensure!(
            labels.iter().all(|s| !s.is_empty())
                && self.round_name.as_deref().is_none_or(|s| !s.is_empty())
                && self
                    .postseason_round_labels
                    .iter()
                    .all(|(k, v)| !k.is_empty() && !v.is_empty()),
            "Empty display label"
        );
        Ok(())
    }
}

impl LeagueWindows {
    pub fn validate(&self) -> Result<()> {
        let ord = |md: &str| month_day_ordinal(md).with_context(|| format!("Invalid month-day: {md}"));
        let (ss, se) = (ord(&self.season.start)?, ord(&self.season.end)?);
        let (ps, pe) = (ord(&self.postseason.start)?, ord(&self.postseason.end)?);
        ensure!(ss != se, "Season window start and end must be different");
        // Days after season.start, wrapping at the year boundary.
        let span = |to: u32| (to + 365 - ss) % 365;
        ensure!(
            span(ps) <= span(pe) && span(pe) <= span(se),
            "Postseason window must lie within the season window"
        );
        Ok(())
    }
}

/// Non-empty, finite, strictly ascending, and every value inside the parameter's domain.
fn valid_grid_axis(values: &[f64], in_domain: fn(f64) -> bool) -> bool {
    !values.is_empty() && values.iter().all(|&v| v.is_finite() && in_domain(v)) && values.windows(2).all(|pair| pair[0] < pair[1])
}

impl TuningGrids {
    pub fn validate(&self) -> Result<()> {
        let elo = self.elo.as_ref().is_none_or(|g| {
            valid_grid_axis(&g.k, |v| v > 0.0)
                && valid_grid_axis(&g.home_advantage, |v| v >= 0.0)
                && valid_grid_axis(&g.offseason_regression, |v| (0.0..=1.0).contains(&v))
        });
        let bayesian = self.bayesian.as_ref().is_none_or(|g| {
            valid_grid_axis(&g.prior_sd_elo, |v| v > 0.0)
                && valid_grid_axis(&g.tie_prior_games, |v| v > 0.0)
                && valid_grid_axis(&g.tie_prior_rate, |v| v > 0.0 && v < 1.0)
        });
        ensure!(elo && bayesian, "Invalid tuning grid");
        Ok(())
    }
}

impl LeagueConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 2, "Unsupported config schema");
        ensure!(is_safe_league_id(&self.id), "Unsafe league id");
        ensure!(
            (1..=12).contains(&self.season_rollover_month),
            "Invalid season rollover month"
        );
        ensure!((1900..=2200).contains(&self.history_start), "Invalid historical start");
        ensure!(adapter_for(&self.source.kind).is_ok(), "Unknown source adapter");
        ensure!(self.source.url.starts_with("https://"), "Source URL must use HTTPS");
        adapter_for(&self.source.kind)?.validate_config(self)?;
        ensure!(self.teams.len() >= 2, "At least two teams required");
        let ids: BTreeSet<_> = self.teams.iter().map(|t| t.id.as_str()).collect();
        ensure!(ids.len() == self.teams.len(), "Duplicate team ids");
        ensure!(
            self.teams.iter().all(|t| !t.id.is_empty() && !t.name.is_empty()),
            "Empty team id or name"
        );
        ensure!(
            self.aliases.values().all(|id| ids.contains(id.as_str())),
            "Alias points to unknown team"
        );
        ensure!(
            self.aliases
                .iter()
                .all(|(from, to)| !ids.contains(from.as_str()) || from == to),
            "Alias shadows a canonical team"
        );
        for team in &self.teams {
            ensure!(!team.eras.is_empty(), "Missing team identity history: {}", team.id);
            ensure!(
                team.eras[0].from_season <= self.history_start,
                "Team identity history starts too late: {}",
                team.id
            );
            for (i, era) in team.eras.iter().enumerate() {
                ensure!(
                    !era.name.is_empty()
                        && !era.location.is_empty()
                        && era.abbreviation.as_deref() != Some("")
                        && era.division.as_deref() != Some("")
                        && !era.source_ids.is_empty(),
                    "Incomplete team era: {}",
                    team.id
                );
                ensure!(
                    era.through_season.is_none_or(|end| end >= era.from_season),
                    "Invalid team era range"
                );
                ensure!(
                    era.source_ids.iter().all(|id| self.team_id(id) == team.id),
                    "Historical abbreviation points to another franchise"
                );
                if let Some(next) = team.eras.get(i + 1) {
                    ensure!(
                        era.through_season == Some(next.from_season - 1),
                        "Team eras overlap or have a gap: {}",
                        team.id
                    );
                } else {
                    ensure!(
                        era.through_season.is_none() && era.name == team.name && era.location == team.location,
                        "Latest identity must match current team metadata"
                    );
                }
            }
        }
        ensure!(
            self.ties_allowed_in
                .iter()
                .all(|s| ["regular", "postseason"].contains(&s.as_str())),
            "Unknown phase in tie rules"
        );
        self.display.validate()?;
        self.windows.validate()?;
        let e = &self.elo;
        ensure!(
            [e.initial, e.scale, e.k, e.home_advantage, e.offseason_regression]
                .iter()
                .all(|x| x.is_finite()),
            "Non-finite Elo parameter"
        );
        ensure!(
            e.scale > 0.0 && e.k > 0.0 && (0.0..=1.0).contains(&e.offseason_regression),
            "Invalid Elo parameters"
        );
        let b = &self.bayesian;
        ensure!(
            b.prior_sd_elo.is_finite()
                && b.prior_sd_elo > 0.0
                && b.tie_prior_games.is_finite()
                && b.tie_prior_games > 0.0
                && b.tie_prior_rate > 0.0
                && b.tie_prior_rate < 1.0,
            "Invalid Bayesian parameters"
        );
        if let Some(grids) = &self.tuning_grids {
            grids.validate()?;
        }
        self.validate_postseason()?;
        Ok(())
    }
    fn validate_postseason(&self) -> Result<()> {
        let Some(p) = &self.postseason else {
            for team in &self.teams {
                ensure!(
                    team.eras.iter().all(|e| e.division.is_none()),
                    "Team division needs a postseason format: {}",
                    team.id
                );
            }
            return Ok(());
        };
        ensure!(
            !self.ties_allowed_in.iter().any(|s| s == "postseason"),
            "Postseason format requires postseason ties to be disallowed"
        );
        ensure!(
            p.conferences.len().is_power_of_two(),
            "Conference count must be a power of two"
        );
        let mut ids = BTreeSet::new();
        for c in &p.conferences {
            ensure!(ids.insert(c.id.as_str()), "Duplicate conference or division id");
            for d in &c.divisions {
                ensure!(ids.insert(d.id.as_str()), "Duplicate conference or division id");
            }
        }
        ensure!(p.teams_per_conference >= 2, "Invalid playoff field size");
        ensure!(
            !p.division_winners_first
                || p.conferences
                    .iter()
                    .all(|c| c.divisions.len() <= p.teams_per_conference as usize),
            "More divisions than playoff spots"
        );
        let bracket = p.teams_per_conference.next_power_of_two().trailing_zeros() as usize;
        ensure!(
            p.rounds.len() == bracket + p.conferences.len().trailing_zeros() as usize,
            "Round count does not match the bracket"
        );
        let mut labels = BTreeSet::new();
        ensure!(
            p.rounds.iter().all(|r| labels.insert(r.round_label.as_str())),
            "Duplicate round label"
        );
        ensure!(
            p.rounds.iter().all(|r| !r.pattern.is_empty()
                && r.pattern.len() % 2 == 1
                && r.pattern.chars().all(|c| matches!(c, 'H' | 'A' | 'N'))),
            "Invalid series pattern"
        );
        ensure!(
            p.rounds
                .iter()
                .skip(bracket)
                .all(|r| r.home != SeriesHome::HigherSeed || !r.pattern.contains(['H', 'A'])),
            "Rounds between conferences cannot give home advantage by seed"
        );
        let rules = || p.tiebreakers.division.iter().chain(&p.tiebreakers.conference);
        ensure!(
            rules().all(|r| r.min_games.is_none() || r.rule == TiebreakKind::CommonGames),
            "min_games applies only to common_games"
        );
        ensure!(rules().all(|r| r.min_games != Some(0)), "min_games must be positive");
        for list in [&p.tiebreakers.division, &p.tiebreakers.conference] {
            let mut seen = Vec::new();
            for r in list {
                ensure!(!seen.contains(&r.rule), "Duplicate tiebreaker");
                seen.push(r.rule);
            }
        }
        let divisions: BTreeSet<&str> = p
            .conferences
            .iter()
            .flat_map(|c| &c.divisions)
            .map(|d| d.id.as_str())
            .collect();
        for team in &self.teams {
            for era in &team.eras {
                ensure!(
                    era.division.as_deref().is_none_or(|d| divisions.contains(d)),
                    "Unknown division: {}",
                    team.id
                );
            }
        }
        fn current(team: &Team) -> Option<&str> {
            team.eras.last().and_then(|e| e.division.as_deref())
        }
        for team in &self.teams {
            ensure!(current(team).is_some(), "Current division missing: {}", team.id);
        }
        for d in p.conferences.iter().flat_map(|c| &c.divisions) {
            ensure!(
                self.teams.iter().any(|t| current(t) == Some(d.id.as_str())),
                "Division has no current teams: {}",
                d.id
            );
        }
        for c in &p.conferences {
            let count = self
                .teams
                .iter()
                .filter(|t| current(t).is_some_and(|d| c.divisions.iter().any(|x| x.id == d)))
                .count();
            ensure!(
                count >= p.teams_per_conference as usize,
                "Conference has fewer teams than playoff spots: {}",
                c.id
            );
        }
        Ok(())
    }
    pub fn current_season(&self) -> i32 {
        let today = Utc::now().date_naive();
        today.year() - i32::from(today.month() < self.season_rollover_month)
    }
    pub fn team_id(&self, value: &str) -> String {
        self.aliases.get(value).cloned().unwrap_or_else(|| value.to_owned())
    }
    pub fn identity(&self, id: &str, season: i32) -> Result<&TeamEra> {
        self.teams
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| {
                t.eras
                    .iter()
                    .find(|e| season >= e.from_season && e.through_season.is_none_or(|end| season <= end))
            })
            .with_context(|| format!("No historical identity for {id} in {season}"))
    }
}

/// Source-local game row that source adapters build and normalize into a `Game`.
#[derive(Deserialize)]
pub(crate) struct SourceGame {
    pub(crate) id: String,
    pub(crate) league: String,
    pub(crate) season: i32,
    #[serde(default)]
    pub(crate) date: String,
    /// Source-local HH:mm, or null when the source does not supply a start time.
    pub(crate) time: Option<String>,
    pub(crate) timezone: String,
    pub(crate) phase: String,
    pub(crate) round_label: String,
    pub(crate) round: u32,
    pub(crate) home_team: String,
    pub(crate) away_team: String,
    pub(crate) home_source_id: String,
    pub(crate) away_source_id: String,
    pub(crate) neutral: bool,
    pub(crate) result: Option<Outcome>,
}

impl SourceGame {
    pub(crate) fn normalize(self, warn: &mut dyn FnMut(String)) -> Result<Game> {
        Ok(Game {
            start_time_utc: time::start_time_utc(&self, warn)?,
            id: self.id,
            league: self.league,
            season: self.season,
            phase: self.phase,
            round_label: self.round_label,
            round: self.round,
            home_team: self.home_team,
            away_team: self.away_team,
            home_source_id: self.home_source_id,
            away_source_id: self.away_source_id,
            neutral: self.neutral,
            result: self.result,
        })
    }
}

/// Parse source documents with the configured adapter, then validate and sort the games.
pub fn parse_documents(documents: &[String], cfg: &LeagueConfig, mut warn: impl FnMut(String)) -> Result<Vec<Game>> {
    let mut games = adapter_for(&cfg.source.kind)?.parse(documents, cfg, &mut warn)?;
    validate_games(&mut games, cfg)?;
    Ok(games)
}

/// Provider adapters normalize into the same league-independent game contract.
pub fn parse_source(text: &str, cfg: &LeagueConfig) -> Result<Vec<Game>> {
    parse_source_with_warnings(text, cfg, |_| {})
}

pub fn parse_source_with_warnings(text: &str, cfg: &LeagueConfig, warn: impl FnMut(String)) -> Result<Vec<Game>> {
    parse_documents(&[text.to_owned()], cfg, warn)
}

pub fn validate_games(games: &mut [Game], cfg: &LeagueConfig) -> Result<()> {
    let strict_source_ids = adapter_for(&cfg.source.kind)?.strict_source_ids();
    let ids: BTreeSet<_> = cfg.teams.iter().map(|t| t.id.as_str()).collect();
    let mut seen = BTreeSet::new();
    for g in games.iter() {
        ensure!(!g.id.is_empty() && seen.insert(&g.id), "Empty or duplicate game id: {}", g.id);
        ensure!(g.league == cfg.id, "Wrong league for {}", g.id);
        ensure!(
            ids.contains(g.home_team.as_str()) && ids.contains(g.away_team.as_str()),
            "Unknown team for {}",
            g.id
        );
        ensure!(g.home_team != g.away_team, "Team playing itself: {}", g.id);
        for (team, source) in [(&g.home_team, &g.home_source_id), (&g.away_team, &g.away_source_id)] {
            ensure!(
                cfg.team_id(source) == *team,
                "Source id does not match franchise for {}",
                g.id
            );
            let era = cfg.identity(team, g.season)?;
            ensure!(
                !strict_source_ids || era.source_ids.contains(source),
                "Source team abbreviation is invalid for season {} in {}",
                g.season,
                g.id
            );
        }
        ensure!(g.round > 0, "Invalid round");
        ensure!(["regular", "postseason"].contains(&g.phase.as_str()), "Invalid phase");
        ensure!(
            g.result != Some(Outcome::Tie) || cfg.ties_allowed_in.contains(&g.phase),
            "Tie in a phase that forbids ties: {}",
            g.id
        );
    }
    games.sort_by(chronological);
    Ok(())
}

/// Replay order: season, then start time, then id.
pub(crate) fn chronological(a: &Game, b: &Game) -> Ordering {
    (a.season, a.start_time_utc, &a.id).cmp(&(b.season, b.start_time_utc, &b.id))
}

pub fn fetch_source(url: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("game-results-prediction/0.1")
        .build()?;
    let mut last = None;
    for attempt in 0..3 {
        match client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text())
        {
            Ok(body) => return Ok(body),
            Err(error) => {
                last = Some(error);
                if attempt < 2 {
                    std::thread::sleep(Duration::from_secs(1 << attempt));
                }
            }
        }
    }
    Err(last.unwrap().into())
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("Output needs a parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    // Only POSIX lets a directory be opened as a file to durably record the rename.
    // On Windows the rename is already atomic and the handle would be denied.
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// Save a tool report as `<dir>/<name>-<league>-<UTC date of run_at>.json` and return its path.
pub fn save_report<T: Serialize>(dir: &Path, name: &str, league: &str, run_at: &str, report: &T) -> Result<PathBuf> {
    let date = DateTime::parse_from_rfc3339(run_at)?.with_timezone(&Utc).format("%Y-%m-%d");
    let path = dir.join(format!("{name}-{league}-{date}.json"));
    write_json(&path, report).with_context(|| format!("Write report {}", path.display()))?;
    Ok(path)
}

/// Advisory lock is released by the OS even if the process crashes.
pub fn lock(path: &Path) -> Result<File> {
    fs::create_dir_all(path.parent().context("Lock needs a parent directory")?)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    fs2::FileExt::try_lock_exclusive(&file).context("Another process is writing this output; try again after it finishes")?;
    Ok(file)
}

pub fn build_seed(
    history: &GameFile,
    history_bytes: &[u8],
    cfg: &LeagueConfig,
    config_bytes: &[u8],
    target: i32,
) -> Result<EloSeed> {
    seed_with_ties(history, history_bytes, cfg, config_bytes, target).map(|(seed, _)| seed)
}

/// `build_seed`, also returning the tie history that set the seed's tie weight.
pub(crate) fn seed_with_ties(
    history: &GameFile,
    history_bytes: &[u8],
    cfg: &LeagueConfig,
    config_bytes: &[u8],
    target: i32,
) -> Result<(EloSeed, bayesian::TieHistory)> {
    cfg.validate()?;
    ensure!(
        history.teams == cfg.teams,
        "Team identity history changed; rerun history-importer before elo-ratings"
    );
    ensure!(
        history.schema_version == HISTORY_SCHEMA_VERSION && history.league == cfg.id,
        "Incompatible history file; rerun history-importer"
    );
    ensure!(
        history.from_season == cfg.history_start && history.through_season == target - 1,
        "History must cover {} through {}. Run history-importer first.",
        cfg.history_start,
        target - 1
    );
    ensure!(
        history
            .games
            .iter()
            .all(|g| g.season >= cfg.history_start && g.season < target),
        "History contains a game outside its declared training period"
    );
    let EloReplay { mut ratings, audit } = replay_elo(&history.games, cfg, target - 1)?;
    let ties = bayesian::TieHistory::from_audit(&audit, &history.games, cfg)?;
    // Apply exactly one offseason regression between the final historical season and the target.
    for r in &mut ratings {
        r.elo = regress_rating(r.elo, &cfg.elo);
    }
    let seed = EloSeed {
        schema_version: 1,
        league: cfg.id.clone(),
        target_season: target,
        through_season: target - 1,
        generated_at: Utc::now().to_rfc3339(),
        history_sha256: digest(history_bytes),
        config_sha256: digest(config_bytes),
        settings: cfg.elo.clone(),
        completed_games: audit.len(),
        tied_games: ties.tied_games,
        tie_weight: ties.estimate(&cfg.bayesian),
        ratings,
        audit,
    };
    Ok((seed, ties))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> EloSettings {
        EloSettings {
            initial: 1500.0,
            scale: 400.0,
            k: 20.0,
            home_advantage: 55.0,
            offseason_regression: 1.0 / 3.0,
        }
    }
    #[test]
    fn elo_symmetry_and_neutral_venue() {
        let e = settings();
        assert_eq!(expected_home(1500.0, 1500.0, true, &e), 0.5);
        assert!(expected_home(1500.0, 1500.0, false, &e) > 0.5);
        assert!((expected_home(1600.0, 1400.0, true, &e) + expected_home(1400.0, 1600.0, true, &e) - 1.0).abs() < 1e-12);
    }
    #[test]
    fn tie_moves_favorite_down() {
        let e = settings();
        let delta = e.k * (0.5 - expected_home(1600.0, 1400.0, true, &e));
        assert!(delta < 0.0);
        assert!((1600.0 + delta + 1400.0 - delta - 3000.0).abs() < 1e-12);
    }
}
