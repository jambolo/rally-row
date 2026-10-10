# History import

`import-history` downloads a league's completed historical seasons through its source adapter and saves them to
`data/<id>/history.json`. Every other command-line tool reads that file:
[`generate-preseason-seed`](preseason-seed.md) builds preseason seeds from it, the tuners and
[`simulate-season`](season-simulation.md) replay it, and [`evaluate-model`](model-evaluation.md) checks the
simulation against it. The importer never writes current-season data; the browser
downloads the current season itself.

## Running the tool

The importer needs network access unless `--input` supplies saved provider files, or the saved history already
holds every season.

```powershell
cargo run --release -p import-history -- --league nfl
cargo run --release -p import-history -- --league mlb

# Import through an earlier season into a separate data directory
cargo run --release -p import-history -- --league nfl --through-season 2024 --data-dir data-2024

# Read a saved provider file instead of downloading
cargo run --release -p import-history -- --league nfl --input games.csv

# Download every season again, for example to pick up provider corrections
cargo run --release -p import-history -- --league mlb --refresh
```

| Option | Meaning |
| --- | --- |
| `--league <id>` | Required. Reads `<config-dir>/<id>.json`, whose `id` must equal `<id>`. Ids use letters, digits, and hyphens. |
| `--config-dir <dir>` | Configuration directory; default `config`. |
| `--data-dir <dir>` | Data directory; default `data`. Writes `<dir>/<id>/history.json`. |
| `--through-season <year>` | Last imported season; default the current season minus one. It must be at least `history_start` and before the current season. |
| `--input <file>...` | Reads saved provider files instead of downloading ([Saved provider files](#saved-provider-files)). |
| `--refresh` | Downloads every season, including those the saved history already holds. Can't be combined with `--input`. |

The imported history always starts at the configuration's `history_start`. A league's current season is the UTC year,
minus one before its `season_rollover_month` ([Data and configuration](../DEVELOPMENT.md#data-and-configuration)).

Warnings go to stderr, and stdout gets one line naming the saved file:
`Saved <games> games, seasons <first>–<last>, to <path>`. When the saved history already holds every season, the
importer downloads nothing, leaves the file unchanged, and prints
`<path> already holds seasons <first>–<last>; nothing to download`.

## What the import does

1. Keeps the seasons of the saved `history.json` when it is current and doesn't go past `--through-season`. It is
   current when it has this schema, league, source URL, `history_start` and franchise registry, and its games still
   pass the checks in step 4 under the configuration. The importer prints
   `Keeping seasons <first>–<last> from <path>; downloading <first>–<last>` to stderr. `--input` and `--refresh`
   skip this step.
2. Downloads the documents the league's adapter names for the seasons it didn't keep, through `--through-season`:
   one nflverse games CSV for every season (NFL) or one Stats API schedule per season (MLB). The NFL CSV covers every
   season, so it is downloaded in full whenever any season is missing. Each request times out after 60 seconds and
   is tried up to three times.
3. Parses them with the league's [source adapter](extending.md#source-adapters), which maps provider team ids to
   franchises through the configured `aliases` and eras, derives each game's outcome from its score and discards
   the score, and converts the local start time to UTC.
4. Validates every parsed game against the configuration: unique game ids, the configured league, known
   franchises whose source ids are valid in the game's season, a positive round, a `regular` or `postseason`
   phase, and ties only in phases listed in `ties_allowed_in`.
5. Adds the parsed games of the downloaded seasons to the kept games, validates them together, and checks that each
   season from `history_start` through `--through-season` has a completed game and the adapter's completion marker:
   a completed Super Bowl (NFL) or a completed World Series game (MLB). `canonical-json` has no completion marker.
6. Writes the file, with the SHA-256 of the configuration's franchise registry as `teams_sha256`.

The adapters print a warning, prefixed `Warning:`, for each listing they skip or repair, and the import still
succeeds:

| Warning | Fallback |
| --- | --- |
| Missing start time, or a time in a daylight saving gap | Local midnight |
| Repeated local time during a daylight saving overlap | The earlier UTC occurrence |
| `Game <pk>: final status without scores; ignored` (MLB) | The listing is skipped |

## Failures

The importer replaces `history.json` only after every step succeeds, and it replaces the file atomically, so a
failed or interrupted run leaves the previous file in place. A lock file, `data/<id>/history.lock`, keeps two runs
from writing the same league at once; the second fails with
`Another process is writing this output; try again after it finishes`.

| Error | Cause |
| --- | --- |
| `Historical range must end before the current season` | `--through-season` is before `history_start` or not before the current season |
| `Source missing completed season <year>; previous file has been kept` | A season has no completed game |
| `Season <year> has no completed Super Bowl; previous file has been kept` | An NFL season lacks its completion marker; MLB reports `no completed World Series` |
| `The <kind> adapter expects exactly one document, got <n>` | More than one `--input` file for a source that publishes one document |

## Saved provider files

`--input` takes one or more files, listed after one `--input` or each after its own. They must be the documents a
download would return, in the same format:

| `source.kind` | Files |
| --- | --- |
| `nflverse-csv` | One nflverse games CSV covering every season |
| `mlb-statsapi` | One Stats API schedule JSON per season, `history_start` through `--through-season` |
| `canonical-json` | One [provider envelope](extending.md#provider-response) |

Games outside the imported seasons are dropped. Saved files make an import repeatable and let tests run offline.

## Output

[Historical output](extending.md#historical-output) describes the file format. Games are sorted by season, UTC start
time, and id. The other tools reject a history whose schema, league, or `teams_sha256` no longer match the
configuration, and ask you to rerun the importer.

Rerun the importer when a new season completes and after any change to franchise identities, aliases, or divisions.
Then rerun [`generate-preseason-seed`](preseason-seed.md), whose seed records the history's hash, and
[`simulate-season`](season-simulation.md), whose file [`evaluate-model`](model-evaluation.md) scores. The Pages
workflow starts without saved data, so it downloads every league's full history before each build.
