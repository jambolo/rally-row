# Rally Row

<img src="docs/branding/production/svg/rally-row-tagline-on-navy.svg" alt="Rally Row. A little insight. A lot to talk about." width="640">

[![CI (Rust)](https://github.com/jambolo/rally-row/actions/workflows/ci-rust.yml/badge.svg?branch=develop)](https://github.com/jambolo/rally-row/actions/workflows/ci-rust.yml?query=branch%3Adevelop)
[![CI (Web)](https://github.com/jambolo/rally-row/actions/workflows/ci-web.yml/badge.svg?branch=develop)](https://github.com/jambolo/rally-row/actions/workflows/ci-web.yml?query=branch%3Adevelop)
[![Coverage (develop)](https://codecov.io/gh/jambolo/rally-row/branch/develop/graph/badge.svg)](https://codecov.io/gh/jambolo/rally-row/tree/develop "Coverage on develop")

## Overview

Rally Row helps sports fans see how teams stack up and who's more likely to win. It combines historical
team ratings with the current season's results to estimate each team's chance of winning a matchup.
It covers the NFL and MLB, including regular-season and postseason games, and you can switch leagues
at any time.

Use it to explore any two teams in a league, compare home and neutral venues, follow the schedule, and see
how team strength estimates change from preseason. Predictions include separate win, loss, and tie
probabilities and an uncertainty range. The app runs in your browser with no account, API key, or
subscription required.

## Usage

Open [Rally Row](https://jambolo.github.io/rally-row/) in your browser. The first visit to a league downloads
its published ratings and current-season results and prepares the predictions. Returning visits show saved
results immediately while the app checks for updates.

### Choose a league

Pick a league with the **League** selector in the header; the season shown next to it follows the selected
league. You can also open a league directly with the address hash `#nfl` or `#mlb` (exact and
case-sensitive; any other hash is ignored).

Switching leagues, with the selector or by changing the hash, sets the address hash to that league (a browser
history entry), remembers your choice in this browser, loads that league, resets the schedule filter to all
games, and picks a new default matchup. Each league keeps its own saved results.

When the page loads, Rally Row shows the first league that applies:

1. The league named by a valid address hash.
2. The league you last chose in this browser.
3. The in-season default league.

Loading the page does not change the address or the remembered choice.

The in-season default uses your browser's local date (February 29 counts as February 28) and each league's
windows, which include both end dates:

| Window | NFL | MLB |
| --- | --- | --- |
| Season | September 1 – February 15 | March 20 – November 5 |
| Postseason | January 8 – February 15 | September 30 – November 5 |

- If exactly one league is in its season window, it is the default.
- If both leagues are in season, the first one (NFL, then MLB) in its postseason window is the default; if
  neither is in its postseason window, the one whose postseason starts sooner is the default.
- If no league is in season, the one whose season starts sooner is the default.
- Remaining ties go to the NFL, then MLB.

For example, June 15 defaults to MLB; December 1 and January 10 to the NFL; September 15 to MLB (both
leagues are in season and MLB's postseason starts sooner); October 15 to MLB (in its postseason window);
and February 20 to MLB (neither league is in season and MLB's season starts sooner).

### Explore a matchup

1. Select **Explore matchups**, then choose two different teams under **Who takes it?**
2. Choose the **Home team** and **Away team**. Use **Swap teams** to reverse them.
3. Check **Neutral venue** for a game with no home advantage; the selectors become **Team A** and **Team B**.
4. Set **Game type** to **Regular season** or **Postseason**. Postseason games have zero tie probability in
   both leagues, and the tie column shows the league's note instead.
5. Read each team's chance of winning and the chance of a tie. Expand **How certain is this estimate?**
   for the approximate 95% credible interval for the home team's (or Team A's) win probability.
6. Read each team's **Two-way ML**, the fair American moneyline implied by the model, with no bookmaker
   margin (vig). A two-way line treats a tie as a push (the stake is refunded), so it prices each team's
   chance of winning a decisive game. For example, 75.9% / 23.8% with a 0.3% tie becomes 76.1% / 23.9%,
   shown as −319 / +319. The two lines are always mirror images, and even money is shown as +100.
7. Select **Compare book odds** to enter a sportsbook's moneyline for either team as quoted, vig included,
   such as `-110` or `+150`. The dialog shows each bet's expected value (EV) in dollars per $100 staked at the
   book's payout, assuming the fair two-way moneylines are exact and refunding the stake on a tie. The book's
   vig is not removed, so it lowers the EV of both sides. A positive EV is shown in blue. Lines between −100
   and +100 are rejected. Changing the teams, venue, or game type clears the entered lines.

The model predicts outcomes, not scores. The uncertainty interval shows how precisely it estimates a
team's win probability. Displayed percentages are rounded, so their sum can differ slightly from 100%.
The moneylines are the model's fair prices, not betting advice or market odds, and the expected values are
only as accurate as those prices.

### Follow the season

In **Schedule & results**, filter by **Week** (NFL) or **Date** (MLB) and switch between **Upcoming**,
**Completed**, and **Awaiting result**. **Awaiting result** lists games that have started but whose result
cannot be used yet. Select an upcoming game's prediction to load its teams, venue, and game type into the
matchup controls. Completed games show the actual result alongside the reconstructed pregame favorite and
its win probability, using only results from earlier days.

The **Team ratings** table compares preseason and current strength estimates. Higher ratings indicate a
stronger estimated team; **± uncertainty** is one standard deviation of the current strength estimate,
not the matchup's 95% probability interval. Expand **Franchise names and locations over time** to see
how renamed or relocated teams retain their rating history.

### League differences

Both leagues use the same model and the same screens. These details differ:

| Feature | NFL | MLB |
| --- | --- | --- |
| Schedule filter | **Week**, with **All weeks** | **Date**, with **All dates** |
| Game start label | **Kickoff** | **First pitch** |
| Postseason games | Labeled **Playoffs**; every game also shows its week | Labeled by round: **Wild Card**, **Division Series**, **League Championship Series**, **World Series** |
| Postseason tie note | **No ties in NFL playoffs** | **No ties in MLB postseason** |
| When a result counts | On the next calendar day in Eastern Time | As soon as the provider marks the game final |
| Earlier days for pregame favorites | Earlier dates in Eastern Time | Earlier official game dates, so doubleheader games never inform each other |
| Current season | Runs into the next year; January–March games belong to the season that began the previous year | The calendar year |

### Updates and saved results

The status card shows **Data retrieved** and **Last checked**. These are your browser's timestamps, not
the data provider's publication time. Compatible saved results remain usable while an update is being prepared.
If an update fails, the app keeps the previous results and displays a warning.

The footer shows the running app version. Loading a different version rebuilds saved predictions before
showing them. If that rebuild fails, the app reports an error and keeps the saved data for retry.

The status card also summarizes the league's result policy; **How these predictions work** explains it and
links to the league's data source:

- NFL results enter the model on the next calendar day in Eastern Time. The source doesn't say whether a
  score is final, so the app ignores today's scores.
- MLB results enter the model as soon as the provider marks a game final. Postponed, suspended, and
  canceled games are not results.

A game may remain under **Awaiting result** until its result is eligible and the provider has published it.
Provider delays can extend this wait.

The app checks a league for updates when it loads, with a one-minute cooldown between attempts for each
league. Reload to check again; an open page does not poll periodically. Corrections to eligible outcomes and
updates to published preseason ratings or historical data take effect on the next successful update. NFL
results that become eligible after midnight Eastern also trigger a rebuild on the next successful check,
even if the source data is unchanged. A newly final MLB game changes the source data, so the next
successful check rebuilds.

Saved data stays in the current browser, and each league keeps its own. If browser storage is unavailable
or full, the app still runs but cannot preserve results across reloads.

The season is selected automatically for each league on page load (see [League differences](#league-differences)).
After a season rollover, a successful update requires the new season's published preseason ratings and
schedule. If those are unavailable, saved results may remain visible with a warning.

A game's start alone does not trigger a rebuild, so saved game statuses may remain stale until another change
requires one. Clearing the site's saved data and reloading forces a rebuild from the current published inputs.

For local setup, building, and contributing, see [DEVELOPMENT.md](DEVELOPMENT.md).

## Methodology

Rally Row builds preseason Elo ratings from each league's regular-season and postseason results:
NFL since 2002 and MLB since 1998. NFL results come from the nflverse games dataset and MLB results from
the MLB Stats API. Offseason regression moves ratings toward the league average while preserving each
franchise's history.

A Bayesian model starts from those ratings and updates team strength estimates using eligible
current-season wins, losses, and ties. It accounts for opponent strength, home advantage, neutral venues,
tie rules, and uncertainty to produce matchup probabilities. Both leagues use the same model with their own
configured settings.

The model does not use score margins, injuries, rosters, or betting markets. Evaluation tools use
chronological backtests and separate tuning and held-out seasons.

See [Statistical model](docs/model.md) for data sources and their terms, formulas, model settings, parameter
selection, and limitations.
