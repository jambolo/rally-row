# Bayesian tuning

`bayes-tune` searches a league's Bayesian prior uncertainty (`prior_sd_elo`) and tie smoothing (`tie_prior_games`
and `tie_prior_rate`) on historical seasons, keeping the configured Elo settings fixed. It then scores the selected
and current settings on later held-out seasons. It prints a JSON report and never changes the configuration or data.
[Bayesian parameter tuning](model.md#bayesian-parameter-tuning) describes the method.

## Running the tool

[Import the league's history](history-import.md) through the last held-out season first. The tool runs offline
against `data/<id>/history.json` and the league configuration; it doesn't use published Elo seeds. Adopt any
[Elo tuning](elo-tuning.md) results first, because the search uses the configured Elo settings.

```powershell
cargo run --release -p bayes-tune -- --league nfl
cargo run --release -p bayes-tune -- --league mlb

# Also save the report
cargo run --release -p bayes-tune -- --league nfl --report-dir target

# Split overrides
cargo run --release -p bayes-tune -- --league nfl --tune-start 2010 --tune-end 2022 --test-end 2025
```

| Option | Meaning |
| --- | --- |
| `--league <id>` | Required. Reads `<config-dir>/<id>.json`, whose `id` must equal `<id>`. Ids use letters, digits, and hyphens. |
| `--config-dir <dir>` | Configuration directory; default `config`. |
| `--data-dir <dir>` | Data directory; default `data`. Reads `<dir>/<id>/history.json`. |
| `--report-dir <dir>` | Also saves the report as `<dir>/bayes-tuning-report-<id>-<YYYY-MM-DD>.json`, dated by the run's UTC start. A same-day run replaces it. |
| `--tune-start <year>` | First tuning season; default `bayes_tune.tune_start`, then `elo_tune.tune_start`. |
| `--tune-end <year>` | Last tuning season; the held-out seasons follow it. Default `bayes_tune.tune_end`, then `elo_tune.tune_end`. |
| `--test-end <year>` | Last held-out season; default `bayes_tune.test_end`, then `elo_tune.test_end`. |

The split defaults come from the configuration's `bayes_tune`, or from `elo_tune` when `bayes_tune` is absent.
Neither shipped configuration sets `bayes_tune`, so both leagues use the [Elo tuning seasons](elo-tuning.md#seasons),
with held-out seasons 2023–2025. The split checks and their errors are the same as `elo-tune`'s; a missing boundary
reports `Set bayes_tune.tune_start (or elo_tune.tune_start) in the config or pass --tune-start`.

The JSON report goes to stdout. Progress messages go to stderr, including a count every ten candidates. The tool only
reads its inputs, so it takes no lock, and repeated runs on the same inputs give the same report apart from `run_at`.
The MLB run takes about 4 minutes in a release build.

## The search

Each candidate is scored by predicting every tuning season one UTC date at a time and averaging each season's mean
log loss:

- Each season's preseason Elo ratings and tie weight are rebuilt from the earlier seasons, as
  [`generate-preseason-seed`](preseason-seed.md) builds a seed. The tie weight depends on the candidate's tie smoothing.
- Each UTC date is predicted from a Laplace posterior fit to the season's earlier dates only, so no prediction sees
  its own result or any result from the same date.

1. The search scores the current settings and a starting grid: `tuning_grids.bayesian` from the configuration, or the
   default grid of 120 combinations. MLB configures its own grid.
2. The three best candidates are refined once by multiplying each parameter by `1/sqrt(2)`, 1, or `sqrt(2)`.
3. Among candidates within one paired-season standard error of the lowest log loss, the one closest to the current
   settings is selected.
4. Only the selected and current settings are then scored on the held-out seasons.

[Bayesian parameter tuning](model.md#bayesian-parameter-tuning) gives the grids and the distance used for selection.

## Report

`Report` in `apps/bayes-tune/src/tuning.rs` defines the report.

| Field | Contents |
| --- | --- |
| `run_at` | RFC 3339 UTC timestamp of the run's start |
| `league` | League id |
| `config_sha256`, `history_sha256` | SHA-256 of the configuration and history bytes the run read |
| `method`, `selection_rule` | One-line descriptions of the scoring and the selection |
| `split` | `warmup_start`, `tune_start`, `tune_end`, `test_end` |
| `fixed_elo` | The configuration's `elo` block |
| `baseline_parameters` | Current settings: `prior_sd_elo`, `tie_prior_games`, `tie_prior_rate` |
| `selected_parameters` | Selected settings, with the same fields |
| `search` | How the search went ([Search fields](#search-fields)) |
| `tuning` | Current versus selected settings on the tuning seasons ([Comparisons](#comparisons)) |
| `holdout` | Current versus selected settings on the held-out seasons |
| `notes` | Caveats, including a boundary warning ([Adopting the selection](#adopting-the-selection)) |

### Search fields

| Field | Contents |
| --- | --- |
| `grid_source` | `config` for `tuning_grids.bayesian`, else `default` |
| `coarse_grid` | Starting grid: `prior_sd_elo`, `tie_prior_games`, and `tie_prior_rate` value lists |
| `refinement_centers` | The three candidates refined |
| `refinement_multipliers` | `[1/sqrt(2), 1, sqrt(2)]` |
| `candidates_evaluated` | Distinct candidates scored |
| `near_best_candidates` | Candidates within one paired-season standard error of the minimum |
| `minimum_log_loss_parameters` | Candidate with the lowest log loss, which may differ from the selection |
| `top_candidates` | The ten lowest-log-loss candidates: `parameters`, `mean_season_log_loss`, `mean_season_brier` |
| `evaluated_ranges` | Each parameter's `[minimum, maximum]` over all candidates |
| `selected_on_boundary` | Parameters whose selected value equals an end of its evaluated range |

### Comparisons

`tuning` and `holdout` each compare the current (`baseline`) and selected settings:

| Field | Contents |
| --- | --- |
| `baseline`, `selected` | Scores of each setting ([Scores](#scores)) |
| `mean_season_log_loss_improvement` | Baseline minus selected mean season log loss; positive favors the selection |
| `paired_season_standard_error` | Standard error of the per-season differences, or null with fewer than two seasons |
| `season_differences` | Each season's `baseline_minus_selected_log_loss` |

### Scores

| Field | Contents |
| --- | --- |
| `games` | Games scored |
| `mean_season_log_loss`, `mean_season_brier` | Equally weighted means of the season scores |
| `pooled` | Scores over all games |
| `seasons` | Per season: `season`, its `tie_weight`, and `overall`, `first_half`, and `second_half` scores; halves split the season's games by count and are null when empty |
| `calibration` | Per outcome, the tenths of forecast probability that contain games: `outcome` (`home_win`, `away_win`, or `tie`), `lower`, `upper`, `games`, `mean_probability`, `observed_rate` |

Each score has `games`, `log_loss`, `brier`, `expected_ties` (the sum of tie probabilities), and `observed_ties`.
[Reading the scores](model.md#reading-the-scores) explains the scores and their standard errors.

## Adopting the selection

When `selected_on_boundary` is not empty, the notes say
`Selected parameters lie on a searched boundary (<names>); extend tuning_grids in the league configuration and rerun before adopting.`
Widen `tuning_grids.bayesian` past that boundary and rerun.

To adopt the selection, copy `selected_parameters` into the configuration's `bayesian` block. The seed's tie weight
depends on the tie smoothing, so rerun [`generate-preseason-seed`](preseason-seed.md) for every seed, then
[`simulate-season`](season-simulation.md) and [`evaluate-model`](model-evaluation.md) to score the adopted settings. Revising the search after reading the held-out scores makes them useless for evaluation.
