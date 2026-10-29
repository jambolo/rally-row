# Rally Row: code review of `develop`

As of October 10, 2026.

## Summary

The review covers the seven commits on `develop` since release v1.5.0, which change 62 files. It found 15 issues, all since resolved. Five affect behavior: three in the release and deployment process and two in `import-history`. One is a misleading error message, one is a slow and brittle test, two are documentation errors, and the rest are maintenance and efficiency cleanups.

| # | Finding | Where | Impact |
| --- | --- | --- | --- |
| 1 | The scheduled build runs once per rollover month and is never retried | `.github/workflows/pages.yml` | Resolved; was Medium: a failed build leaves a new season's seed unpublished for months |
| 2 | Kept seasons don't record which adapter parsed them | `apps/import-history/src/main.rs` | Resolved; was Medium: adapter fixes don't reach local history without `--refresh` |
| 3 | The web version isn't bumped for the seed format change | `apps/web/package.json` | Resolved; was Medium: breaks the documented release invariant |
| 4 | A rejected saved history is re-downloaded without a reason | `apps/import-history/src/main.rs` | Resolved; was Low: unexplained full downloads |
| 5 | The schedule reads `develop`'s configuration but builds `master` | `.github/workflows/pages.yml` | Resolved; was Low: wrong months when the branches' configurations differ |
| 6 | `replay_seeds` names the wrong season in its error | `crates/rating-core/src/lib.rs` | Resolved; was Low: misleading message |
| 7 | The new import tests sleep through retries and match reqwest's error text | `apps/import-history/tests/cli.rs` | Resolved; was Low: slower tests that break on a reqwest upgrade |
| 8 | Four copies of the "history is current" check | `walk_forward.rs`, `build_seed`, `tuning::validate`, `import-history` | Resolved; was Low: maintenance |
| 9 | `replay_seeds` returns its results through a callback | `crates/rating-core/src/lib.rs` | Resolved; was Low: maintenance |
| 10 | The seed schema version is an inline literal | `crates/rating-core/src/lib.rs` | Resolved; was Low: maintenance |
| 11 | `preseasons` filters and sorts the whole history for each season | `crates/rating-core/src/walk_forward.rs` | Resolved; was Low: runtime |
| 12 | Every replay builds an `Audit` per game and discards it | `crates/rating-core/src/lib.rs` | Resolved; was Low: allocations |
| 13 | `model.md` describes `evaluate-model`'s old season split | `docs/model.md` | Resolved; was Low: documentation |
| 14 | `teams_sha256` is documented as a hash of compact JSON | `docs/extending.md` | Resolved; was Low: documentation |
| 15 | A doc comment restates its function's name | `crates/rating-core/src/lib.rs` | Resolved; was Trivial |

## 1. The scheduled build is never retried (resolved)

The scheduled rebuild in `.github/workflows/pages.yml` (line 38) now dispatches only on the first Tuesday of a month that is some league's `season_rollover_month`. If that one build fails, nothing retries it until the next push to `master` or the next league's rollover month.

MLB rolls over in January. If the January build fails, whether from a Stats API or nflverse outage, a dropped GitHub cron run, or a missing completion marker, the new season's `elo-<season>.json` is never published, and the app shows an MLB seed error until the NFL's April rollover build. Under the old monthly schedule, the February build would have retried.

## 2. Kept seasons don't record which adapter parsed them (resolved)

`reusable()` in `apps/import-history/src/main.rs` (line 35) keeps saved seasons when the schema, league, source URL, teams hash, `history_start`, and game validation all match. Nothing records which adapter or parser version produced those seasons, and `HISTORY_SCHEMA_VERSION` covers only the file format, so an adapter fix never reaches kept seasons unless the user passes `--refresh`.

Suppose a fix to nflverse start times or round mapping, or to MLB `gamePk` deduplication, is merged. A local `import-history --league nfl` reports nothing to download or keeps the old seasons. The tuners, `simulate-season`, and the seeds then run on the stale parse locally, while CI, which always imports in full, publishes a different history and different seed hashes.

## 3. The web version isn't bumped for the seed format change (resolved)

The seed embedded in the saved snapshot changed: the seed schema went from 1 to 2 and `settings` was removed. The web version in `apps/web/package.json` (line 3) is still 1.5.0. AGENTS.md says to bump the web version, and with it the project version, before deploying changes to prediction math or the snapshot format.

Saved models are discarded anyway, but by accident: `snapshot.ts` validates `model.seed` with `seedSchema`, so every saved 1.5.0 snapshot now fails zod parsing and `readSnapshot` drops it, even though its `app_version` still matches. A release built from this branch would also reuse the existing version tags.

## 4. A rejected saved history is re-downloaded without a reason (resolved)

`reusable()` (line 43) turns every reason for rejecting the saved history into `None`: a parse or I/O error, a teams hash mismatch, a changed source URL, or a validation failure. The user sees only a full download.

If a user edits a team's display name, or `history.json` can't be read because of its permissions, the importer quietly downloads every season, which for MLB means every schedule since 1998. No message says why the saved seasons were dropped, so a slow or failing run is hard to diagnose.

