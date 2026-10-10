# Season simulation

`simulate-season` simulates a league's held-out seasons with the configured settings and saves every game's
pregame predictions to `data/<id>/simulated-seasons.json`. [`evaluate-model`](model-evaluation.md) scores this file,
and other evaluation tools can read it instead of refitting the model.

## Running the tool

Import the league's history first. The tool runs offline against `data/<id>/history.json` and the league
configuration; it doesn't need the Elo seed.

```powershell
cargo run --release -p simulate-season -- --league nfl
cargo run --release -p simulate-season -- --league mlb

# Split overrides; the held-out seasons are tune_end + 1 through test_end
cargo run --release -p simulate-season -- --league nfl --tune-end 2022 --test-end 2025
```

| Option | Meaning |
| --- | --- |
| `--league <id>` | Required. Reads `<config-dir>/<id>.json`, whose `id` must equal `<id>`. |
| `--config-dir <dir>` | Configuration directory; default `config`. |
| `--data-dir <dir>` | Data directory; default `data`. Reads `<dir>/<id>/history.json` and writes `<dir>/<id>/simulated-seasons.json`. |
| `--tune-start <year>` | First tuning season, validated only; default `bayes_tune.tune_start`, then `elo_tune.tune_start`. |
| `--tune-end <year>` | Last tuning season; the held-out seasons follow it. Default `bayes_tune.tune_end`, then `elo_tune.tune_end`. |
| `--test-end <year>` | Last held-out season; default `bayes_tune.test_end`, then `elo_tune.test_end`. |

The split defaults and checks match [`bayes-tune`](bayesian-tuning.md), so both shipped leagues simulate 2023–2025
([Seasons](elo-tuning.md#seasons) lists the checks and their errors). `evaluate-model` scores the seasons the split
holds out. The history must cover every held-out season with completed games, and the last
held-out season must precede the current season.

Progress goes to stderr, and stdout gets one line naming the saved file. The file is written only after every season
is simulated, and it is replaced atomically, so a failed run leaves the previous file in place. A lock file,
`data/<id>/simulated-seasons.lock`, keeps two runs from writing the same league at once. In a release build, the
NFL run takes about a second and the MLB run about 7 seconds.

Rerun the tool after any change to the league's configuration or history. The file records hashes of both inputs,
and `evaluate-model` rejects a file whose hashes don't match
([Checking that a file is current](#checking-that-a-file-is-current)).

## How the seasons are simulated

Each held-out season is replayed as the app would have run it during that season:

1. Preseason Elo ratings and the tie weight ν are rebuilt from the earlier seasons only, as
   [`generate-preseason-seed`](preseason-seed.md) builds a seed. Earlier held-out seasons count as history, so 2025
   starts from ratings that include 2023 and 2024.
2. Games are taken one UTC date at a time, in start order. Both predictors forecast every game of a date from the
   preseason values and the results of earlier dates:
   - **Bayesian**: the app's model. A Laplace posterior is fit from the preseason prior to the season's earlier
     results, and each game's probabilities average the Davidson likelihood over the posterior uncertainty of the
     matchup ([Posterior and numerical method](model.md#posterior-and-numerical-method)).
   - **Elo**: the preseason ratings, updated after every earlier game as in the
     [historical replay](model.md#historical-elo). Its three outcome probabilities apply the Davidson tie term to
     the pregame Elo difference ([Model evaluation](model.md#model-evaluation)).
3. The date's results are then added, and the next date is predicted.

No prediction sees its own result or any result from the same UTC date. The app instead uses each source's pregame
day, so for MLB a late game whose UTC start falls on the day after its official date is predicted here with the
earlier games of that official date, unlike in the app
([Pregame reconstruction and backtesting](model.md#pregame-reconstruction-and-backtesting)).

## File format

`SimulatedSeasons` in `crates/rating-core/src/lib.rs` defines the file, and `SeasonPredictions`,
`PredictedGame`, and `Prediction` in `crates/rating-core/src/walk_forward.rs` define its parts. Rust tools can
deserialize it with serde.

| Field | Contents |
| --- | --- |
| `schema_version` | `1` |
| `league` | League id matching the configuration |
| `generated_at` | RFC 3339 UTC timestamp of the run |
| `history_sha256` | SHA-256 of the `history.json` bytes the run read |
| `config_sha256` | SHA-256 of the configuration bytes the run read |
| `split` | `warmup_start`, `tune_start`, `tune_end`, `test_end`; the simulated seasons are `tune_end + 1` through `test_end` |
| `elo_settings` | The configuration's `elo` block |
| `bayesian_settings` | The configuration's `bayesian` block |
| `seasons` | One entry per simulated season, in season order |

Each entry of `seasons` has these fields:

| Field | Contents |
| --- | --- |
| `season` | Season label |
| `tie_weight` | Tie weight ν from the earlier seasons; both predictors use it in phases listed in `ties_allowed_in` |
| `games` | Every game of the season, in start order |

Each game has every field of a [`history.json` game](extending.md#historical-output), followed by a `bayesian` and an
`elo` prediction. Every simulated game is completed, so `result` is never null.

```json
{
  "id": "2023_01_DET_KC",
  "league": "nfl",
  "season": 2023,
  "start_time_utc": "2023-09-08T00:20:00Z",
  "phase": "regular",
  "round_label": "REG",
  "round": 1,
  "home_team": "KC",
  "away_team": "DET",
  "home_source_id": "KC",
  "away_source_id": "DET",
  "neutral": false,
  "result": "away_win",
  "bayesian": {
    "home_win": 0.7169698137311274,
    "away_win": 0.2805971172505295,
    "tie": 0.0024330690183424868,
    "expected_home_score": 0.7181863482402987
  },
  "elo": {
    "home_win": 0.7422808528462821,
    "away_win": 0.2551778287029246,
    "tie": 0.0025413184507931666,
    "expected_home_score": 0.7441720309591229
  }
}
```

Each prediction has these fields:

| Field | Contents |
| --- | --- |
| `home_win`, `away_win`, `tie` | Probabilities of the three outcomes. They sum to one, and `tie` is zero in phases that don't allow ties. |
| `expected_home_score` | Expected fractional home score (win 1, tie 1/2, loss 0). Bayesian: `home_win + tie / 2`. Elo: its logistic expected score `E_h`, which equals `home_win / (home_win + away_win)`. |

## Using the file

### Checking that a file is current

A file describes the inputs it was generated from. Before using it, check that `schema_version` is `1` and that
`config_sha256` and `history_sha256` equal the SHA-256 hashes of the current `config/<id>.json` and
`data/<id>/history.json` bytes (`rating_core::digest` in Rust). If either differs, rerun the tool. An incompatible
change to the format increases `schema_version`. In Rust, `SimulatedSeasons::load` loads a file and makes these
checks, as `evaluate-model` does.

### Equal-strength baseline

`evaluate-model` also scores an equal-strength baseline, which the file doesn't store because it follows from
`tie_weight`. In a phase that allows ties, the baseline gives a tie `ν / (2 + ν)` and each team `1 / (2 + ν)`; in
other phases, each team gets 1/2. Its expected home score is 1/2.

### Publishing

`data/` is ignored by Git, and the Pages workflow doesn't generate this file, so the site never publishes it. A
local `pnpm -C apps/web build` copies every JSON file under `data/` into the site, including this file when it is
present.
