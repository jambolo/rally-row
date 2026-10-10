# Preseason seed

`generate-preseason-seed` replays a league's history with the configured Elo settings and saves the preseason seed
for one season to `data/<id>/elo-<season>.json`. The seed holds each team's preseason rating and the tie weight ν.
The replay behind it goes to a separate audit file, `data/<id>/elo-audit-<season>.json`, which the site doesn't
publish.
The browser's Bayesian model starts from them ([Bayesian prior](model.md#bayesian-prior)), and so does the Node
[backtest](backtesting.md).

## Running the tool

[Import the league's history](history-import.md) through the season before the target season first. The tool runs
offline.

```powershell
cargo run --release -p generate-preseason-seed -- --league nfl
cargo run --release -p generate-preseason-seed -- --league mlb

# Seed an earlier season from history imported through the season before it
cargo run --release -p import-history -- --league nfl --through-season 2024 --data-dir data-backtest
cargo run --release -p generate-preseason-seed -- --league nfl --target-season 2025 --data-dir data-backtest
```

| Option | Meaning |
| --- | --- |
| `--league <id>` | Required. Reads `<config-dir>/<id>.json`, whose `id` must equal `<id>`. Ids use letters, digits, and hyphens. |
| `--config-dir <dir>` | Configuration directory; default `config`. |
| `--data-dir <dir>` | Data directory; default `data`. Reads `<dir>/<id>/history.json` and writes `<dir>/<id>/elo-<season>.json` and `<dir>/<id>/elo-audit-<season>.json`. |
| `--target-season <year>` | Seed season; default the current season. |

The history must cover `history_start` through exactly the season before the target season, so seeding another
season needs a history imported through the season before it, usually in a separate data directory.

stdout gets one line naming the saved files:
`Saved <teams> team priors for <season> from <games> completed games (<ties> ties) to <seed path>, and their replay to
<audit path>`. Each file is replaced atomically, so a failed run leaves the previous seed in place. A lock file, `data/<id>/elo.lock`, keeps two runs from
writing the same league at once.

| Error | Cause |
| --- | --- |
| `Read history.json; run import-history first` | No history file |
| `History must cover <first> through <last>. Run import-history first.` | The history doesn't end with the season before the target season |
| `Team identity history changed; rerun import-history before generate-preseason-seed` | The configuration's `teams` changed after the import |
| `Incompatible history file; rerun import-history` | The history's schema version or league doesn't match |

## How the seed is built

1. Every team starts at `elo.initial`. Completed games are replayed in order of season, UTC start time, and game id.
   Each game is scored with the pregame ratings, and then both ratings move by K times the surprise
   ([Historical Elo](model.md#historical-elo)).
2. Before each new season, ratings regress toward `elo.initial` by `offseason_regression`. One more regression
   after the last historical season makes the seed's ratings the target season's preseason values.
3. The tie weight ν is estimated from the historical games in phases that allow ties and their pregame Elo
   differences, smoothed by `tie_prior_games` and `tie_prior_rate`
   ([Win, loss, and tie likelihood](model.md#win-loss-and-tie-likelihood)).

[`bayes-tune`](bayesian-tuning.md), [`evaluate-model`](model-evaluation.md), and
[`simulate-season`](season-simulation.md) build each evaluated season's preseason values the same way, from the
seasons before it.

## File format

`EloSeed` in `crates/rating-core/src/lib.rs` defines the file. `seedSchema` in `apps/web/src/contracts.ts` validates
it in the browser and the backtest.

| Field | Contents |
| --- | --- |
| `schema_version` | `1` |
| `league` | League id matching the configuration |
| `target_season` | Season the seed is for |
| `through_season` | Last replayed season, `target_season - 1` |
| `generated_at` | RFC 3339 UTC timestamp of the run |
| `history_sha256` | SHA-256 of the `history.json` bytes the run read |
| `config_sha256` | SHA-256 of the configuration bytes the run read |
| `settings` | The configuration's `elo` block |
| `completed_games` | Completed historical games replayed |
| `tied_games` | Ties among them |
| `tie_weight` | Tie weight ν |
| `ratings` | One entry per team: `team` id, preseason `elo` rating, and `games`, its number of completed historical games |

### Audit file

`EloAudit` in `crates/rating-core/src/lib.rs` defines `elo-audit-<season>.json`. No tool reads it. It has `schema_version`
`1`, the seed's `league`, `target_season`, `through_season`, `generated_at`, `history_sha256`, and `config_sha256`, and
`games`, one entry per replayed game in replay order. Each entry records one game's update:

| Field | Contents |
| --- | --- |
| `game_id`, `season` | The game |
| `home_before`, `away_before` | Pregame ratings |
| `expected_home_score` | Elo expected home score `E_h` |
| `observed_home_score` | 1 for a home win, 1/2 for a tie, 0 for an away win |
| `home_after`, `away_after` | Ratings after the game, before any offseason regression |

## Using the seed

The browser loads the current season's seed for the selected league. It rejects a seed that doesn't match:

| Error | Check |
| --- | --- |
| `Elo seed is for the wrong league or season; run the two Rust programs` | `league`, `target_season`, and `through_season` match the league and season |
| `Configuration changed since Elo was calculated; rerun generate-preseason-seed` | `config_sha256` matches the published configuration |
| `History changed since Elo was calculated; rerun generate-preseason-seed` | `history_sha256` matches the published history hash, `data/<id>/history.sha256` |

The [backtest](backtesting.md) applies the same checks. The hash covers the whole configuration file, so rerun
`generate-preseason-seed` after any configuration change, not only a change to `elo` or `bayesian`, and after every
import. The Pages workflow regenerates every league's history and seed before each build, and its monthly run
publishes each league's new-season seed after its rollover month.
