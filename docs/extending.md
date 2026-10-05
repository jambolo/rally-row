# Add a league

Rally Row serves every league in its league registry; the NFL and MLB ship today. Rust and TypeScript source adapters share the same team and outcome metadata. Historical data stores each game's UTC start time; the browser's current-season data also keeps the source's local timing fields. There is no league-specific team count, schedule length, or Elo formula in either model. Provider-specific rules, such as finality, result eligibility, the pregame day, and the historical completion marker, are defined only in source adapters.

Adding a league takes these steps:

1. Write its configuration, `config/<id>.json` ([Configuration](#configuration)).
2. Use an existing source adapter, or add one in both languages ([Source adapters](#source-adapters)).
3. Add its id to the browser's league registry ([League registry](#league-registry)).
4. Generate its history and Elo seed, then tune and backtest it ([Generate and publish data](#generate-and-publish-data)).
5. Add its importer and Elo invocations to `.github/workflows/pages.yml` so the site publishes its data.

## Configuration

Create `config/<id>.json`, usually by copying `config/nfl.json` or `config/mlb.json`. The league id `<id>` uses letters, digits, and hyphens; it names the file and must equal the file's `id`. Every program reads `<config-dir>/<id>.json` for `--league <id>` and rejects a file whose `id` differs. Change the name, complete team list, aliases, historical start, season rollover month, source, display vocabulary, windows, model settings, and tie rules.

| Key | Contents |
| --- | --- |
| `schema_version` | `2` |
| `id`, `name` | League id and display name; the name labels the league in the header's **League** switcher |
| `history_start` | First historical season; every configured team must exist from then on |
| `season_rollover_month` | Month (1–12) that starts a new season: the current season is the UTC year, minus one before this month |
| `source` | `kind`, a registered [source adapter](#source-adapters), and its HTTPS `url` |
| `teams`, `aliases` | Franchise identities with their eras, and provider ids that map to them |
| `ties_allowed_in` | Phases whose games may end in a tie |
| `display` | Interface vocabulary ([Display vocabulary](#display-vocabulary)) |
| `windows` | Season and postseason windows ([Windows](#windows)) |
| `elo` | `initial`, `scale`, `k`, `home_advantage`, `offseason_regression` |
| `bayesian` | `prior_sd_elo`, `tie_prior_games`, `tie_prior_rate` |
| `elo_tune`, `bayes_tune` | Optional default tuning splits (`tune_start`, `tune_end`, `test_end`); `bayes_tune` falls back to `elo_tune` |
| `tuning_grids` | Optional starting grids: `elo` (`k`, `home_advantage`, `offseason_regression`) for `elo-tune` and `bayesian` (`prior_sd_elo`, `tie_prior_games`, `tie_prior_rate`) for `bayes-tune`. Each list must be non-empty, finite, strictly ascending, and inside the parameter's domain. |

For the existing generic provider, use:

```json
{
  "kind": "canonical-json",
  "url": "https://your-provider.example/games.json"
}
```

Set `ties_allowed_in` to `[]` if no games may end in a tie, `["regular"]` if only regular-season games allow ties, or `["regular", "postseason"]` if both phases do. A tie in a phase that doesn't allow ties is an error. Where ties are allowed, Elo scores a tie as 0.5.

Team IDs are stable franchise identities. Map the aliases of renamed or relocated teams to those identities; create a new team only for a new franchise. Each team has a current `name` and `location`, plus `eras` containing `from_season`, inclusive `through_season` (null for the open-ended current era), historical `name`, home-market `location`, optional display `abbreviation` (default: the first source id), and accepted `source_ids`. Eras must be contiguous and non-overlapping, cover the configured history start, and end in an open-ended era matching the current metadata. See [franchise identity rules](team-history.md). The model assumes the configured team list is appropriate for the entire modeled historical period. Supporting league expansion *within* that period may need activation dates and a policy for new-team priors.

### Display vocabulary

The browser renders every league-specific label from `display`:

| Key | Use | NFL | MLB |
| --- | --- | --- | --- |
| `start_time_label` | Label before each game's date | `Kickoff` | `First pitch` |
| `round_name` | Round label in game details; `null` hides it | `Week` | `null` |
| `schedule_filter` | Schedule filter `unit` (`round` or `date`), `label`, and `all_label` | `round`, `Week`, `All weeks` | `date`, `Date`, `All dates` |
| `postseason_label` | Postseason game label when no round label applies | `Playoffs` | `Postseason` |
| `postseason_round_labels` | Postseason labels keyed by the source's round label | `{}` | `F` Wild Card, `D` Division Series, `L` League Championship Series, `W` World Series |
| `postseason_tie_note` | Shown in place of the tie label for postseason matchups when postseason ties are not allowed | `No ties in NFL playoffs` | `No ties in MLB postseason` |

### Windows

`windows.season` and `windows.postseason` are inclusive `MM-DD` ranges (`start`, `end`) that may wrap the year boundary. The postseason window must lie within the season window.

| Window | NFL | MLB |
| --- | --- | --- |
| `season` | `09-01` to `02-15` | `03-20` to `11-05` |
| `postseason` | `01-08` to `02-15` | `09-30` to `11-05` |

The browser uses the windows only to choose the default league when neither the address hash nor a remembered choice decides. On the browser-local date (February 29 counts as February 28), exactly one league in its season window is the default. If several are in season, the first in registry order that is in its postseason window wins, else the one with the fewest days until its postseason starts. If none is in season, the one whose season starts soonest wins. Remaining ties follow registry order.

## Source adapters

A source adapter holds every rule specific to one provider format. A configuration names its adapter with `source.kind`. Each language has one adapter registry, and no code outside the adapter modules and registries compares `source.kind` values.

| `source.kind` | Downloads | Result eligibility | Pregame day | Completion marker |
| --- | --- | --- | --- | --- |
| `nflverse-csv` | One CSV for all seasons | From the next calendar day in Eastern Time (no live/final flag) | Eastern Time date | Completed Super Bowl |
| `canonical-json` | One envelope for all seasons | Immediate; the provider supplies finality | UTC date | None |
| `mlb-statsapi` | One schedule per season (`{season}` in the URL) | Immediate once the provider marks the game final | `officialDate` | Completed World Series game |

The Rust trait `SourceAdapter` in `crates/rating-core/src/adapters/mod.rs` serves the importer, configuration validation, and the tuners' history checks:

- `kind()`: the registry key, equal to `source.kind`.
- `history_urls(cfg, from, through)`: URLs whose documents, in order, cover seasons `from` through `through`. Whole-file sources return one URL; per-season sources return one URL per season.
- `parse(documents, cfg, warn)`: normalized but unvalidated games from downloaded or `--input` documents; unfinished games have no result.
- `strict_source_ids()`: whether each game's source ids must be listed in the franchise era for its season.
- `season_incomplete(games, season)`: a reason when the season lacks the league's completion marker.
- `validate_config(cfg)`: adapter-specific configuration checks; the default accepts every configuration.

The registry `ADAPTERS` in the same file lists `NflverseCsv`, `CanonicalJson`, and `MlbStatsApi`, in the modules `nflverse_csv.rs`, `canonical_json.rs`, and `mlb_statsapi.rs`. `adapter_for(kind)` returns an adapter or fails with `Unknown source adapter`.

The TypeScript interface `SourceAdapter` in `apps/web/src/adapters/types.ts` serves the browser and the backtest:

- `kind`, `strictSourceIds`, and optional `sourceStartTimes` (the adapter supplies each game's UTC start time).
- `resultPolicy`: `summary` for the status card and `detail` for **How these predictions work**.
- Optional `validateConfig(config)`: an adapter-specific configuration problem, or `null`.
- `seasonUrls(config, season)`: URLs to download, in order, for one season.
- `parse(documents, config)`: unvalidated game records from the downloaded documents.
- `isResultEligible(game, now)`: whether a result may train the model at `now`.
- `pregameDay(game)`: a `YYYY-MM-DD` day key; a completed game's pregame model uses only results with an earlier key.

The registry in `apps/web/src/adapters/index.ts` lists `nflverseCsv`, `canonicalJson`, and `mlbStatsApi`, in the modules `nflverse-csv.ts`, `canonical-json.ts`, and `mlb-statsapi.ts`. `adapterFor(config)` returns the configuration's adapter, and `isSourceKind(kind)` validates configurations.

To support an upstream API whose response differs from the existing formats, add a Rust module under `crates/rating-core/src/adapters/` and register it in `ADAPTERS`, add a TypeScript module under `apps/web/src/adapters/` and register it in `index.ts`, and add a shared parity fixture ([Tests and parity fixtures](#tests-and-parity-fixtures)). The provider endpoint must permit browser requests through CORS. Keep source parsing separate from ratings. Do not guess a mapping for an unknown team, turn a missing result into a tie, or reuse a previous season's priors when a new season begins. The canonical adapter treats every non-null outcome as final, so its provider must publish only final results.

## Provider response

The generic `canonical-json` HTTPS endpoint returns all relevant historical seasons and the current season in this envelope, which may include additional metadata:

```json
{
  "schema_version": 2,
  "league": "example",
  "games": [
    {
      "id": "2026-001",
      "league": "example",
      "season": 2026,
      "date": "2026-09-01",
      "time": "19:00",
      "timezone": "Europe/London",
      "phase": "regular",
      "round_label": "REG",
      "round": 1,
      "home_team": "TEAM_A",
      "away_team": "TEAM_B",
      "home_source_id": "TEAM_A",
      "away_source_id": "TEAM_B",
      "neutral": false,
      "result": "home_win"
    }
  ]
}
```

- `result`: `"home_win"`, `"away_win"`, `"tie"`, or `null` for a game without a confirmed final outcome. The provider is responsible for finality; provisional live outcomes must be null.
- `phase`: `"regular"` or `"postseason"`.
- `season`: a consistent integer season label, also for games played in the next calendar year.
- `date`, `time`, `timezone`: local calendar date, optional HH:mm game start time, and IANA time zone. The date remains required when the time is null.
- `start_time_utc`: omit this field in provider responses. Both adapters derive it from the required local date, optional time, and timezone. The Rust importer ignores a supplied value; the browser validates any supplied value's format before replacing it with the derived timestamp.
- `round`: positive integer round index, used only for display.
- `round_label`: source-specific round label. Postseason display labels may use it; ratings do not.
- `neutral`: true disables home advantage.
- `home_source_id`, `away_source_id`: original provider abbreviations/IDs. They must resolve to the matching stable franchise through configuration; use the stable IDs when your upstream uses them.
- `id`: unique game ID within the league, stable across updates and score corrections.

Normalization sorts by `(season, start_time_utc, id)`, using IANA timezone rules for historical daylight saving offsets. A missing date is a source error and aborts the import without replacing the saved history. A missing time (omitted, null, or empty) or a time in a daylight saving gap uses local midnight. A repeated time during a daylight saving overlap uses the earlier UTC occurrence. The importer prints warnings to stderr identifying each affected game and the fallback or selected occurrence. If local midnight itself does not exist, import fails. Historical output discards the original local fields. The current-season browser retains them for display and provider-specific result eligibility.

The importer accepts this exact envelope from HTTPS or through its `--input` file option. The browser app downloads from the provider itself and saves only the current season. The Elo program reads only the historical JSON, so it works with any provider.

## Historical output

The importer writes `data/<league>/history.json` with these fields:

| Field | Contents |
| --- | --- |
| `schema_version` | `3` |
| `league` | League ID matching the configuration |
| `fetched_at` | RFC 3339 timestamp of the import |
| `source_url` | Configured provider URL |
| `from_season` | First included season |
| `through_season` | Last included season, inclusive |
| `teams` | Franchise identity registry described in [Configuration](#configuration) |
| `games` | Array of normalized historical game records |

Each game has the following structure:

```json
{
  "id": "2026-001",
  "league": "example",
  "season": 2026,
  "start_time_utc": "2026-09-01T18:00:00Z",
  "phase": "regular",
  "round_label": "REG",
  "round": 1,
  "home_team": "TEAM_A",
  "away_team": "TEAM_B",
  "home_source_id": "TEAM_A",
  "away_source_id": "TEAM_B",
  "neutral": false,
  "result": "home_win"
}
```

The identity, season, phase, venue, and result fields follow the [provider response](#provider-response) format.
Historical records store `start_time_utc` instead of the provider's `date`, `time`, and `timezone` fields.
The importer serializes this required timestamp as RFC 3339 UTC with `Z`. Programs that read the file use it as the
game's start time, require an explicit offset, and sort games by `(season, start_time_utc, id)`.
Missing, null, or malformed timestamps are errors.

The browser and Node backtest only check the history file's hash; they get game records from the provider.

Generate the Elo seed from the published history and configuration so its SHA-256 hashes match those files.

## League registry

`leagueIds` in `apps/web/src/leagues.ts` is the browser's only list of league ids (`['nfl', 'mlb']`). Each id names its published configuration, `config/<id>.json`. Adding an id adds the league to the header's **League** switcher and makes `#<id>` a valid address hash. Registry order breaks ties in the default-league rule ([Windows](#windows)). The rest of the app has no league ids or labels; it reads them from the configuration and the league's adapter.

The Vite build (`apps/web/vite.config.ts`) serves and publishes every JSON file under the repository's `config/` and `data/` directories, as `config/<id>.json`, `data/<id>/history.json`, and `data/<id>/elo-<season>.json`. If a program writes to another data directory, copy its outputs into `data/<id>/` before building.

## Browser storage

Browser storage is namespaced by league id, so a new league needs no storage changes:

| Storage | Key | Contents |
| --- | --- | --- |
| IndexedDB database `game-results-prediction` (version 1), object store `entries` | `game-results-prediction:<id>:model-v1` | Model snapshot |
| IndexedDB database `game-results-prediction` (version 1), object store `entries` | `game-results-prediction:<id>:current-<season>` | Current-season cache |
| localStorage | `game-results-prediction:<id>:last-attempt` | Last update attempt, for the one-minute cooldown |
| localStorage | `game-results-prediction:league` | Remembered league |
| Web Lock | `game-results-prediction:<id>:refresh` | Allows one refresh per league at a time |

The IndexedDB module is `apps/web/src/persistence.ts`, and `apps/web/src/small-store.ts` holds the small localStorage keys. A web version change (`APP_VERSION`) invalidates saved snapshots. Startup removes legacy localStorage snapshot and cache entries from earlier versions.

## Generate and publish data

Every command-line program and the backtest take `--league <id>`, `--config-dir <dir>` (default `config`), and `--data-dir <dir>` (default `data`). [Data and configuration](../DEVELOPMENT.md#data-and-configuration) lists every option and the commands for the shipped leagues; [Tuning](../DEVELOPMENT.md#tuning), [Model evaluation](../DEVELOPMENT.md#model-evaluation), and [Backtesting](../DEVELOPMENT.md#backtesting) cover evaluation. For a new league, generate its history and current-season seed:

```text
cargo run --release -p history-importer -- --league <id>
cargo run --release -p elo-ratings -- --league <id>
```

To publish it, add the same two invocations to the `Generate league data` step of [`pages.yml`](../.github/workflows/pages.yml), after the existing leagues and one command per line, so a failure fails the build:

```yaml
      - name: Generate league data
        run: |
          cargo run --release -p history-importer -- --league nfl
          cargo run --release -p elo-ratings -- --league nfl
          cargo run --release -p history-importer -- --league mlb
          cargo run --release -p elo-ratings -- --league mlb
          cargo run --release -p history-importer -- --league <id>
          cargo run --release -p elo-ratings -- --league <id>
```

The workflow's monthly scheduled build then publishes the league's new-season seed after its `season_rollover_month`.

## Tests and parity fixtures

Rust and TypeScript implement the adapters and the Bayesian model separately. Shared fixtures keep them in agreement:

| Fixture | Rust test | TypeScript test |
| --- | --- | --- |
| `crates/rating-core/tests/fixtures/bayesian-parity.json` | `crates/rating-core/tests/bayesian.rs` | `apps/web/test/bayesian-parity.test.ts` |
| `crates/rating-core/tests/fixtures/mlb-statsapi.json` (source documents and expected normalized games) | `crates/rating-core/tests/mlb_statsapi.rs` | `apps/web/test/mlb-statsapi.test.ts` |

A new adapter needs its own fixture of provider documents and expected games, consumed by tests in both languages. `apps/web/test/leagues.test.ts` checks that every registry league has a published configuration naming its id.

## Limits

The game format and likelihood describe a single game between two teams. Multi-leg competitions, aggregate scores, and matches with more than two teams would need both to be extended.
