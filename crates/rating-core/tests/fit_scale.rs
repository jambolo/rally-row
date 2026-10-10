use chrono::{TimeZone, Utc};
use rating_core::{EloSeed, Game, LeagueConfig, Outcome, Rating, bayesian::fit_posterior};
use test_support::league_config;

// Mirrored by apps/web/test/fit-scale.test.ts: same generator, seed value, and fixed model settings, so both
// languages fit the identical 2,430-game season. With plain objective summation this fit does not converge.
struct Lcg(u32);

impl Lcg {
    fn draw(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 16
    }
}

fn config() -> LeagueConfig {
    let mut cfg = league_config("mlb");
    cfg.elo.initial = 1500.0;
    cfg.elo.scale = 400.0;
    cfg.elo.home_advantage = 24.0;
    cfg.bayesian.prior_sd_elo = 75.0;
    cfg
}

fn synthetic_season(value: u32, cfg: &LeagueConfig) -> (EloSeed, Vec<Game>) {
    let mut rng = Lcg(value);
    let teams: Vec<&str> = cfg.teams.iter().map(|t| t.id.as_str()).collect();
    let elo: Vec<i64> = teams.iter().map(|_| 1400 + i64::from(rng.draw() % 201)).collect();
    let mut games = Vec::new();
    for i in 0..2430 {
        let h = rng.draw() as usize % teams.len();
        let mut a = rng.draw() as usize % (teams.len() - 1);
        if a >= h {
            a += 1;
        }
        let result = if i64::from(rng.draw() % 1000) < 535 + elo[h] - elo[a] {
            Outcome::HomeWin
        } else {
            Outcome::AwayWin
        };
        games.push(Game {
            id: format!("g{i}"),
            league: cfg.id.clone(),
            season: 2026,
            start_time_utc: Utc.with_ymd_and_hms(2026, 9, 1, 17, 0, 0).unwrap(),
            phase: "regular".into(),
            round_label: "R".into(),
            round: 1,
            home_team: teams[h].into(),
            away_team: teams[a].into(),
            home_source_id: teams[h].into(),
            away_source_id: teams[a].into(),
            neutral: false,
            result: Some(result),
        });
    }
    let seed = EloSeed {
        schema_version: 1,
        league: cfg.id.clone(),
        target_season: 2026,
        through_season: 2025,
        generated_at: "2026-09-01T00:00:00Z".into(),
        history_sha256: String::new(),
        config_sha256: String::new(),
        settings: cfg.elo.clone(),
        completed_games: 0,
        tied_games: 0,
        tie_weight: 0.0003,
        ratings: teams
            .iter()
            .zip(&elo)
            .map(|(&team, &elo)| Rating {
                team: team.into(),
                elo: elo as f64,
                games: 162,
            })
            .collect(),
    };
    (seed, games)
}

#[test]
fn mlb_scale_synthetic_season_converges() {
    let cfg = config();
    assert_eq!(cfg.teams.len(), 30);
    let (seed, games) = synthetic_season(4558, &cfg);
    let model = fit_posterior(&seed, &games, &cfg).unwrap();
    assert_eq!(model.means.len(), 30);
    assert!(model.means.iter().all(|v| v.is_finite()));
    assert!(model.covariance.iter().flatten().all(|v| v.is_finite()));
    assert!((0..30).all(|i| model.covariance[i][i] > 0.0));
}
