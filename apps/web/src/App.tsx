import { useEffect, useEffectEvent, useMemo, useState } from 'react';
import { message } from './service.ts';
import { formatTwoWayMoneyline, predict, twoWayMoneylines, type Prediction } from './model.ts';
import { startSession, type SessionView } from './session.ts';
import { leagueIds } from './leagues.ts';
import { hashChangeTarget, loadLeagueConfigs, localMonthDay, recordSwitch, startupLeague } from './selection.ts';
import { smallStore } from './small-store.ts';
import { postseasonLabel, scheduleKey, scheduleOptions } from './schedule.ts';
import { APP_VERSION } from './version.ts';
import { BookOdds } from './BookOdds.tsx';
import { PostseasonOdds } from './PostseasonOdds.tsx';
import { identityRuns } from './contracts.ts';

const base = import.meta.env.BASE_URL;
/** Absolute URL of a published site directory such as `config` or `data`. */
const siteUrl = (path: string) => new URL(`${base}${path}`, window.location.href).href;
const initialView: SessionView = { state: null, model: null, phase: 'checking', retryAt: null, error: '' };
const percent = (p: number) => `${(100 * p).toFixed(1)}%`;
const number = (n: number) => Math.round(n).toLocaleString();

function exploreMatchups() {
  const matchup = document.getElementById('matchups');
  matchup?.focus({ preventScroll: true });
  matchup?.scrollIntoView({
    behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth',
    block: 'start',
  });
}

