# Rally Row: redundant and overlapping tool work

As of October 9, 2026.

## Summary

Two overlaps matter: two tools compute the same Elo held-out error, and the Node backtest repeats work done by `evaluate-model` and by the app. Three smaller overlaps follow. Two more are resolved: the browser downloading about 45 MB of MLB data it barely used, and `evaluate-model` recomputing the Bayesian held-out predictions that `simulate-season` saves. The review covers the six Rust tools (`import-history`, `generate-preseason-seed`, `elo-tune`, `bayes-tune`, `evaluate-model`, `simulate-season`), the Node backtest, and the files the browser reads.

| # | Overlap | Where | Impact |
| --- | --- | --- | --- |
| 1 | Unused seed `audit` and `history.json` downloaded by the browser | `generate-preseason-seed`, `storage.ts`, `vite.config.ts` | Resolved; was High: \~45 MB uncompressed for MLB |
| 2 | Identical Bayesian held-out predictions | `bayes-tune`, `evaluate-model`, `simulate-season` | Resolved; `bayes-tune`'s copy is by design. Was Medium: same computation and checks three times |
| 3 | Elo held-out error with two definitions | `elo-tune`, `evaluate-model` | Medium: same metric, can disagree for MLB |
| 4 | Backtest repeats evaluation and pregame reconstruction | Node backtest, `evaluate-model`, `service.ts` | Medium: separate code paths that disagree for MLB |
| 5 | Score summaries built three times | `elo-tune`, `bayes-tune`, `evaluate-model` | Low: maintenance |
| 6 | History replayed once per season | `walk_forward::preseason` | Low: runtime |
| 7 | Repeated imports and copied data | Pages workflow, data files | Low |

## 1. The browser downloaded files it barely used (resolved)

The browser needs about 30 ratings and one tie weight per league from the seed, and only a hash of the history. It used to fetch the whole seed, replay audit included, and the whole history. Sizes are uncompressed; GitHub Pages compresses them in transit.

| Published file | MLB before | MLB now | NFL before | NFL now | What the browser uses |
| --- | --- | --- | --- | --- | --- |
| `elo-2026.json` | 20.5 MB | 3.2 KB | 2.0 MB | 3.3 KB | `ratings`, `tie_weight`, `completed_games`, hashes |
| `history.json`, now `history.sha256` | 24.5 MB | 65 bytes | 2.4 MB | 65 bytes | The SHA-256, compared with `seed.history_sha256` |

- **Seed `audit`:** `generate-preseason-seed` now writes the replay audit to `elo-audit-<season>.json` instead of the seed, and the site doesn't publish it.
- **`history.json`:** the site build publishes `history.sha256`, the history's SHA-256, in its place. `readSeedWithHash` in `storage.ts` compares that value with `seed.history_sha256` instead of downloading and hashing the whole history. The backtest still hashes `history.json` from disk.

## 2. Three tools produced the same Bayesian held-out predictions (resolved)

`bayes-tune`, `evaluate-model`, and `simulate-season` each predicted the held-out seasons with the configured Bayesian settings, and their scores matched to the last digit.

- **`evaluate-model`** now scores `simulated-seasons.json` instead of calling `walk_forward::predict_season` itself. It no longer takes split options or runs `Split::resolve` and `validate`: the held-out seasons come from the file, and `SimulatedSeasons::load` rejects a file whose schema, league, configuration hash, or history hash doesn't match. Its NFL and MLB reports are unchanged apart from `run_at` and one note. Exact scores required parsing JSON floats exactly (serde_json's `float_roundtrip`); the default parser changed some scores in the last digit.
- **`bayes-tune`**'s `holdout.baseline` still runs the same steps (`walk_forward::preseason`, then `posteriors_by_utc_date`) with the configured settings, by design. The tuner comes before the simulation: it selects the settings that `simulate-season` then uses, so it must not depend on the simulation's output. It also needs the same code to score the selected settings.

| NFL, held-out 2023–2025, mean of seasons | `bayes-tune` `holdout.baseline` | `evaluate-model` Bayesian |
| --- | --- | --- |
| Log loss | 0.6474945103923352 | 0.6474945103923352 |
| Brier | 0.4488847058044494 | 0.4488847058044494 |

