# Rally Row: redundant and overlapping tool work

As of October 10, 2026.

## Summary

One overlap matters: two tools compute the same Elo held-out error. One smaller overlap follows. Five more are resolved: the browser downloading about 45 MB of MLB data it barely used, `evaluate-model` recomputing the Bayesian held-out predictions that `simulate-season` saves, the Node backtest repeating work done by `evaluate-model` and by the app, the history being replayed once per predicted season, and repeated imports and copied data. The review covers the six Rust tools (`import-history`, `generate-preseason-seed`, `elo-tune`, `bayes-tune`, `evaluate-model`, `simulate-season`), the Node backtest (since removed), and the files the browser reads.

| # | Overlap | Where | Impact |
| --- | --- | --- | --- |
| 1 | Unused seed `audit` and `history.json` downloaded by the browser | `generate-preseason-seed`, `storage.ts`, `vite.config.ts` | Resolved; was High: \~45 MB uncompressed for MLB |
| 2 | Identical Bayesian held-out predictions | `bayes-tune`, `evaluate-model`, `simulate-season` | Resolved; `bayes-tune`'s copy is by design. Was Medium: same computation and checks three times |
| 3 | Elo held-out error with two definitions | `elo-tune`, `evaluate-model` | Medium: same metric, can disagree for MLB |
| 4 | Backtest repeated evaluation and pregame reconstruction | Node backtest, `evaluate-model`, `service.ts` | Resolved; the backtest is removed. Was Medium: separate code paths that disagreed with the app |
| 5 | Score summaries built three times | `elo-tune`, `bayes-tune`, `evaluate-model` | Low: maintenance |
| 6 | History replayed once per season | `walk_forward::preseason` | Resolved; was Low: runtime |
| 7 | Repeated imports and copied data | Pages workflow, `import-history`, data files | Resolved; was Low |

## 1. The browser downloaded files it barely used (resolved)

The browser needs about 30 ratings and one tie weight per league from the seed, and only a hash of the history. It used to fetch the whole seed, replay audit included, and the whole history. Sizes are uncompressed; GitHub Pages compresses them in transit.

| Published file | MLB before | MLB now | NFL before | NFL now | What the browser uses |
| --- | --- | --- | --- | --- | --- |
| `elo-2026.json` | 20.5 MB | 3.2 KB | 2.0 MB | 3.3 KB | `ratings`, `tie_weight`, `completed_games`, hashes |
| `history.json`, now `history.sha256` | 24.5 MB | 65 bytes | 2.4 MB | 65 bytes | The SHA-256, compared with `seed.history_sha256` |

- **Seed `audit`:** removed. `generate-preseason-seed` no longer writes the replay audit, which nothing read.
- **`history.json`:** the site build publishes `history.sha256`, the history's SHA-256, in its place. `readSeedWithHash` in `storage.ts` compares that value with `seed.history_sha256` instead of downloading and hashing the whole history.

## 2. Three tools produced the same Bayesian held-out predictions (resolved)

`bayes-tune`, `evaluate-model`, and `simulate-season` each predicted the held-out seasons with the configured Bayesian settings, and their scores matched to the last digit.

- **`evaluate-model`** now scores `simulated-seasons.json` instead of calling `walk_forward::predict_season` itself. It no longer takes split options or runs `Split::resolve` and `validate`: the held-out seasons come from the file, and `SimulatedSeasons::load` rejects a file whose schema, league, configuration hash, or history hash doesn't match. Its NFL and MLB reports are unchanged apart from `run_at` and one note. Exact scores required parsing JSON floats exactly (serde_json's `float_roundtrip`); the default parser changed some scores in the last digit.
- **`bayes-tune`**'s `holdout.baseline` still runs the same steps (`walk_forward::preseasons`, then `posteriors_by_utc_date`) with the configured settings, by design. The tuner comes before the simulation: it selects the settings that `simulate-season` then uses, so it must not depend on the simulation's output. It also needs the same code to score the selected settings.

| NFL, held-out 2023–2025, mean of seasons | `bayes-tune` `holdout.baseline` | `evaluate-model` Bayesian |
| --- | --- | --- |
| Log loss | 0.6474945103923352 | 0.6474945103923352 |
| Brier | 0.4488847058044494 | 0.4488847058044494 |

## 3. The Elo held-out error is computed twice, two ways

`elo-tune`'s `holdout.baseline.mean_season_mse` and `evaluate-model`'s Elo `expected_score_mse` are the same metric on the same games: both are 0.2226960472097451 for the NFL.

They can differ only when a team plays twice on one UTC date, as in MLB doubleheaders. `elo-tune` updates ratings between games on the same date; `simulate-season`, whose predictions `evaluate-model` scores, predicts each date from the ratings at its start. The same metric therefore has two definitions, and `evaluate-model`'s notes have to explain the gap.

## 4. The Node backtest overlapped both `evaluate-model` and the app (resolved)

The Node backtest, `apps/web/scripts/backtest.ts`, is removed, along with its `backtest` script, the `tsx` dependency, and `docs/backtesting.md`. Its scores were a subset of `evaluate-model`'s, and its date-by-date refits repeated the app's own pregame reconstruction with different grouping.