export default function App() {
  // Null until the startup rule decides.
  const [league, setLeague] = useState<string | null>(null);
  const [names, setNames] = useState<Record<string, string>>({});
  // Views carry their league, so a previous league's late update never shows.
  const [session, setSession] = useState<{ league: string; view: SessionView } | null>(null);
  const view = session !== null && session.league === league ? session.view : initialView;
  const { state, model, error } = view;
  const [home, setHome] = useState(''),
    [away, setAway] = useState(''),
    [neutral, setNeutral] = useState(false);
  const [phase, setPhase] = useState<'regular' | 'postseason'>('regular');
  const [tab, setTab] = useState<'scheduled' | 'completed' | 'awaiting_result'>('scheduled');
  const [scheduleFilter, setScheduleFilter] = useState('all');
  useEffect(() => {
    let active = true;
    const configs = loadLeagueConfigs(leagueIds, siteUrl('config'));
    void configs.then((loaded) => {
      if (active) setNames(Object.fromEntries(loaded.map((l) => [l.id, l.name])));
    });
    void startupLeague({
      ids: leagueIds,
      hash: window.location.hash,
      store: smallStore(),
      today: localMonthDay(),
      loadWindows: () => configs,
    }).then((id) => {
      // A switch made while the configurations were loading wins.
      if (active) setLeague((current) => current ?? id);
    });
    return () => {
      active = false;
    };
  }, []);
  useEffect(() => {
    if (league === null) return;
    let selected = false;
    return startSession({
      league,
      configBase: siteUrl('config'),
      dataBase: siteUrl('data'),
      onChange: (nextView) => {
        setSession({ league, view: nextView });
        const s = nextView.state;
        if (s?.status === 'ready' && !selected) {
          selected = true;
          const next = s.games.find((g) => g.status === 'scheduled');
          setHome(next?.home_team ?? s.teams[0]?.id ?? '');
          setAway(next?.away_team ?? s.teams[1]?.id ?? '');
          setNeutral(next?.neutral ?? false);
          setPhase(next?.phase ?? 'regular');
        }
      },
    });
  }, [league]);
  const switchLeague = (id: string) => {
    if (id === league || !leagueIds.includes(id)) return;
    recordSwitch(id, { location: window.location, store: smallStore() });
    setLeague(id);
    setSession(null);
    setScheduleFilter('all');
  };
  const onHashChange = useEffectEvent(() => {
    const id = hashChangeTarget(window.location.hash, league ?? '', leagueIds);
    if (id !== null) switchLeague(id);
  });
  useEffect(() => {
    const listener = () => onHashChange();
    window.addEventListener('hashchange', listener);
    return () => window.removeEventListener('hashchange', listener);
  }, []);
  const leagueName = (id: string) => names[id] ?? (id === league && state ? state.league : id);
  // Predicting is a pure function of the fitted model, so it is derived, never stored.
  const { prediction, predictionError } = useMemo<{
    prediction: Prediction | null;
    predictionError: string;
  }>(() => {
    if (!model || state?.status !== 'ready' || !home || !away) return { prediction: null, predictionError: '' };
    if (home === away)
      return {
        prediction: null,
        predictionError: 'Choose two different teams.',
      };
    try {
      return {
        prediction: predict(model, home, away, neutral, phase),
        predictionError: '',
      };
    } catch (e) {
      return { prediction: null, predictionError: message(e) };
    }
  }, [model, state?.status, home, away, neutral, phase]);
  const twoWayLines = prediction ? twoWayMoneylines(prediction) : { home: null, away: null };
  const teams = [...(state?.teams ?? [])].sort((a, b) => a.name.localeCompare(b.name));
  const teamName = (id: string) => teams.find((t) => t.id === id)?.name ?? id;
  const teamLabel = (id: string) => teams.find((t) => t.id === id)?.abbreviation ?? id;
  const games = state
    ? state.games.filter(
        (g) =>
          g.status === tab && (scheduleFilter === 'all' || scheduleKey(g, state.display.schedule_filter.unit) === scheduleFilter),
      )
    : [];
  const scheduleValues = state ? scheduleOptions(state.games, state.display.schedule_filter.unit) : [];
  return (
    <>
      <header className="topbar">
        <div className="topbar-inner">
          <a className="brand" href={base} aria-label="Rally Row home">
            <img src={`${base}brand/rally-row-primary.svg`} width="694" height="176" alt="Rally Row" />
          </a>
          <p className="brand-tagline">A little insight. A lot to talk about.</p>
          <span className="tag">
            <select
              className="league-select"
              aria-label="League"
              value={league ?? leagueIds[0]}
              disabled={league === null}
              onChange={(e) => switchLeague(e.target.value)}
            >
              {leagueIds.map((id) => (
                <option key={id} value={id}>
                  {leagueName(id)}
                </option>
              ))}
            </select>{' '}
            · {state?.season ?? 'Season'}
          </span>
        </div>
      </header>
      <main>
        <div className="intro">
          <div className="intro-copy">
            <p className="eyebrow">A seat for every sports fan</p>
            <h1>
              What's your <span>call?</span>
            </h1>
            <p className="lede">
              See how the teams stack up and who's more likely to win. Bring a little more insight to the game-day conversation.
            </p>
            <button className="primary-action" disabled={state?.status !== 'ready'} onClick={exploreMatchups}>
              Explore matchups <span aria-hidden="true">↗</span>
            </button>
          </div>
          <aside className="status-card" aria-label="Data update status">
            <span className={`status-dot ${state?.status === 'ready' ? 'ready' : ''}`} aria-hidden="true" />
            <strong>
              {view.phase === 'rebuilding'
                ? 'Updating the matchup picture'
                : view.phase === 'checking'
                  ? 'Checking for updates'
                  : view.phase === 'building'
                    ? 'Getting the matchups ready'
                    : state?.status === 'ready'
                      ? state.cached
                        ? 'Showing saved results'
                        : 'Up to date'
                      : state?.status === 'error'
                        ? 'Matchups unavailable'
                        : 'Refreshing season'}
            </strong>
            <small>
              {state?.refreshed_at ? `Data retrieved: ${new Date(state.refreshed_at).toLocaleString()}` : 'No saved data yet'}
            </small>
            {state?.checked_at && <small>Last checked: {new Date(state.checked_at).toLocaleString()}</small>}
            <small>
              {view.retryAt
                ? `Next check: ${new Date(view.retryAt).toLocaleTimeString()}`
                : 'Checks on page load · at most once per minute'}
            </small>
            {state && <p className="status-policy">{state.result_policy_summary}</p>}
          </aside>
        </div>
        {view.phase === 'rebuilding' && (
          <div role="status" className="notice">
            Updating the matchup picture. Your previous predictions are still here while we bring in the latest results.
          </div>
        )}
        {error && (
          <div role="alert" className="notice error">
            <strong>We couldn't load the matchups.</strong>
            <p>Please reload the page to try again.</p>
            <details>
              <summary>Technical details</summary>
              <p>{error}</p>
            </details>
          </div>
        )}
        {state?.warning && (
          <div role="status" className="notice">
            <p>
              {state.cached
                ? "We couldn't update the results. Your last saved predictions are still here."
                : 'The matchup picture is ready. There was a note about this update.'}
            </p>
            <details>
              <summary>Update details</summary>
              <p>{state.warning}</p>
            </details>
          </div>
        )}
        {state?.status === 'error' && (
          <div role="alert" className="notice error">
            <strong>We couldn't get the matchups ready.</strong>
            <p>Please try again later.</p>
            <details>
              <summary>Technical details</summary>
              <p>{state.error}</p>
            </details>
          </div>
        )}
        {(!state || state.status === 'loading') && !error && (
          <section className="panel loading" aria-live="polite">
            <span className="spinner" />
            {view.phase === 'waiting'
              ? 'Waiting for the next data check…'
              : 'Getting the season’s results together. Your matchup picture is on its way…'}
          </section>
        )}
        {state?.status === 'ready' && (
          <>
            <section className="metrics" aria-label="Data overview">
              <div>
                <small>Past games</small>
                <strong>{number(state.historical_games)}</strong>
                <span>
                  {state.history_start}–{state.season - 1} · regular + postseason
                </span>
              </div>
              <div>
                <small>This season's results</small>
                <strong>{state.training_games}</strong>
                <span>Wins, losses, and ties</span>
              </div>
              <div>
                <small>Teams to explore</small>
                <strong>{state.teams.length}</strong>
                <span>See how they stack up</span>
              </div>
            </section>
            <section className="panel matchup" id="matchups" tabIndex={-1} aria-labelledby="matchup-heading">
              <div className="section-heading">
                <div>
                  <p className="eyebrow">Explore a matchup</p>
                  <h2 id="matchup-heading">Who takes it?</h2>
                </div>
                <span className="chip">{phase === 'regular' ? 'Regular season' : 'Postseason'}</span>
              </div>
              <div className="matchup-controls">
                <label>
                  {neutral ? 'Team A' : 'Home team'}
                  <select aria-label={neutral ? 'Team A' : 'Home team'} value={home} onChange={(e) => setHome(e.target.value)}>
                    {teams.map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.name}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  className="swap"
                  title="Swap teams"
                  aria-label="Swap teams"
                  onClick={() => {
                    setHome(away);
                    setAway(home);
                  }}
                >
                  ⇄
                </button>
                <label>
                  {neutral ? 'Team B' : 'Away team'}
                  <select aria-label={neutral ? 'Team B' : 'Away team'} value={away} onChange={(e) => setAway(e.target.value)}>
                    {teams.map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.name}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              <div className="options">
                <label className="checkbox">
                  <input type="checkbox" checked={neutral} onChange={(e) => setNeutral(e.target.checked)} /> Neutral venue
                </label>
                <label className="phase-label">
                  Game type{' '}
                  <select aria-label="Game type" value={phase} onChange={(e) => setPhase(e.target.value as typeof phase)}>
                    <option value="regular">Regular season</option>
                    <option value="postseason">Postseason</option>
                  </select>
                </label>
              </div>
              {predictionError && (
                <p role="alert" className="inline-error">
                  {predictionError}
                </p>
              )}
              {prediction ? (
                <div className="prediction" aria-live="polite">
                  <div className="prediction-numbers">
                    <div>
                      <span className="team-code">{teamLabel(home)}</span>
                      <strong>{percent(prediction.home_win)}</strong>
                      <small>{teamName(home)} chance of winning</small>
                      <span className="moneyline" title="Fair two-way moneyline (no vig; a tie is a push)">
                        Two-way ML {formatTwoWayMoneyline(twoWayLines.home)}
                      </span>
                    </div>
                    <div className="tie-probability">
                      <span>Tie</span>
                      <strong>{percent(prediction.tie)}</strong>
                      <small>
                        {phase === 'postseason' && !state.ties_allowed_in.includes('postseason')
                          ? state.display.postseason_tie_note
                          : 'Chance of a tie'}
                      </small>
                    </div>
                    <div className="away-probability">
                      <span className="team-code">{teamLabel(away)}</span>
                      <strong>{percent(prediction.away_win)}</strong>
                      <small>{teamName(away)} chance of winning</small>
                      <span className="moneyline" title="Fair two-way moneyline (no vig; a tie is a push)">
                        Two-way ML {formatTwoWayMoneyline(twoWayLines.away)}
                      </span>
                    </div>
                  </div>
                  <div
                    className="probability-bar"
                    role="img"
                    aria-label={`${teamName(home)} win ${percent(prediction.home_win)}, tie ${percent(prediction.tie)}, ${teamName(away)} win ${percent(prediction.away_win)}`}
                  >
                    <span className="bar-home" style={{ width: percent(prediction.home_win) }} />
                    <span className="bar-tie" style={{ width: percent(prediction.tie) }} />
                    <span className="bar-away" style={{ width: percent(prediction.away_win) }} />
                  </div>
                  {twoWayLines.home !== null && twoWayLines.away !== null && (
                    <BookOdds
                      key={`${home}|${away}|${neutral}|${phase}`}
                      home={{ name: teamName(home), fair: twoWayLines.home }}
                      away={{ name: teamName(away), fair: twoWayLines.away }}
                      tie={prediction.tie}
                    />
                  )}
                  <details className="prediction-note">
                    <summary>How certain is this estimate?</summary>
                    <p>
                      These are estimates, not guarantees. The model's uncertainty range for {teamName(home)}'s chance of winning is{' '}
                      <strong>
                        {percent(prediction.home_probability_interval[0])}–{percent(prediction.home_probability_interval[1])}
                      </strong>{' '}
                      (approximate 95% credible interval).
                    </p>
                  </details>
                </div>
              ) : (
                !predictionError && <p className="muted">Calculating matchup…</p>
              )}
            </section>
            <div className="columns">
              <section className="panel schedule">
                <div className="section-heading">
                  <div>
                    <p className="eyebrow">The season</p>
                    <h2>Schedule &amp; results</h2>
                  </div>
                  <label className="week-label">
                    {state.display.schedule_filter.label}
                    <select
                      aria-label={state.display.schedule_filter.label}
                      value={scheduleFilter}
                      onChange={(e) => setScheduleFilter(e.target.value)}
                    >
                      <option value="all">{state.display.schedule_filter.all_label}</option>
                      {scheduleValues.map((value) => (
                        <option key={value} value={value}>
                          {value}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
                <div className="tabs" role="group" aria-label="Game status">
                  <button aria-pressed={tab === 'scheduled'} onClick={() => setTab('scheduled')}>
                    Upcoming
                  </button>
                  <button aria-pressed={tab === 'completed'} onClick={() => setTab('completed')}>
                    Completed
                  </button>
                  <button aria-pressed={tab === 'awaiting_result'} onClick={() => setTab('awaiting_result')}>
                    Awaiting result
                  </button>
                </div>
                {tab === 'completed' && <p className="muted">Pregame expectations use prior-day results only.</p>}
                <div className="game-list" tabIndex={0} role="region" aria-label="Games in this view">
                  {games.length === 0 ? (
                    <p className="empty">No games in this view.</p>
                  ) : (
                    games.map((g) => (
                      <article className="game" key={g.id}>
                        <div className="game-meta">
                          {state.display.start_time_label} {g.date}
                          {state.display.round_name !== null && ` · ${state.display.round_name} ${g.round}`}{' '}
                          {g.phase === 'postseason' && `· ${postseasonLabel(g, state.display)}`} {g.neutral && '· Neutral'}
                        </div>
                        <div className="game-row">
                          <div>
                            <strong title={teamName(g.away_team)}>{teamLabel(g.away_team)}</strong>
                            <span className="at">{g.neutral ? 'vs' : 'at'}</span>
                            <strong title={teamName(g.home_team)}>{teamLabel(g.home_team)}</strong>
                          </div>
                          {g.status === 'completed' ? (
                            <div className="game-outcome">
                              <span className="result-label">
                                {g.result === 'tie'
                                  ? 'Tie'
                                  : `${teamLabel(g.result === 'home_win' ? g.home_team : g.away_team)} won`}
                              </span>
                              {g.prediction && (
                                <span className="game-expectation">
                                  Expected: {teamLabel(g.prediction.home_win >= g.prediction.away_win ? g.home_team : g.away_team)}{' '}
                                  {percent(Math.max(g.prediction.home_win, g.prediction.away_win))}
                                </span>
                              )}
                            </div>
                          ) : g.prediction ? (
                            <button
                              className="game-pick"
                              aria-label={`Explore ${teamName(g.away_team)} ${g.neutral ? 'versus' : 'at'} ${teamName(g.home_team)}`}
                              onClick={() => {
                                setHome(g.home_team);
                                setAway(g.away_team);
                                setNeutral(g.neutral);
                                setPhase(g.phase);
                                exploreMatchups();
                              }}
                            >
                              {teamLabel(g.prediction.home_win >= g.prediction.away_win ? g.home_team : g.away_team)}{' '}
                              {percent(Math.max(g.prediction.home_win, g.prediction.away_win))} <span aria-hidden="true">↗</span>
                            </button>
                          ) : (
                            <span className="muted">Awaiting confirmed result</span>
                          )}
                        </div>
                      </article>
                    ))
                  )}
                </div>
              </section>
              <section className="panel rankings">
                <div className="section-heading">
                  <div>
                    <p className="eyebrow">Team ratings</p>
                    <h2>See how they stack up</h2>
                  </div>
                </div>
                <p className="table-note">
                  Higher ratings mean a stronger team estimate. ± shows uncertainty (one standard deviation).
                </p>
                <div className="table-wrap" tabIndex={0} role="region" aria-label="Team ratings">
                  <table>
                    <thead>
                      <tr>
                        <th scope="col">Team</th>
                        <th scope="col">Preseason</th>
                        <th scope="col">Current ± uncertainty</th>
                      </tr>
                    </thead>
                    <tbody>
                      {state.teams.map((t, i) => (
                        <tr key={t.id}>
                          <th scope="row">
                            <span className="rank">{i + 1}</span>
                            <abbr title={t.name}>{t.abbreviation}</abbr>
                          </th>
                          <td>{number(t.initial_elo)}</td>
                          <td>
                            <strong>{number(t.rating)}</strong>
                            <span className="sd"> ±{number(t.sd)}</span>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </section>
            </div>
            {state.postseason !== null && (
              <PostseasonOdds odds={state.postseason} label={state.display.postseason_label} teamName={teamName} />
            )}
            <details className="panel methodology">
              <summary>Franchise names and locations over time</summary>
              <div>
                <p>
                  Ratings follow a permanent franchise ID. Names and locations reflect the season in which each game was played; a
                  relocation or rename does not create a new rating.
                </p>
                <div className="table-wrap" tabIndex={0} role="region" aria-label="Franchise history">
                  <table>
                    <thead>
                      <tr>
                        <th>Franchise ID</th>
                        <th>Seasons</th>
                        <th>Name / location</th>
                      </tr>
                    </thead>
                    <tbody>
                      {state.team_history
                        .filter((t) => identityRuns(t).length > 1)
                        .flatMap((t) =>
                          identityRuns(t).map((run) => (
                            <tr key={`${t.id}-${run.from_season}`}>
                              <th>{t.id}</th>
                              <td>
                                {run.from_season}–{run.through_season ?? 'present'}
                              </td>
                              <td>
                                {run.name}
                                <br />
                                <span className="sd">{run.location}</span>
                              </td>
                            </tr>
                          )),
                        )}
                    </tbody>
                  </table>
                </div>
                <p>
                  Location means the franchise’s home market or region. Venue changes within a market do not change its identity.
                </p>
              </div>
            </details>
            <details className="panel methodology">
              <summary>How these predictions work</summary>
              <div>
                <p>
                  Rally Row builds team ratings from past seasons and updates them as the current season progresses. For each
                  matchup, we compare those ratings to show each team’s chance of winning.
                </p>
                <p>
                  Historical games determine Elo ratings, with ties scored as half a win. An offseason adjustment moves ratings
                  toward the league average. Those ratings become the prior means for a Bayesian model of team strength.
                </p>
                <p>
                  The model learns from this season’s wins, losses, and ties, accounts for the opponent and venue, and integrates
                  over uncertainty to estimate matchup probabilities. It uses a Davidson extension of the Bradley–Terry model with a
                  Laplace approximation. It does not use scores, injuries, rosters, or betting markets.
                </p>
                <p>{state.result_policy} Games without a confirmed outcome do not update the ratings.</p>
                <p>
                  The parameters are starting settings, not a claim of validated forecasting accuracy. The project includes a
                  chronological backtest command.
                </p>
                <a href={state.source} target="_blank" rel="noreferrer">
                  View the data source ↗
                </a>
              </div>
            </details>
          </>
        )}
      </main>
      <footer>
        <div className="footer-inner">
          <strong>Rally Row · v{APP_VERSION}</strong>
          <span>A little insight. A lot to talk about.</span>
        </div>
      </footer>
    </>
  );
}