## 3. The Elo held-out error is computed twice, two ways

`elo-tune`'s `holdout.baseline.mean_season_mse` and `evaluate-model`'s Elo `expected_score_mse` are the same metric on the same games: both are 0.2226960472097451 for the NFL.

They can differ only when a team plays twice on one UTC date, as in MLB doubleheaders. `elo-tune` updates ratings between games on the same date; `simulate-season`, whose predictions `evaluate-model` scores, predicts each date from the ratings at its start. The same metric therefore has two definitions, and `evaluate-model`'s notes have to explain the gap.

## 4. The Node backtest overlaps both `evaluate-model` and the app

The backtest's scores are a subset of `evaluate-model`'s, and its date-by-date refits repeat the app's own pregame reconstruction with different grouping.

- **Same scoring as `evaluate-model`, fewer scores.** It scores Bayesian against the equal-strength baseline with log loss and Brier, predicting each UTC date from earlier dates. Scoring a past season also needs its own data directory, import, and seed run.
- **Re-implements the app's pregame reconstruction differently.** `service.ts` already refits per pregame day to show completed games' favorites. The backtest groups by UTC date instead of the adapter's pregame day, so for MLB its predictions don't match what the app shows.
- **Duplicated code.** The seed checks are copied from `readSeedWithHash`. Log loss, Brier, and the equal-strength baseline are written again in TypeScript, separately from `scoring.rs` and `outcome_probabilities`.
- **What only the backtest covers:** the TypeScript adapter and result eligibility running on live provider data, and scoring a season still in progress. The shared fixtures (`bayesian-parity.json`, `mlb-statsapi.json`) already check that the TypeScript and Rust model and MLB adapter agree.

## 5. Score summaries are built three times

`elo-tune`, `bayes-tune`, and `evaluate-model` each define their own score summaries; only the basic functions in `scoring.rs` (`log_loss`, `brier`, `calibration`, `standard_error`, `mean`) are shared.

| Piece | `elo-tune` | `bayes-tune` | `evaluate-model` |
| --- | --- | --- | --- |
| Per-game scores | MSE | Log loss, Brier, ties | Log loss, Brier, expected-score MSE, accuracy, ties |
| Season, first/second half, pooled | Own code | Own code | Own code |
| Mean of seasons, paired standard error | Own code | Own code | Own code |
| Calibration | Own bins on expected score | Shared `scoring::calibration` | Shared `scoring::calibration` |

`bayes-tune`'s per-game score fields are a strict subset of `evaluate-model`'s.

## 6. History is replayed once per season instead of once

`walk_forward::preseason` replays the whole history from `history_start` for each season it predicts, although a single replay passes through every season start. Each replay also builds the full audit (67,494 entries for MLB) and then discards it.

| Tool | Full replays, NFL | Full replays, MLB |
| --- | --- | --- |
| `bayes-tune` | 16 | 20 |
| `evaluate-model` | 0 | 0 |
| `simulate-season` | 3 | 3 |

This is cheap next to the Bayesian fits, so it is minor.

## 7. Repeated imports and copied data (minor)

The rest is data fetched or stored more than once.

- **Re-import on every Pages build.** Every master push and every monthly run re-downloads the full history: about 28 Stats API requests for MLB and one CSV for the NFL. Completed seasons rarely change, and the MLB Stats API terms disallow bulk use.
- **`simulated-seasons.json`** repeats every game record from `history.json` beside its predictions; the MLB file is 6.4 MB.
- **`history.json` `teams`** copies the configuration's `teams` and is used only for an equality check.
- **The seed's `settings`** copies the configuration's `elo` block, which `config_sha256` already pins.

## Already shared, and how this was checked

The season-split options and checks (`Split::resolve`, `tuning::validate`), the input hashing, and the seed builder are already shared in `rating-core`, so they are not redundant.

File sizes and entry counts come from the local `data/` directory. The matching scores in findings 2 and 3 come from running `bayes-tune`, `elo-tune`, and `evaluate-model` on the NFL data with the configured settings.
