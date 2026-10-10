# Backtesting

The Node backtest, `apps/web/scripts/backtest.ts`, scores the browser's Bayesian model on one season as the app
would have predicted it. It runs the app's own TypeScript source adapter and model, so it checks the browser
implementation rather than the Rust one. It compares the model's predictions with an equal-strength baseline and
prints a JSON summary.

Unlike [`evaluate-model`](model-evaluation.md), which scores held-out seasons offline from the saved history, the
backtest downloads the evaluated season from the provider, so it needs network access and can score the current
season's completed games.

## Running the tool

Install the web dependencies (`pnpm -C apps/web install --frozen-lockfile`). The backtest needs the league's history
and an Elo seed for the evaluated season, [built](preseason-seed.md) from history through the season before it.

```powershell
# Evaluate the current season's completed games with the regular data
pnpm -C apps/web backtest -- --league nfl

# Evaluate 2025 with priors trained through 2024
cargo run --release -p import-history -- --league nfl --through-season 2024 --data-dir data-backtest
cargo run --release -p generate-preseason-seed -- --league nfl --target-season 2025 --data-dir data-backtest
pnpm -C apps/web backtest -- --league nfl --season 2025 --data-dir data-backtest
```

| Option | Meaning |
| --- | --- |
| `--league <id>` | Required. Reads `<config-dir>/<id>.json`, whose `id` must equal `<id>`. Ids use letters, digits, and hyphens. |
| `--config-dir <dir>` | Configuration directory; default `config`. |
| `--data-dir <dir>` | Data directory; default `data`. Reads `<dir>/<id>/history.json` and `<dir>/<id>/elo-<season>.json`. |
| `--season <year>` | Evaluated season; default the current season. |

Relative directories are resolved from the repository root, not the current directory. Without `--league`, the
backtest prints its usage and exits with status 2.

## What the backtest does

1. Reads the configuration and the seed `elo-<season>.json`, and checks the seed as the browser does: its league and
   seasons must match (`Elo seed is for the wrong league or season; run the two Rust programs`), and its hashes must
   match the configuration and `history.json` bytes
   (`Configuration changed since Elo was calculated; rerun generate-preseason-seed` or
   `History changed since Elo was calculated; rerun generate-preseason-seed`). The history is only hashed; its games
   are not used.
2. Downloads the season through the league's TypeScript [source adapter](extending.md#source-adapters) and keeps the
   results the adapter deems eligible at run time, as the app would. For the NFL, today's scores in Eastern Time
   don't count yet. With no eligible results, it fails with `No completed games to evaluate`.
3. Groups the games by UTC start date. Each date is predicted from a fresh posterior fit to the earlier dates' results
   only, so no prediction sees a same-day or later outcome.
4. Scores each prediction and the equal-strength baseline with natural-log loss and multiclass Brier score.

The baseline gives each team the same strength with no home advantage. In phases that allow ties, it gives a tie
`ν / (2 + ν)`, with the seed's tie weight ν, and each team `1 / (2 + ν)`; in other phases, each team gets 1/2.

The backtest batches by UTC date, while the app's pregame reconstruction uses each source's pregame day. For MLB, a
late game whose UTC start falls on the day after its official date is therefore predicted with earlier games of the
same official date ([Pregame reconstruction and backtesting](model.md#pregame-reconstruction-and-backtesting)).

## Output

stdout gets one JSON object:

| Field | Contents |
| --- | --- |
| `season` | Evaluated season |
| `games` | Games scored |
| `method` | One-line description of the method |
| `bayesian` | The model's mean `log_loss` and `brier` |
| `equal_strength_baseline` | The baseline's mean `log_loss` and `brier` |
| `note` | Caveat about interpreting the scores |

Lower scores are better; [Reading the scores](model.md#reading-the-scores) explains them. The backtest uses the
configured settings, which may have been tuned on the evaluated season. Scores on a season inside a configured tuning
range overstate the model's skill, so evaluate a held-out season for a fair measure.
