# AGENTS.md

This file provides guidance to AI when working with code in this repository.

Rally Row predicts NFL and MLB game outcomes. A Rust workspace generates static league data offline, and a
browser-only React app (`apps/web`) fits a Bayesian model in a Web Worker. There is no backend: GitHub Pages
serves the app with `config/` and `data/` as static JSON. See [DEVELOPMENT.md](DEVELOPMENT.md) for the full
command reference, release process, tuning and backtesting.

## Commands

Run from the repository root. The docs write commands in PowerShell, and CI runs on Ubuntu and Windows.

```sh
pnpm -C apps/web install --frozen-lockfile

# Generate local data (needs network; output in data/ is gitignored). Required before the app works locally.
cargo run --release -p history-importer -- --league nfl
cargo run --release -p elo-ratings -- --league nfl      # repeat both for --league mlb

pnpm -C apps/web dev                                    # served under /rally-row/; VITE_BASE overrides

# Checks (lint and format run in CI only on pull requests)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
pnpm -C apps/web lint
pnpm -C apps/web format:check
pnpm -C apps/web test
pnpm -C apps/web build                                  # tsc + vite build

# Single tests
pnpm -C apps/web exec vitest run test/model.test.ts -t "two-way"
cargo test -p rating-core --test bayesian
cargo test -p rating-core <test_name_substring>
```

`pnpm -C apps/web run docs` generates TypeDoc; plain `pnpm docs` hits pnpm 12's built-in command.
Rust and Prettier both use a 132-column line width.

## Architecture

### Offline data pipeline (Rust)

`crates/rating-core` is the shared library: config and data contracts (serde structs in `lib.rs`), source
adapters, Elo replay, the Bayesian model, tuning validation and the tuners' search and selection rule (`tuning.rs`),
season-by-season prediction from earlier UTC dates (`walk_forward.rs`), and forecast scores (`scoring.rs`). The
binaries in `apps/` are thin CLIs over it, so logic two tools share belongs in the library.

1. `history-importer --league <id>` downloads the provider's history through the league's source adapter and
   writes `data/<id>/history.json`. It replaces the file only if every season is complete, so a failure leaves the
   previous file in place.
2. `elo-ratings --league <id>` replays history and writes `data/<id>/elo-<season>.json`. The seed embeds SHA-256
   hashes of the config and history bytes. The browser and the backtest reject a seed whose hashes don't match,
   so rerun `elo-ratings` after any config or history change.
3. `elo-tune` and `bayes-tune` are offline parameter searches. They never modify config.
4. `evaluate-model` scores the model's predictions, made with the configured settings, on held-out seasons, which
   tuning never uses. It compares Bayesian predictions with in-season Elo ratings.

The custom `projectData` plugin in `apps/web/vite.config.ts` serves the root `config/` and `data/` directories
in dev, and copies them into the build.

### Browser runtime (`apps/web/src`)

- `main.tsx` → `App.tsx`. The app chooses the startup league in `selection.ts` (hash, then remembered choice,
  then the in-season default from config `windows`) and calls `startSession` in `session.ts`.
- `session.ts` loads the league's persisted entries from IndexedDB (`persistence.ts`). It enforces a one-minute
  refresh cooldown in localStorage (`small-store.ts`) and serializes refreshes across tabs with Web Locks. It then
  runs `refresh.worker.ts`.
- The worker (`refresh-worker.ts`) builds a `PredictionService` (`service.ts`) over an in-memory `Store`. The
  service:
  - downloads the current season through the source adapter;
  - filters results the adapter deems eligible (`provider.ts`);
  - fits the posterior (`model.ts`);
  - writes the model snapshot (`snapshot.ts`) and the game cache (`storage.ts`) into that store.

  The worker posts back `PublicState`, the `Posterior`, and the store's entries, and the page persists them.
- Matchup predictions are computed on the page from the returned `Posterior` (`predict` in `model.ts`, derived
  in `App.tsx` with `useMemo`, never stored).

### Invariants that span files

- **Rust/TypeScript parity.** The Bayesian model (`crates/rating-core/src/bayesian.rs` ↔
  `apps/web/src/model.ts`) and the source adapters (`crates/rating-core/src/adapters/` ↔
  `apps/web/src/adapters/`) are implemented twice. Both test suites consume the shared fixtures in
  `crates/rating-core/tests/fixtures/`: `bayesian-parity.json` and `mlb-statsapi.json`. Change both
  implementations together and keep the fixtures passing on both sides. The config schema is also defined twice:
  zod in `contracts.ts` and serde in `rating-core/src/lib.rs`.
- **League-agnostic code.** League-specific values live in `config/<id>.json`: teams, aliases, display
  vocabulary, windows, model settings, and `ties_allowed_in`. Provider rules (finality, result eligibility,
  pregame day, completion marker) live only in source adapters. No code outside the adapter modules and their
  registries compares `source.kind`, and `leagues.ts` is the only place league ids appear in app source. Adding
  a league follows [docs/extending.md](docs/extending.md) and includes new lines in `.github/workflows/pages.yml`.
- **Franchise identity.** Team ids are stable franchises with `eras` and provider `aliases`. Renames and
  relocations map to the existing id ([docs/team-history.md](docs/team-history.md)).
- **Saved-model invalidation.** A snapshot stores `app_version`, which must equal `APP_VERSION` from
  `apps/web/package.json`, so any web version bump discards saved models. Bump the web version before deploying
  changes to prediction math or the snapshot format.

## Branches and versions

The repo uses git-flow. `develop` is the integration branch. `release/vX.Y.Z` branches merge into `master`, and
the CD workflow then tags them and merges `master` back into `develop`. Pages deploys from `master`.

- **Project version** (`Cargo.toml` `[workspace.package].version`, tag `vX.Y.Z`): bump it for every release,
  including web-only or docs-only releases, and refresh `Cargo.lock`.
- **Web version** (`apps/web/package.json`, tag `web-vX.Y.Z`): bump it for app changes. A web bump also requires
  a project bump.

## Docs map

- `README.md`: user-facing behavior of the app, described in detail. Keep it in sync with UI changes.
- `DEVELOPMENT.md`: setup, CLI options, checks, releases, tuning, model evaluation, backtesting.
- `docs/model.md`: statistical methodology (Elo, the Davidson–Bradley–Terry Bayesian model, two-way
  moneylines, tuning, model evaluation).
- `docs/extending.md`: adding leagues and source adapters, and the parity-fixture table.