## 5. The schedule reads `develop`'s configuration but builds `master` (resolved)

Scheduled workflows run on the default branch, `develop`. The schedule job (line 31) decides whether to dispatch from `config/*.json` on `develop`, but the build it dispatches runs on `master`.

If `develop` has an unreleased league, or a `season_rollover_month` that `master` doesn't have yet, the cron dispatches `master` builds in the wrong months, or skips the month in which `master`'s configuration actually rolls over. The site then misses that season's seed.

## 6. `replay_seeds` names the wrong season in its error (resolved)

`replay_seeds` in `crates/rating-core/src/lib.rs` (line 974) checks the first target season (`history_start < seasons.start()`) but reports "Invalid Elo replay end season". Calling `preseasons(history, cfg, cfg.history_start..=2006)` fails with that message, although the end season is valid and the problem is the start season.

## 7. The new import tests are slow and brittle (resolved)

The new tests in `apps/import-history/tests/cli.rs` (line 131) make downloads fail with an unparsable URL and assert reqwest's "builder error" text. `fetch_source` retries that error, which can never succeed, with 1 s and 2 s sleeps, adding about 3 s per failing call. The two tests make three such calls, adding roughly 6–9 s to `cargo test`, and they break on any reqwest upgrade that rewords the error's `Display` text, though the importer's behavior is unchanged.

## 8. Four copies of the "history is current" check (resolved)

`preseasons` in `walk_forward.rs` (line 26) checks the history's schema, league, `teams_sha256`, and starting season against the configuration. The same predicate is also written out in `build_seed`, `tuning::validate`, and `import-history`'s `reusable()`, each with a slightly different subset: `reusable()` also checks the source URL, and `build_seed` checks the final season.

The next field added to `GameFile` has to be added to all four, and missing one makes that tool silently accept a stale history. A single `GameFile::is_current_for(cfg)` would remove the duplication.

## 9. `replay_seeds` returns its results through a callback (resolved)

`replay_seeds` (line 960) takes an `FnMut` callback, but both callers only collect what it produces. `build_seed` stashes the result in an `Option` and calls `.expect("one target season")`, and `preseasons` pushes into a `Vec` from inside the closure. Returning `Vec<(EloSeed, TieHistory)>` would remove the panic path and the captured mutable state: `build_seed` could take `.pop()`, and `preseasons` could map over the result.

## 10. The seed schema version is an inline literal (resolved)

The seed's `schema_version` is the literal `2` (line 992), while the other generated files use constants (`HISTORY_SCHEMA_VERSION`, `SIMULATED_SEASONS_SCHEMA_VERSION`). The browser matches it with `z.literal(2)`, and the tests (`fit_scale.rs` and the parity fixture) hard-code `2`. The next seed format change has to find the literal inside `replay_seeds`, and a missed bump leaves Rust and the web out of step, which the browser reports only at runtime.

## 11. `preseasons` filters and sorts the history for each season (resolved)

For each target season, `preseasons` (`walk_forward.rs`, line 39) filters the whole history and sorts that season's matches again, although `replay_games` has already sorted the games by season, start, and id. For MLB, about 28 seasons and 70,000 games, `bayes-tune` and `simulate-season` scan every game once per prepared season and clone and sort each season. Partitioning the history by season in one pass would avoid the O(seasons × games) work.

## 12. Every replay builds an `Audit` per game and discards it (resolved)

`replay_seeds` (line 983) calls `elo.apply` for every historical game. Each call builds a full `Audit`, including a cloned `game_id` string, but `TieHistory::add` reads only `home_before` and `away_before`. Every replay that builds a seed, in `generate-preseason-seed`, `bayes-tune`, and `simulate-season`, therefore makes one heap allocation per game, about 70,000 for MLB, that is dropped immediately. `Elo::apply` could return only the pregame ratings and leave the `Audit` to `replay_elo`.

## 13. `model.md` describes `evaluate-model`'s old season split (resolved)

`docs/model.md` (line 387) still says `evaluate-model` scores `tune_end + 1` through `test_end` of the `bayes_tune` split. It now scores whatever split `simulate-season` saved, which `simulate-season`'s `--tune-end` and `--test-end` options can override. After `simulate-season --tune-end 2022`, `evaluate-model` reports held-out seasons from 2023 on, and a reader following `model.md` would expect the configured `bayes_tune` split.

## 14. `teams_sha256` is documented as a hash of compact JSON (resolved)

`docs/extending.md` (line 231) describes `teams_sha256` as the SHA-256 of `teams` serialized as compact JSON. The hash actually comes from serde's re-serialization of the parsed teams: fields in struct order, `through_season: null` kept, and `abbreviation` and `division` omitted when absent. It isn't a hash of the configuration's bytes. A tool written from the documentation, for example one hashing `JSON.stringify(config.teams)` on the zod-parsed configuration or the raw `teams` text, gets a different digest and treats every history as stale.

## 15. A doc comment restates its function's name (resolved)

`SimulatedSeasons::save` in `crates/rating-core/src/lib.rs` (line 369) gained the doc comment ``/// Writes the file to `path`.``, which only restates the function's name and signature. The project's convention is to comment only on why, and only when it isn't obvious.