- **Same scoring as `evaluate-model`, fewer scores.** It scored Bayesian against the equal-strength baseline with log loss and Brier, predicting each UTC date from earlier dates. Scoring a past season also needed its own data directory, import, and seed run.
- **Re-implemented the app's pregame reconstruction differently.** `service.ts` refits per pregame day to show completed games' favorites. The backtest grouped by UTC date instead, so its predictions didn't match the app's for NFL night games or for MLB games that follow a team's late game on the same UTC date.
- **Duplicated code.** The seed checks were copied from `readSeedWithHash`. Log loss, Brier, and the equal-strength baseline were written again in TypeScript, separately from `scoring.rs` and `outcome_probabilities`.
- **What only the backtest covered:** scoring a season still in progress; no tool does that now. The shared fixtures (`bayesian-parity.json`, `mlb-statsapi.json`) check that the TypeScript and Rust model and MLB adapter agree, and the app itself runs the TypeScript adapter on live provider data.

## 5. Score summaries are built three times

`elo-tune`, `bayes-tune`, and `evaluate-model` each define their own score summaries; only the basic functions in `scoring.rs` (`log_loss`, `brier`, `calibration`, `standard_error`, `mean`) are shared.

| Piece | `elo-tune` | `bayes-tune` | `evaluate-model` |
| --- | --- | --- | --- |
| Per-game scores | MSE | Log loss, Brier, ties | Log loss, Brier, expected-score MSE, accuracy, ties |
| Season, first/second half, pooled | Own code | Own code | Own code |
| Mean of seasons, paired standard error | Own code | Own code | Own code |
| Calibration | Own bins on expected score | Shared `scoring::calibration` | Shared `scoring::calibration` |

`bayes-tune`'s per-game score fields are a strict subset of `evaluate-model`'s.

## 6. History was replayed once per season instead of once (resolved)

`walk_forward::preseason` replayed the whole history from `history_start` for each season it predicted, although a single replay passes through every season start. Each replay also built the full audit (67,494 entries for MLB) and then discarded it.

- **One replay:** `walk_forward::preseasons` replays the history once, through the season before the last one requested. When the replay reaches each requested season's start, it saves that season's preseason ratings, game counts, and tie history. `TieHistory` grows one game at a time during the replay instead of being rebuilt from the audit.
- **Same seeds as `generate-preseason-seed`:** `build_seed` runs the same replay for its one season, so a season's seed doesn't depend on which tool builds it. A test checks each season's seed from one replay against `build_seed` run on the history up to that season.
- **Callers:** `bayes-tune` prepares its tuning and held-out seasons in one call. `simulate-season` predicts each prepared season with `Preseason::predict`.

| Tool | Full replays before, NFL | Full replays before, MLB | Full replays now |
| --- | --- | --- | --- |
| `bayes-tune` | 16 | 20 | 1 |
| `evaluate-model` | 0 | 0 | 0 |
| `simulate-season` | 3 | 3 | 1 |

Outputs are unchanged: apart from timestamps, `simulate-season`'s files, both leagues' seeds, and `bayes-tune`'s NFL and MLB reports match those from before the change exactly. Run times barely moved, because the replays were cheap next to the Bayesian fits. MLB `bayes-tune` took 5 min 8 s before and 5 min 5 s after, and MLB `simulate-season` 4.4 s and 4.1 s. The replay no longer builds an audit.

## 7. Repeated imports and copied data (resolved)

Data was fetched or stored more than once.

- **Re-import on every Pages build:** every master push and every monthly run downloaded the full history, about 28 Stats API requests for MLB and one CSV for the NFL. The weekly scheduled check now builds only while some league's current-season seed is missing from the site, which happens after its `season_rollover_month` (January for MLB, April for the NFL), the only times a published seed changes. That makes about 2 scheduled full imports a year instead of 12, plus a retry each week after a failed build. Master pushes still import in full, because the runner starts without data.
- **Incremental import:** `import-history` keeps the seasons of a current `history.json` and downloads only later ones. A rerun with no new season downloads nothing, and `--refresh` downloads every season. A saved history is current when its schema, league, source URL, adapter version, `history_start`, and `teams_sha256` match the configuration and its games still validate. This saves requests during development only, since a Pages build has no saved file.
- **`simulated-seasons.json`:** saves each game's `id` and its two predictions instead of the whole game record. `SimulatedSeasons::load` restores the record from `history.json`, which `history_sha256` pins. The NFL file went from 743 KB to 419 KB; the MLB file from 6.4 MB to 3.6 MB.
- **`history.json` `teams`:** replaced by `teams_sha256`, the SHA-256 of the configuration's `teams`. The check stays, because aliases and eras decide which franchise each game belongs to, and `config_sha256` doesn't cover the history.
- **The seed's `settings`:** removed, along with `simulated-seasons.json`'s `elo_settings` and `bayesian_settings`, which `config_sha256` already pins. `evaluate-model` reports the settings from the configuration, which `load` has matched by hash.

The history schema is now 5 (4 at the time of this change; 5 adds `adapter_version`), the seed schema 2, and the simulated-seasons schema 2. The data is unchanged apart from these fields: an NFL import through 2024 followed by one through 2025 matched a single full import except for `fetched_at`, and for both leagues the reimported games, the regenerated seed's ratings and tie weight, and every simulated prediction matched the files from before the change.

## Already shared, and how this was checked

The season-split options and checks (`Split::resolve`, `tuning::validate`), the input hashing, and the seed builder are already shared in `rating-core`, so they are not redundant.

File sizes and entry counts come from the local `data/` directory. The matching scores in findings 2 and 3 come from running `bayes-tune`, `elo-tune`, and `evaluate-model` on the NFL data with the configured settings. The before-and-after comparison in finding 6 comes from running `simulate-season`, `generate-preseason-seed`, and `bayes-tune` on the local NFL and MLB data. The comparison in finding 7 comes from importing the NFL history in two steps and in one, reimporting the MLB history, and rerunning `generate-preseason-seed` and `simulate-season` for both leagues.
