# Elo tuning

`elo-tune` searches a league's Elo update factor K, home advantage, and offseason regression on historical seasons,
then scores the selected and current settings on later held-out seasons. It prints a JSON report and never changes
the configuration or data. [Elo-only tuning](model.md#elo-only-tuning) describes the method.

## Running the tool

[Import the league's history](history-import.md) through the last held-out season first. The tool runs offline
against `data/<id>/history.json` and the league configuration; it doesn't need the Elo seed.

```powershell
cargo run --release -p elo-tune -- --league nfl
cargo run --release -p elo-tune -- --league mlb

# Also save the report
cargo run --release -p elo-tune -- --league nfl --report-dir target

# Split overrides
cargo run --release -p elo-tune -- --league nfl --tune-start 2010 --tune-end 2022 --test-end 2025
```

| Option | Meaning |
| --- | --- |
| `--league <id>` | Required. Reads `<config-dir>/<id>.json`, whose `id` must equal `<id>`. Ids use letters, digits, and hyphens. |
| `--config-dir <dir>` | Configuration directory; default `config`. |
| `--data-dir <dir>` | Data directory; default `data`. Reads `<dir>/<id>/history.json`. |
| `--report-dir <dir>` | Also saves the report as `<dir>/elo-tuning-report-<id>-<YYYY-MM-DD>.json`, dated by the run's UTC start. A same-day run replaces it. |
| `--tune-start <year>` | First tuning season; default `elo_tune.tune_start`. |
| `--tune-end <year>` | Last tuning season; the held-out seasons follow it. Default `elo_tune.tune_end`. |
| `--test-end <year>` | Last held-out season; default `elo_tune.test_end`. |

The JSON report goes to stdout and progress messages to stderr. The tool only reads its inputs, so it takes no lock,
and repeated runs on the same inputs give the same report apart from `run_at`.

### Seasons

The split divides the history into three ranges:

| Seasons | Range | NFL default | MLB default |
| --- | --- | --- | --- |
| Warm-up | `history_start` through `tune_start - 1` | 2002–2009 | 1998–2005 |
| Tuning | `tune_start` through `tune_end` | 2010–2022 | 2006–2022 |
| Held-out | `tune_end + 1` through `test_end` | 2023–2025 | 2023–2025 |

Each option overrides one boundary of the configuration's `elo_tune` split. Without `elo_tune`, all three options
are required. [`bayes-tune`](bayesian-tuning.md), [`evaluate-model`](model-evaluation.md), and
[`simulate-season`](season-simulation.md) take the same options and run the same checks, but read `bayes_tune` first.

| Error | Cause |
| --- | --- |
| `Set elo_tune.tune_start in the config or pass --tune-start` | A boundary has neither a configured default nor an option; likewise for the others |
| `Require warm-up history, at least two tuning seasons, and later completed held-out seasons` | The seasons are not ordered `history_start < tune_start < tune_end < test_end < current season` |
| `History must cover <first> through <last>; rerun import-history with --through-season <last>` | The history ends before `test_end` |
| `History schema, league, or franchise identities do not match configuration; rerun import-history` | The history is stale or for another league |
| `Evaluation history contains an unreported game; use completed historical seasons` | A game through `test_end` has no result |
| `Season <year> has <reason>; refresh history before tuning` | A season lacks the league's completion marker |

## The search

Each candidate is scored by replaying the history with its settings and averaging each season's mean squared error
between the pregame expected score `E_h` and the observed score (win 1, tie 1/2, loss 0) over the tuning seasons.
The initial rating and Elo scale stay fixed.

1. The search scores the current settings and a starting grid: `tuning_grids.elo` from the configuration, or the
   default grid of 125 combinations.
2. While the best candidate lies on an expandable edge of the grid, the grid expands, up to four times.
3. The three best candidates are refined on a finer local grid.
4. Among candidates within one paired-season standard error of the lowest error, the one closest to the current
   settings is selected.
5. Only the selected and current settings are then scored on the held-out seasons.

[Elo-only tuning](model.md#elo-only-tuning) gives the grids, step sizes, and distance used for selection.

## Report

`Report` in `apps/elo-tune/src/tuning.rs` defines the report.

| Field | Contents |
| --- | --- |
| `run_at` | RFC 3339 UTC timestamp of the run's start |
| `league` | League id |
| `config_sha256`, `history_sha256` | SHA-256 of the configuration and history bytes the run read |
| `method`, `selection_rule` | One-line descriptions of the scoring and the selection |
| `split` | `warmup_start`, `tune_start`, `tune_end`, `test_end` |
| `initial_elo`, `elo_scale` | The fixed configured values |
| `baseline_parameters` | Current settings: `k`, `home_advantage`, `offseason_regression` |
| `selected_parameters` | Selected settings, with the same fields |
| `search` | How the search went ([Search fields](#search-fields)) |
| `tuning` | Current versus selected settings on the tuning seasons ([Comparisons](#comparisons)) |
| `holdout` | Current versus selected settings on the held-out seasons |
| `notes` | Caveats, including a boundary warning ([Adopting the selection](#adopting-the-selection)) |

### Search fields

| Field | Contents |
| --- | --- |
| `grid_source` | `config` for `tuning_grids.elo`, else `default` |
| `coarse_grid` | Starting grid: `k`, `home_advantage`, and `offseason_regression` value lists |
| `expanded_grid` | The grid after expansion |
| `expansion_rounds` | Number of expansions, 0 to 4 |
| `refinement_steps` | Step size of each parameter in the local refinement |
| `candidates_evaluated` | Distinct candidates scored |
| `near_best_candidates` | Candidates within one paired-season standard error of the minimum |
| `minimum_mse_parameters` | Candidate with the lowest error, which may differ from the selection |
| `top_candidates` | The ten lowest-error candidates: `parameters` and `mean_season_mse` |
| `evaluated_ranges` | Each parameter's `[minimum, maximum]` over all candidates |
| `selected_on_boundary` | Parameters whose selected value equals an end of its evaluated range |

### Comparisons

`tuning` and `holdout` each compare the current (`baseline`) and selected settings:

| Field | Contents |
| --- | --- |
| `baseline`, `selected` | Scores of each setting ([Scores](#scores)) |
| `mean_season_mse_improvement` | Baseline minus selected mean season MSE; positive favors the selection |
| `relative_improvement_percent` | The improvement as a percentage of the baseline's error, or null when that is zero |
| `paired_season_standard_error` | Standard error of the per-season differences, or null with fewer than two seasons |
| `season_differences` | Each season's `baseline_minus_selected_mse` |

### Scores

| Field | Contents |
| --- | --- |
| `games` | Games scored |
| `mean_season_mse` | Equally weighted mean of the season errors |
| `pooled_mse` | Error over all games |
| `seasons` | Per season: `overall`, `first_half`, and `second_half`, each with `games` and `mse`; halves split the season's games by count and are null when empty |
| `calibration` | Tenths of expected score that contain games: `lower`, `upper`, `games`, `mean_expected_score`, `mean_observed_score` |

[Reading the scores](model.md#reading-the-scores) explains the error and its standard errors.

## Adopting the selection

When `selected_on_boundary` is not empty, the notes say
`Selected parameters lie on a searched boundary (<names>); extend tuning_grids in the league configuration and rerun before adopting.`
Widen `tuning_grids.elo` past that boundary and rerun. A note also reports when the grid still touched an expandable
edge after four expansions.

To adopt the selection, copy `selected_parameters` into the configuration's `elo` block, then rerun
[`generate-preseason-seed`](preseason-seed.md) for every seed and [`simulate-season`](season-simulation.md) if you use
its file.
[`bayes-tune`](bayesian-tuning.md) keeps the configured Elo settings fixed, so tune Elo before the Bayesian settings.
Revising the search after reading the held-out scores makes them useless for evaluation.
