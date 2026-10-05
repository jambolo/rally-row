# Development

## Local setup

Use Node LTS, stable Rust with `rustfmt` and `clippy`, and the pnpm version pinned in
[`apps/web/package.json`](apps/web/package.json). On Ubuntu, Rust dependencies also need
`build-essential`, `pkg-config`, and `ca-certificates`.

Run commands from the repository root. The importer downloads each league's history, so these commands need
network access; Elo then writes each league's current-season seed:

```powershell
pnpm -C apps/web install --frozen-lockfile
cargo run --release -p history-importer -- --league nfl
cargo run --release -p elo-ratings -- --league nfl
cargo run --release -p history-importer -- --league mlb
cargo run --release -p elo-ratings -- --league mlb
pnpm -C apps/web dev
```

The MLB import requests one Stats API schedule per season. It prints
`Warning: Game <pk>: final status without scores; ignored` for each listing the provider marks final without
scores; the import skips those listings and still succeeds.

Subsequent runs need only `pnpm -C apps/web dev` unless data or configuration changes.
Local Vite and Pages use `/rally-row/`. Set `VITE_BASE` to override it locally.

Dependency install-script permissions and release-age exceptions are in
[`apps/web/pnpm-workspace.yaml`](apps/web/pnpm-workspace.yaml).

## Project structure

| Directory | Purpose |
| --- | --- |
| `apps/history-importer/` | Rust history importer: downloads a league's provider data (or reads saved files) and writes `data/<id>/history.json` |
| `apps/elo-ratings/` | Rust Elo executable: writes a league's preseason seed `data/<id>/elo-<season>.json` |
| `apps/elo-tune/` | Offline Elo parameter search and held-out evaluation |
| `apps/bayes-tune/` | Offline Bayesian parameter search with fixed Elo and held-out evaluation |
| `apps/evaluate-model/` | Offline scoring of Bayesian and in-season Elo predictions on held-out seasons, made with the configured settings |
| `apps/web/src/` | Browser app: league registry (`leagues.ts`) and league selection (`selection.ts`), contracts, source adapters and their registry (`adapters/`), Bayesian model, service and refresh worker, IndexedDB persistence (`persistence.ts`), small-key localStorage (`small-store.ts`), React interface |
| `apps/web/test/` | Vitest tests: model, adapters and parity fixtures, league selection and switching, storage, startup/cache, identity |
| `apps/web/scripts/` | Node backtest |
| `crates/rating-core/` | Shared Rust library: data contracts, league configuration, source adapters and their registry (`src/adapters/`), file IO, Elo replay, Bayesian model, tuning validation and parameter search, season-by-season prediction, forecast scores |
| `crates/rating-core/tests/fixtures/` | Rust/TypeScript parity fixtures: `bayesian-parity.json` (Bayesian fit) and `mlb-statsapi.json` (MLB adapter) |
| `crates/test-support/` | Rust test helpers shared by every crate's tests: league configs, history fixtures, and command-line checks the tools have in common |
| `config/` | League configurations: `nfl.json`, `mlb.json` |
| `docs/` | Statistical model, extension guide, franchise history, branding |

## Data and configuration

Each league has one configuration file in `config/`, named after its league id:

| Setting | NFL | MLB |
| --- | --- | --- |
| Configuration | [`config/nfl.json`](config/nfl.json) | [`config/mlb.json`](config/mlb.json) |
| Source (`source.kind`) | nflverse games CSV, one file for all seasons (`nflverse-csv`) | MLB Stats API schedule, one request per season (`mlb-statsapi`) |
| History starts (`history_start`) | 2002 | 1998 |
| Season rollover month (`season_rollover_month`) | April (`4`) | January (`1`) |

A league's current season is the UTC year, minus one before its rollover month: NFL games from January
through March belong to the previous season, and MLB seasons are calendar years. Season defaults use the
current UTC date. Generated files are ignored by Git: `data/<id>/history.json` from the importer and
`data/<id>/elo-<target-season>.json` from Elo.

```powershell
cargo run --release -p history-importer -- --league nfl --through-season 2025
cargo run --release -p elo-ratings -- --league nfl --target-season 2026
cargo run --release -p history-importer -- --league mlb --through-season 2025
cargo run --release -p elo-ratings -- --league mlb --target-season 2026
```

All command-line programs and the backtest take these options:

| Option | Programs | Meaning |
| --- | --- | --- |
| `--league <id>` | All | Required. Reads `<config-dir>/<id>.json`, whose `id` must equal `<id>`. Ids use letters, digits, and hyphens. |
| `--config-dir <dir>` | All | Configuration directory; default `config`. |
| `--data-dir <dir>` | All | Data directory; default `data`. Files are read and written under `<dir>/<id>/`. |
| `--through-season <year>` | `history-importer` | Last imported season; default the current season minus one. It must be at least `history_start` and before the current season. |
| `--input <file>...` | `history-importer` | Reads saved provider files instead of downloading: one nflverse games CSV (NFL), one Stats API schedule JSON per season from `history_start` through `--through-season` (MLB), or one canonical JSON envelope. |
| `--target-season <year>` | `elo-ratings` | Seed season; default the current season. |

