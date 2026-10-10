# Model evaluation

`evaluate-model` scores the model's predictions, made with the configured (tuned) settings, on a league's held-out
seasons. It compares two ways of learning from in-season results on the same games: the app's Bayesian model and Elo
ratings updated after every game, with an equal-strength baseline for reference. The tuners never use the held-out
seasons to select parameters, so their scores measure forecasting skill fairly.
[Model evaluation](model.md#model-evaluation) describes the method, and
[Reading the scores](model.md#reading-the-scores) explains what the scores mean and how to compare them.

## Running the tool

[Import the league's history](history-import.md) through the last held-out season first. The tool runs offline
against `data/<id>/history.json` and the league configuration; it doesn't use published Elo seeds. It scores whatever
the configuration holds, so adopt tuned parameters before running it.

```powershell
cargo run --release -p evaluate-model -- --league nfl
cargo run --release -p evaluate-model -- --league mlb

# Print the full JSON report instead of the summary
cargo run --release -p evaluate-model -- --league nfl --json

# Also save the JSON report
cargo run --release -p evaluate-model -- --league nfl --report-dir target

# Split overrides; the held-out seasons are tune_end + 1 through test_end
cargo run --release -p evaluate-model -- --league nfl --tune-end 2022 --test-end 2025
```

| Option | Meaning |
| --- | --- |
| `--league <id>` | Required. Reads `<config-dir>/<id>.json`, whose `id` must equal `<id>`. Ids use letters, digits, and hyphens. |
| `--config-dir <dir>` | Configuration directory; default `config`. |
| `--data-dir <dir>` | Data directory; default `data`. Reads `<dir>/<id>/history.json`. |
| `--report-dir <dir>` | Also saves the JSON report, in either output mode, as `<dir>/model-evaluation-report-<id>-<YYYY-MM-DD>.json`, dated by the run's UTC start. A same-day run replaces it. |
| `--tune-start <year>` | First tuning season, validated only; default `bayes_tune.tune_start`, then `elo_tune.tune_start`. |
| `--tune-end <year>` | Last tuning season; the held-out seasons follow it. Default `bayes_tune.tune_end`, then `elo_tune.tune_end`. |
| `--test-end <year>` | Last held-out season; default `bayes_tune.test_end`, then `elo_tune.test_end`. |
| `--json` | Prints the full JSON report to stdout instead of the summary. |

The split defaults and checks match [`bayes-tune`](bayesian-tuning.md), so both shipped leagues evaluate 2023–2025
([Seasons](elo-tuning.md#seasons) lists the checks and their errors). When a held-out season lies within a configured
tuning range, a note says that those seasons are not fully held out.

Progress messages go to stderr in either mode. The tool only reads its inputs, so it takes no lock. The MLB run takes
a few seconds in a release build.

## How the predictions are made

For each held-out season, preseason Elo ratings and the tie weight ν are rebuilt from the earlier seasons only, as
[`generate-preseason-seed`](preseason-seed.md) builds a seed, and every predictor starts from them. Games are then
predicted one UTC date at a time from the results of earlier dates only:

- **Bayesian**: the app's model, a Laplace posterior fit to the season's earlier dates.
- **Elo**: the preseason ratings, updated after every earlier game. Its three outcome probabilities apply the Davidson
  tie term to the pregame Elo difference.
- **Equal strength**: no home advantage, the non-tie probability split evenly, and the season's tie weight at equal
  strength, as in the [backtest](backtesting.md).

[`simulate-season`](season-simulation.md) saves the same Bayesian and Elo predictions, game by game, to a file.

## Summary

By default, stdout gets a summary like this one, for the NFL:

```text
League nfl: held-out seasons 2023–2025, 855 games
Mean of seasons     Log loss     Brier  Exp. score MSE  Accuracy
Bayesian              0.6475    0.4489          0.2236     64.8%
Elo                   0.6456    0.4471          0.2227     64.2%
Equal strength        0.7019    0.5012          0.2497     50.0%
Bayesian advantage over Elo (positive favors Bayesian), mean of seasons ± paired-season SE:
  log_loss            -0.0019 ± 0.0019
  brier               -0.0017 ± 0.0016
  expected_score_mse  -0.0009 ± 0.0008
  accuracy            +0.0058 ± 0.0051
Season    Log loss (Bayes/Elo)  Accuracy (Bayes/Elo)
2023           0.6644 / 0.6661         61.1% / 60.4%
2024           0.6166 / 0.6138         67.7% / 66.3%
2025           0.6615 / 0.6569         65.5% / 65.8%
Favorites agree in 833 of 854 decisive games. In the other 21, the Bayesian favorite won 13 and the Elo favorite won 8.
Home-win probabilities differ by 1.5 points on average (at most 7.7).
```

The summary's layout is not a stable format; programs should pass `--json`.

## Report

`Report` in `apps/evaluate-model/src/evaluation.rs` defines the JSON report.

| Field | Contents |
| --- | --- |
| `run_at` | RFC 3339 UTC timestamp of the run's start |
| `league` | League id |
| `config_sha256`, `history_sha256` | SHA-256 of the configuration and history bytes the run read |
| `method` | One-line description of the predictions |
| `split` | `warmup_start`, `tune_start`, `tune_end`, `test_end` |
| `holdout_seasons` | `[first, last]` held-out season |
| `elo_settings`, `bayesian_settings` | The configuration's `elo` and `bayesian` blocks |
| `tie_weights` | Each held-out season's `season` and `tie_weight` |
| `predictors` | Scores of `bayesian`, `elo`, and `equal_strength` ([Predictor scores](#predictor-scores)) |
| `comparison` | Bayesian versus Elo, game by game ([Comparison](#comparison)) |
| `notes` | Caveats about the method and the scores |

### Predictor scores

| Field | Contents |
| --- | --- |
| `games` | Games scored |
| `mean_season` | Equally weighted season means of `log_loss`, `brier`, `expected_score_mse`, and `accuracy` |
| `pooled` | Score over all games |
| `seasons` | Per season: `overall`, `first_half`, and `second_half` scores; halves split the season's games by count and are null when empty |
| `calibration` | Per outcome, the tenths of forecast probability that contain games: `outcome` (`home_win`, `away_win`, or `tie`), `lower`, `upper`, `games`, `mean_probability`, `observed_rate` |

Each score has these fields:

| Field | Contents |
| --- | --- |
| `games` | Games scored |
| `log_loss`, `brier`, `expected_score_mse` | Mean scores |
| `decisive_games` | Games that didn't end in a tie |
| `correct_picks` | Decisive games won by the favored team; an exact toss-up counts 1/2 |
| `accuracy` | `correct_picks / decisive_games`, or null without decisive games |
| `expected_ties`, `observed_ties` | Sum of tie probabilities, and ties that happened |

### Comparison

| Field | Contents |
| --- | --- |
| `metrics` | One entry per score: `log_loss`, `brier`, `expected_score_mse`, `accuracy` |
| `picks` | `decisive_games`, `same_favorite`, `different_favorite`, and, among the games with different favorites, `bayesian_correct_when_different` and `elo_correct_when_different` |
| `mean_absolute_home_win_difference`, `max_absolute_home_win_difference` | Mean and largest difference between the two home-win probabilities |

Each `metrics` entry has these fields:

| Field | Contents |
| --- | --- |
| `metric` | Score name |
| `higher_is_better` | `true` only for `accuracy` |
| `bayesian_advantage_mean_season` | Bayesian advantage in the mean of seasons; positive favors the Bayesian predictions |
| `paired_season_standard_error` | Standard error of the per-season advantages |
| `bayesian_advantage_pooled` | Bayesian advantage over all games |
| `paired_game_standard_error` | Standard error of the per-game advantages |
| `seasons` | Each season's `season` and `bayesian_advantage` |

Values that can't be computed, such as a standard error from one season or accuracy without decisive games, are null.
