# Statistical model

Only game outcomes, opponent identities, season boundaries, and venue are model inputs. Scores are used by the source adapter to derive outcomes and are then discarded. The NFL and MLB share the same model and code; each league's configuration ([`config/nfl.json`](../config/nfl.json), [`config/mlb.json`](../config/mlb.json)) sets its data source, historical start, and model settings.

## Historical scope and data source

Each league's history starts with the season of its most recent expansion, so every configured franchise
exists in every modeled season. History includes **regular-season and postseason games**; preseason, spring
training, exhibition, and All-Star games are excluded.

Scores are read only to determine **win, loss, or tie**, then discarded. Neither rating model uses scores,
margin of victory, rosters, injuries, play-by-play, or betting markets. Missing scores mean an unplayed
or unreported outcome, never a tie. Historical training data is accepted only when every included season
has completed games and the league's completion marker: a completed Super Bowl (NFL) or a completed World
Series game (MLB).

### NFL

The NFL configuration starts in **2002**, the first season of the Houston Texans, the league's most recent
expansion team. It includes **regular-season and postseason games, including the Super Bowl**. Preseason and
exhibition games are excluded.

The source (`nflverse-csv`) is the maintained [nflverse/nfldata games dataset](https://github.com/nflverse/nfldata),
exposed as one downloadable CSV covering every season. [nflreadr documents this source](https://nflreadr.nflverse.com/reference/load_schedules.html).
The [Pro Football Hall of Fame's Texans history](https://www.profootballhof.com/teams/houston-texans/team-history)
documents their 2002 debut.

Game dates and times are local to `America/New_York`. The source marks neutral-site games, and home
advantage is removed for them. The source has no authoritative live/final flag, so the app uses a reported
result only from the next calendar day in Eastern Time.

### MLB

The MLB configuration starts in **1998**, when the Arizona Diamondbacks and Tampa Bay Devil Rays debuted in
the league's most recent [expansion](https://en.wikipedia.org/wiki/1998_Major_League_Baseball_expansion);
all 30 franchises exist throughout. It includes **regular-season and postseason games, including the World
Series**.

The source (`mlb-statsapi`) is the MLB Stats API schedule endpoint, requested once per season. The importer
requests one schedule per history season; the browser requests only the current season. The adapter applies
these rules:

- Game types: regular season (`R`) and the postseason rounds Wild Card (`F`), Division Series (`D`), League
  Championship Series (`L`), and World Series (`W`) are included. Spring training (`S`), exhibition (`E`),
  and All-Star (`A`) games are excluded. Any other game type is an error.
- Finality: the status text `status.detailedState`, up to any `:`, decides. `Final` and `Completed Early`
  are final; `Postponed`, `Cancelled`, and `Suspended` listings are dropped; every other state is
  unfinished. `abstractGameState` is not used because it reports `Final` even for postponed listings.
- One game per `gamePk`: a game that was postponed and played later, or suspended and resumed, appears in
  several listings. They collapse into one game represented by its latest final listing.
- Timing: the start time is the listing's `gameDate`, a UTC instant. The browser dates each game by its
  `officialDate`.
- Ties: a final listing with `isTie` and equal scores is a tie. Regular-season ties are rare; postseason
  ties are rejected.
- Venue: every game is a home game (`neutral` is always false). International series, the Field of Dreams
  and Little League Classic games, and the 2020 Toronto Blue Jays' home games in Buffalo stay home games;
  MLB neutral-site modeling is out of scope.
- Missing scores: a final listing without scores is dropped with a warning.
- 2020: the 60-game season and its expanded postseason are included as-is.
- Browser only: an unfinished postseason listing that names a provider placeholder team for an undecided
  matchup (a team id absent from the configuration's `aliases`) is dropped. Any other unknown team is an
  error.

Every Stats API response carries this notice: `Copyright <year> MLB Advanced Media, L.P.  Use of any content on this page acknowledges agreement to the terms posted here http://gdx.mlb.com/components/copyright.txt`.
Those terms permit only individual, non-commercial, non-bulk use of the materials; any other use requires
MLB Advanced Media's prior written authorization. Rally Row's importer, run by the maintainer, fetches one
schedule per history season, and each visitor's browser fetches the current season directly from the Stats
API.

## Franchise continuity

Ratings follow a permanent franchise identity across relocations and renames. Season-based identity eras
preserve each game's historical name and home market.

- NFL: Oakland and Las Vegas share the Raiders' rating history, as do San Diego and Los Angeles for the
  Chargers, and St. Louis and Los Angeles for the Rams. Historical Washington and Jacksonville abbreviations
  also map to their corresponding franchises. Source ids are the provider's era-specific abbreviations.
- MLB: the Montreal Expos and Washington Nationals share one rating history, as do the Anaheim Angels, Los
  Angeles Angels of Anaheim, and Los Angeles Angels; the Tampa Bay Devil Rays and Rays; the Florida and
  Miami Marlins; the Cleveland Indians and Guardians; and the Oakland Athletics and the Athletics in
  Sacramento. Source ids are the Stats API's numeric team ids, which stay the same across renames and
  relocations.

A relocation, rename, change of ownership, or roster change does not reset a rating or create an extra
team. Venue changes are separate: a game's neutral-site designation determines whether home advantage
applies. See [Franchise identity and historical consistency](team-history.md) for additional identity rules and historical sources.

## Historical Elo

Every team starts at the configured initial rating `R_0` (1000 in both leagues). The same update factor K applies to regular-season
and postseason games. For home team h and away team a, with ratings R, scale s, and home advantage H:

```math
E_h = \frac{1}{1 + 10^{(R_a-R_h-H)/s}}
```

H is zero at a neutral venue. The observed score S is 1 for a home win, 0 for an away win, and 1/2 for a tie. Ratings update simultaneously:

```math
R'_h = R_h + K(S-E_h), \qquad R'_a = R_a - K(S-E_h).
```

`E_h` is an expected fractional game score, not a separately modeled probability of a home win when ties are possible. The historical Elo system uses ties correctly without estimating three outcome probabilities.

Games are sorted by season, UTC start time, and game ID, using the stored `start_time_utc` as authoritative.
Before each new season, and once before the target season, regression r is applied:

```math
R_{new} = R_0 + (1-r)(R_{old}-R_0).
```

This keeps very old results from dominating indefinitely. All historical wins and losses remain in the chronological training sequence. The audit records each game's before/after ratings, predicted fractional score, and observed score. Preseason ratings cannot include target-season results; file season boundaries and hashes are checked.

## Bayesian prior

Let `c = ln(10) / s`. Each team's latent strength has independent prior

```math
\theta_i \sim N(c(R_i-R_0), (c\sigma_R)^2).
```

The means come from the Rust preseason Elo file. `sigma_R` is the configured `prior_sd_elo` (NFL 100, MLB 35.3553 Elo points), the initial uncertainty, not the team's historical standard error. Historical Elo alone does not identify a Bayesian prior variance.

## Win, loss, and tie likelihood

For `d = theta_h - theta_a + c H` and positive tie parameter nu, define

```math
D = e^{d/2}+e^{-d/2}+\nu,
\quad P(h\text{ wins}) = e^{d/2}/D,
\quad P(a\text{ wins}) = e^{-d/2}/D,
\quad P(\text{tie}) = \nu/D.
```

This is Davidson's extension of the Bradley–Terry model. Setting nu to zero yields binary Bradley–Terry, equivalent to the Elo logistic relationship at fixed strengths. In both leagues, nu is zero for postseason games because the configuration allows ties only in the regular season (`ties_allowed_in` is `["regular"]`). All three probabilities sum to one. See R. R. Davidson, *On Extending the Bradley–Terry Model to Accommodate Ties in Paired Comparison Experiments*, Journal of the American Statistical Association, 1970, [DOI: 10.1080/01621459.1970.10481082](https://doi.org/10.1080/01621459.1970.10481082).

The Rust calculator estimates nu from historical games in phases that allow ties and their pre-game Elo
differences. It adds `tie_prior_games` equal-strength pseudo-games with observed tie frequency
`tie_prior_rate`, then solves for expected ties across the real and pseudo-games to equal the observed
ties plus `tie_prior_games * tie_prior_rate`. Nu is fixed during the target season; uncertainty in nu
is not included in the posterior.

## Posterior and numerical method

The app forms the joint posterior from the preseason priors and the likelihood of each eligible completed game **once**. It maximizes log posterior by damped Newton iterations, then takes the inverse Hessian of the negative log posterior as covariance. The proper Gaussian prior makes the precision matrix positive definite and anchors absolute rating levels. Cholesky factorization is used for solves and the inverse. The log-posterior objective is accumulated with compensated (Neumaier) summation, so fits over MLB-sized seasons of about 2,500 games converge.

The result is a **Laplace approximation**, not exact sampling. It retains the full team covariance matrix. Given the approximation, the matchup difference is normal with

```math
\mu_d = \mu_h - \mu_a + cH,
\quad \sigma_d^2 = \Sigma_{hh} + \Sigma_{aa} - 2\Sigma_{ha}.
```

Predicted outcome probabilities average the likelihood over this normal distribution using deterministic Simpson quadrature over ±8 standard deviations (160 intervals). They are not just probabilities evaluated at the fitted mean. The displayed home-win credible interval transforms the 2.5th and 97.5th percentiles of d through the monotonically increasing home-win function. It describes uncertainty about the home team's win probability, not a range of possible game scores.

The matchup display also shows fair (no-vig) two-way American moneylines. A two-way line treats a tie as a push, so it depends only on the odds ratio r = P(home win) / P(away win), which conditioning on a decisive game leaves unchanged. The favorite's line is −round(100 · r) and the underdog's is +round(100 · r), with r taken as the larger side over the smaller. Both lines come from the same rounded magnitude, so they are exact mirror images. A magnitude of 100 is even money and is shown as +100 on both sides. If either win probability is zero, or the ratio overflows, no line is shown. Postseason games have no tie probability, so their moneylines are the plain two-outcome lines.

The Rust `rating-core::bayesian` implementation uses the same likelihood, optimizer, compensated summation, covariance, and predictive integration as the browser. A shared fixture (`crates/rating-core/tests/fixtures/bayesian-parity.json`) checks both implementations. `bayes-tune` uses this model for offline chronological parameter search, keeping Elo fixed, regenerating preseason priors and tie weights from earlier seasons, and excluding same-UTC-day outcomes from predictions. It selects by mean season log loss, then evaluates a frozen choice on later held-out seasons. See [Bayesian parameter tuning](#bayesian-parameter-tuning) for search settings and report contents.

Each rebuild uses the original preseason priors and the current set of eligible outcomes, so reloads do
not double-count games. Corrected outcomes replace their earlier versions. A source update that drops a
previously completed game is rejected, preserving the previous snapshot. At each new season, strengths
reset to newly generated preseason priors; Bayesian posteriors do not carry across seasons.

## Current settings

The [NFL configuration](../config/nfl.json) and the [MLB configuration](../config/mlb.json) specify:

| Parameter | NFL | MLB |
| --- | --- | --- |
| Initial Elo | `1000` | `1000` |
| Elo scale | `400` | `400` |
| Elo update factor K | `40` | `4` |
| Home advantage (Elo points; zero at neutral venues) | `45` | `29` |
| Offseason regression toward the initial rating | `0.383333` (38.3333% per season) | `0.333333` (33.3333% per season) |
| Bayesian prior standard deviation (Elo points) | `100` | `35.3553` |
| Tie smoothing pseudo-games | `212.132` | `707107` |
| Tie smoothing rate | `0.00707107` (0.707107%) | `0.0000424264` (0.00424264%) |
| Ties allowed in | regular season | regular season |

The smoothing rate is not the predicted tie probability for every matchup. The estimated tie parameter,
the teams' relative strengths, and the game phase determine that probability.

## Pregame reconstruction and backtesting

Completed games show the actual result alongside the expected winner and win probability reconstructed
from the preseason seed and results from earlier days only. Each source defines a game's pregame day, and
only results from strictly earlier days count:

| Source | Pregame day |
| --- | --- |
| NFL (`nflverse-csv`) | Eastern Time date of the game's start; the cutoff is midnight Eastern Time |
| MLB (`mlb-statsapi`) | The game's `officialDate`, so the games of a doubleheader never inform each other |
| Canonical JSON (`canonical-json`) | UTC date of the game's start; the cutoff is midnight UTC |

Same-day and later outcomes are excluded because the data does not record game-end timestamps.

The offline Bayesian backtest and Bayesian tuning use earlier **UTC dates** only, for every league.
Same-day outcomes cannot influence one another, even with mixed source timezones. For MLB, a late game whose
UTC start falls on the day after its official date is predicted in the next UTC date's batch, so earlier
games of the same official date can inform it there, unlike in the app. For each evaluated season,
preseason ratings and tie estimates use only earlier seasons.

The backtest compares predictions with an equal-strength baseline using multiclass Brier score and
natural-log loss. The baseline splits non-tie probability equally between the teams, with no home advantage,
and uses the preseason tie parameter at equal strength in phases that allow ties. Brier score sums squared
errors across the three outcomes; log loss penalizes low probability assigned to the observed outcome,
with a probability floor of `1e-15` for numerical stability. Lower values are better for both measures. Parameter
selection and evaluation are separate: later held-out seasons are scored after the parameter choice is frozen.

## Elo-only tuning

The Elo search varies the update factor, home advantage, and offseason regression using the same
chronological replay as the preseason ratings. It evaluates expected fractional scores without fitting
a Bayesian model or estimating separate tie probabilities.

Default splits come from each configuration's `elo_tune`:

| Seasons | NFL | MLB |
| --- | --- | --- |
| Warm-up | 2002–2009 | 1998–2005 |
| Selection | 2010–2022 | 2006–2022 |
| Held-out evaluation | 2023–2025 | 2023–2025 |

History must contain completed games for every season and the league's completion marker: a completed
Super Bowl for each NFL season, a completed World Series game for each MLB season.
The initial rating and Elo scale stay fixed at their configured values. The objective is equally weighted
**season mean squared error** between each pre-game expected score and its outcome (win = 1, tie = 0.5,
loss = 0). This evaluates Elo's fractional expected score, not three separate outcome probabilities.

The search starts from the configuration's `tuning_grids.elo` when present; neither shipped configuration
sets one, so both leagues start from the default grid of 125 combinations: K = `{10, 15, 20, 30, 40}`, home advantage = `{0, 25, 40, 55, 70}`,
and regression = `{0, 0.15, 1/3, 0.5, 0.75}`. Current settings are also included. When the minimum lies at an
expandable boundary, the grid expands up to four times: halve/double the lower/upper K boundary, add 25 to
the upper home advantage, or extend regression to 1. Home advantage stays nonnegative and regression stays
within `[0, 1]`. The three best grid points are refined with offsets `{-2, -1, 0, 1, 2}` times steps of 2.5 K,
5 home Elo, and 0.05 regression, discarding invalid values.

To avoid choosing an isolated minimum, selection prefers the candidate closest to current settings among
those within one paired-season standard error of the minimum MSE. Each parameter difference is divided by
its scale (current K, 25 home Elo, or 0.25 regression), then squared; distance is the sum of those squares. Lower MSE
breaks distance ties. The report includes both this conservative selection and the absolute minimum.
This is a stability heuristic, not a statistical significance test.

Only after selection is frozen are the chosen and current settings evaluated on the held-out seasons.
Earlier held-out outcomes update ratings for later games, but never change the selected parameters.
The report includes per-season and pooled MSE, first/second halves of each season by game count, expected-score
calibration bins, paired differences and descriptive standard errors, the ten lowest-MSE candidates,
input SHA-256 hashes, and an RFC 3339 UTC run-start timestamp (`run_at`). Positive MSE improvement means the
selected settings performed better. The report does not claim an improvement is established when it is small
or inconsistent; limited held-out seasons constrain inference. Repeatedly changing the search after reading holdout scores would invalidate that evaluation.

Both tuners' reports describe the search: `search.grid_source` (`default` or `config`), `search.coarse_grid`
(the starting grid), `search.evaluated_ranges` (the minimum and maximum evaluated value of each parameter),
and `search.selected_on_boundary` (the parameters whose selected value lies on an evaluated range boundary).
When that list is not empty, the notes say
`Selected parameters lie on a searched boundary (<names>); extend tuning_grids in the league configuration and rerun before adopting.`

## Bayesian parameter tuning

The Bayesian search varies prior uncertainty (`prior_sd_elo`) and tie smoothing (`tie_prior_games` and
`tie_prior_rate`), keeping all Elo settings fixed. Its default split comes from `bayes_tune`, which falls
back to `elo_tune`; neither shipped configuration sets `bayes_tune`, so both leagues use the Elo tuning
splits above. At least one warm-up season, two tuning seasons, and one later completed held-out season are
required, and each season must include the league's completion marker.

For each evaluated season, preseason Elo and the tie estimate use only earlier seasons.
Each UTC date is predicted from a fresh fit using only earlier dates in that season. Same-day outcomes
cannot influence one another. Team strength resets to the newly generated preseason prior at each season
boundary; Bayesian posteriors do not carry across seasons.

The search starts from the configuration's `tuning_grids.bayesian` when present, else from the default grid
of 120 combinations:

| Parameter | Search values |
| --- | --- |
| `prior_sd_elo` | 50, 75, 100, 150, 200, 300 |
| `tie_prior_games` | 10, 30, 100, 300, 1000 |
| `tie_prior_rate` | 0.001, 0.0025, 0.005, 0.01 |

NFL uses the default grid. MLB configures a grid of 27 combinations:

| Parameter | MLB configured values |
| --- | --- |
| `prior_sd_elo` | 25, 35.35533905932737, 50 |
| `tie_prior_games` | 10000, 1000000, 100000000 |
| `tie_prior_rate` | 0.000001, 0.00003, 0.001 |

MLB ties are rare and not stationary: the 1998–2005 warm-up seasons contain 8 ties and the 2006–2022 tuning
seasons contain 1. Searches from the default grid repeatedly selected values past its boundaries, so the MLB
configuration supplies its own starting grid.

Current settings are also included. The three best initial candidates receive one local refinement using
each parameter multiplied by `1/sqrt(2)`, `1`, or `sqrt(2)`; rates must stay below 1. Duplicate candidates
are skipped. This is a bounded search, not a guarantee of a global optimum.

Selection minimizes equally weighted **season mean log loss**, using natural logs and all three outcomes.
Among candidates within one paired-season standard error of the minimum, it prefers the smallest sum of
squared log parameter ratios relative to current settings, breaking ties by log loss. The report identifies
both this conservative selection and the absolute minimum. Rare ties may leave smoothing parameters weakly
identified; this preference is a stability heuristic, not a significance test.

Only after selection is frozen are current and selected settings evaluated on the held-out seasons.
Earlier held-out outcomes can inform later predictions but never change selected parameters. Holdout validity
also requires that these seasons did not influence Elo settings or earlier search choices.

Reports include input hashes, fixed Elo settings, the search grid and refinement, top candidates, per-season and pooled log
loss/Brier scores, first/second season halves, per-outcome calibration, expected versus observed ties, and
paired season differences. They also include the search fields described under [Elo-only tuning](#elo-only-tuning).
Brier score sums squared errors across all three outcomes. Positive log-loss
improvement means the selected parameters performed better. Neither tuning process automatically changes model settings.

## Limits and validation

- Team strength is modeled as constant within each fitted season. There is no explicit random walk over time, injury adjustment, roster change, rest effect, or recency weighting within that season.
- The covariance and intervals are approximate and conditional on the selected hyperparameters, fixed home advantage, and fixed tie parameter. They do not include all sources of forecast uncertainty.
- Configured parameters and tuning tools do not establish forecasting superiority. Small or inconsistent held-out improvements and a limited number of seasons constrain conclusions. Repeatedly adjusting a search after reading holdout scores invalidates that evaluation.
- Historical continuity across a franchise relocation is a deliberate team-identity rule, not a claim of unchanged roster quality.
- Result timing depends on the source. The NFL source does not expose an authoritative live/final flag, so the app waits until the next calendar day in Eastern Time before using a reported outcome. MLB results count as soon as the Stats API marks a game final. Later corrections are accepted at the next successful update.
- MLB games are never neutral-site games in the model, including games at special or international venues.
- Backtests refit using earlier dates only. Evaluate held-out seasons and calibration before drawing conclusions about model quality.

Unit tests cover symmetry, probability normalization, neutral-site handling, proper tie outcomes, posterior updates, an analytic two-team Hessian/covariance case, duplicate rejection, and repeatable refits. Integration tests cover provider normalization, cache fallback, corrected results, stale priors, and season boundaries. Shared fixtures in `crates/rating-core/tests/fixtures/` check that the Rust and TypeScript implementations agree: `bayesian-parity.json` for the Bayesian fit and `mlb-statsapi.json` for the MLB adapter.