The tuners, the model evaluation, and the backtest take further options; see [Tuning](#tuning),
[Model evaluation](#model-evaluation), and [Backtesting](#backtesting).

The importer replaces `data/<id>/history.json` only when every imported season has completed games and the
league's completion marker (a completed Super Bowl for NFL, a completed World Series game for MLB); on
failure the previous file stays. `elo-ratings` embeds SHA-256 hashes of the configuration and history in
its seed, and the browser and backtest reject a seed whose hashes do not match.

- New season: rerun the importer and Elo for the league, then rebuild the site. Import only historical seasons.
- Model configuration or history changes: rerun Elo to update the linked hashes.
- Franchise identity or alias changes: rerun the importer, then Elo.

Rebuild the site to publish updated data and configuration. The Vite build copies every JSON file under
`config/` and `data/` into the site as `config/<id>.json`, `data/<id>/history.json`, and
`data/<id>/elo-<season>.json`. To add a league, see [Add a league](docs/extending.md).

## Checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
pnpm -C apps/web lint
pnpm -C apps/web format:check
pnpm -C apps/web test
pnpm -C apps/web build
```

Generate TypeDoc with `pnpm -C apps/web run docs`; `run` avoids pnpm 12's built-in `docs` command.
Clear the site's saved data to force a fresh model build during development.

## Building and publishing

```powershell
pnpm -C apps/web build      # apps/web/dist
pnpm -C apps/web preview
```

For hosting at a domain root:

```powershell
$env:VITE_BASE = '/'
pnpm -C apps/web build
```

Workflows in [`.github/workflows/`](.github/workflows/):

- `ci-rust.yml` and `ci-web.yml`: builds and tests on Ubuntu and Windows for pushes to `master`,
  `develop`, and `release/**`, and for pull requests. Lint and formatting run on pull requests;
  Codecov uploads run on `develop` with `rust` and `web` flags.
- `cd.yml`: runs when either version manifest changes on `master`. After builds and tests, creates
  missing project and web tags, then merges `master` into `develop` if it created a tag.
- `pages.yml`: regenerates history and Elo for every league (`--league nfl`, then `--league mlb`), builds
  the site with `data/` and `config/`, and deploys `apps/web/dist`. Runs on `master` pushes, manual
  dispatch, and the first Tuesday of each month at 09:17 UTC; the monthly run publishes each league's
  new-season seed after its configured rollover month. A new league needs its own importer and Elo lines
  in the workflow. Deployment runs separately from CD and does not wait for a release to succeed.

### Versions and releases

| Scope | Version source | Tag | GitHub release title |
| --- | --- | --- | --- |
| Project | `Cargo.toml`: `[workspace.package].version` | `vX.Y.Z` | `Rally Row X.Y.Z` |
| Web app | `apps/web/package.json`: `version` | `web-vA.B.C` | `Rally Row Web A.B.C` |

Bump the project version for every release, including web-only, workflow, or documentation changes,
and refresh the workspace versions in `Cargo.lock`. All Rust crates inherit the project version.
Bump the web version for app changes; every web release also requires a project bump. Data-only
refreshes need neither bump.

The web version also controls model-cache compatibility. Bump it before deploying prediction or
snapshot-format changes, including dependencies that affect either. Every web version change
invalidates saved models; a project-only bump does not. Pages does not enforce this requirement.

Merge releases into `master` for CD to tag them. Create GitHub releases manually using the titles
above, cross-reference the project and app versions in their notes, and mark the project release
as **Latest**. CD does not validate version increases, required project bumps, or existing tag targets.

Preserve published tags, including historical `cli-v*` and `web-v*` tags and releases. The first `v*`
project release must exceed `1.0.0` rather than reuse the historical `cli-v1.0.0` version.

## Tuning

Both tuners run offline against `data/<id>/history.json` without modifying inputs, so import the league's
history first. `elo-tune` searches Elo parameters; `bayes-tune` searches Bayesian parameters with Elo
fixed. Methods and report contents:
[Elo tuning](docs/model.md#elo-only-tuning) and
[Bayesian tuning](docs/model.md#bayesian-parameter-tuning).

```powershell
cargo run --release -p elo-tune -- --league nfl
cargo run --release -p bayes-tune -- --league nfl
cargo run --release -p elo-tune -- --league mlb
cargo run --release -p bayes-tune -- --league mlb

# Save reports
cargo run --release -p elo-tune -- --league nfl --report-dir target
cargo run --release -p bayes-tune -- --league nfl --report-dir target
cargo run --release -p elo-tune -- --league mlb --report-dir target
cargo run --release -p bayes-tune -- --league mlb --report-dir target

# Path and split overrides (also supported by bayes-tune)
cargo run --release -p elo-tune -- --league nfl --config-dir config --data-dir data --tune-start 2010 --tune-end 2022 --test-end 2025
cargo run --release -p elo-tune -- --league mlb --config-dir config --data-dir data --tune-start 2006 --tune-end 2022 --test-end 2025
```

JSON reports go to stdout; progress goes to stderr. `--report-dir` also writes
`elo-tuning-report-<league>-<YYYY-MM-DD>.json` or `bayes-tuning-report-<league>-<YYYY-MM-DD>.json`
using the UTC run date. Same-day runs replace the corresponding report. The MLB `bayes-tune` run takes
about 4 minutes in a release build.

Split defaults come from `elo_tune` or `bayes_tune` in the league configuration; `bayes_tune` falls
back to `elo_tune`. Neither shipped configuration sets `bayes_tune`, so both tuners default to:

| Seasons | NFL | MLB |
| --- | --- | --- |
| Warm-up | 2002–2009 | 1998–2005 |
| Tuning | 2010–2022 | 2006–2022 |
| Held-out | 2023–2025 | 2023–2025 |

CLI options override individual boundaries. Without configuration defaults, all three boundaries are
required. Splits require at least one warm-up season, two tuning seasons, and one later completed
held-out season.

Each tuner starts from the league's `tuning_grids.elo` or `tuning_grids.bayesian` grid when the
configuration has one, else from its built-in default grid. NFL uses the default grids for both tuners.
MLB uses the default Elo grid and this configured Bayesian grid of 27 combinations, because MLB's rare ties
are not stationary across its history
(see [Bayesian tuning](docs/model.md#bayesian-parameter-tuning)):

| Parameter | MLB `tuning_grids.bayesian` values |
| --- | --- |
| `prior_sd_elo` | 25, 35.35533905932737, 50 |
| `tie_prior_games` | 10000, 1000000, 100000000 |
| `tie_prior_rate` | 0.000001, 0.00003, 0.001 |

Reports record the grid source and the evaluated ranges. When selected parameters lie on a searched
boundary, the report notes ask you to extend `tuning_grids` in the league configuration and rerun before
adopting them. Neither tuner changes the configuration.

After applying tuned parameters, regenerate Elo seeds and rebuild the site.

## Model evaluation

`evaluate-model` scores the model's predictions, made with the configured (tuned) settings, on a league's
held-out seasons. It compares two predictors on the same games: the Bayesian model and Elo ratings updated after
every game. It uses the held-out seasons because the tuners never use them to select parameters. Like the tuners,
it runs offline against `data/<id>/history.json`. Method and report contents:
[Model evaluation](docs/model.md#model-evaluation); what the scores mean and how to compare them:
[Reading the scores](docs/model.md#reading-the-scores).

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

Unlike the tuners, `evaluate-model` prints a human-readable summary table to stdout by default. Its layout is not
a stable format; programs should pass `--json`, which prints the full JSON report to stdout instead of the
summary. Progress messages go to stderr either way. `--report-dir` also writes the JSON report, in either mode, to
`model-evaluation-report-<league>-<YYYY-MM-DD>.json` using the UTC run date. Split defaults and overrides
(`--tune-start`, `--tune-end`, `--test-end`) match `bayes-tune`: `bayes_tune`, falling back to `elo_tune`, so
both shipped leagues evaluate 2023–2025. The tool validates the split as the tuners do, and its notes flag any
held-out season that falls within a configured tuning range. The MLB run takes a few seconds in a release
build. The tool scores whatever the configuration holds, so apply tuned parameters before running it.

## Backtesting

The Node backtest reads a league's local history and Elo seed, then downloads the evaluated season's games
through the league's source adapter, so it requires network access. `--season`
(default the current season) selects the evaluated season; the seed must be trained through the previous
season. Each UTC date is predicted from earlier UTC dates only.

```powershell
# Evaluate 2025 with priors trained through 2024
cargo run --release -p history-importer -- --league nfl --through-season 2024 --data-dir data-backtest
cargo run --release -p elo-ratings -- --league nfl --target-season 2025 --data-dir data-backtest
pnpm -C apps/web backtest -- --league nfl --season 2025 --data-dir data-backtest
cargo run --release -p history-importer -- --league mlb --through-season 2024 --data-dir data-backtest
cargo run --release -p elo-ratings -- --league mlb --target-season 2025 --data-dir data-backtest
pnpm -C apps/web backtest -- --league mlb --season 2025 --data-dir data-backtest
```

For MLB, a late game whose UTC start falls on the day after its official date is predicted in the next UTC
date's batch, so earlier games of the same official date can inform it; see
[Pregame reconstruction and backtesting](docs/model.md#pregame-reconstruction-and-backtesting).

## References

- [Browser usage](README.md)
- [Statistical model and backtesting methodology](docs/model.md)
- [Adding leagues, source adapters, and data formats](docs/extending.md)
- [Franchise identities and aliases](docs/team-history.md)
- [Branding and messaging](docs/branding.md)
