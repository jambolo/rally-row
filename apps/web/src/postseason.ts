import type { Game } from './contracts.ts';
import { homeAdvantageLogit, outcomeInto, posteriorSampler, tieWeightIn, type Posterior, type Probabilities } from './model.ts';
import { normalSource, sfc32, SIMULATION_SEED } from './random.ts';
import {
  alignLeague,
  createSeeding,
  createStandings,
  pickBest,
  recordGame,
  resetToBase,
  saveBase,
  seedLeague,
  winPercentage,
  type Alignment,
  type Seeding,
  type Standings,
  type TieState,
  type Tiebreakers,
} from './standings.ts';

/** Structural subset of the service's game view, so the engine does not depend on the service. */
export type SimulationGame = Pick<Game, 'id' | 'phase' | 'round_label' | 'home_team' | 'away_team' | 'neutral' | 'result'> & {
  status: 'completed' | 'scheduled' | 'awaiting_result';
};

export const POSTSEASON_SIMULATIONS = 10_000;

export type TeamStatus =
  | { kind: 'out' }
  | { kind: 'pending' }
  | { kind: 'qualified' }
  | { kind: 'bye' }
  | { kind: 'champion' }
  | { kind: 'series'; round: number; wins: number; losses: number; opponent: string }
  | { kind: 'advanced'; round: number }
  | { kind: 'eliminated'; round: number };

export type TeamOdds = {
  id: string;
  division: string;
  division_label: string;
  record: { wins: number; losses: number; ties: number };
  mean_seed: number | null;
  playoffs: number;
  win_division: number;
  bye: number;
  reach: number[];
  title: number;
  status: TeamStatus | null;
};

export type PostseasonOdds = {
  status: 'ready';
  mode: 'regular' | 'postseason';
  simulations: number;
  playoff_spots: number;
  byes: number;
  rounds: { name: string; short: string; games: number }[];
  conferences: { id: string; name: string; teams: TeamOdds[] }[];
  notes: string[];
};

export type PostseasonState = PostseasonOdds | { status: 'error'; error: string };

export type SimulationOptions = { simulations?: number; seed?: number };

/** Seeds in bracket slot order, so the top seeds can meet only in the latest rounds. */
export function bracketOrder(size: number): number[] {
  if (size <= 1) return [1];
  return bracketOrder(size / 2).flatMap((s) => [s, size + 1 - s]);
}

export function formatOdds(p: number): string {
  if (p <= 0) return '0%';
  if (p >= 1) return '100%';
  const r = Math.round(p * 100);
  if (r < 1) return '<1%';
  if (r > 99) return '>99%';
  return `${r}%`;
}

export function formatRecord(record: TeamOdds['record']): string {
  return record.ties > 0 ? `${record.wins}-${record.losses}-${record.ties}` : `${record.wins}-${record.losses}`;
}

export function statusText(status: TeamStatus | null, rounds: PostseasonOdds['rounds']): string {
  if (status === null) return '';
  switch (status.kind) {
    case 'out':
      return 'Out';
    case 'pending':
      return 'Pending';
    case 'qualified':
      return 'Qualified';
    case 'bye':
      return 'Bye';
    case 'champion':
      return 'Champion';
    case 'series':
      return `${rounds[status.round].short} ${status.wins}-${status.losses} vs ${status.opponent}`;
    case 'advanced':
      return `Won ${rounds[status.round].short}`;
    case 'eliminated':
      return `Lost ${rounds[status.round].short}`;
  }
}

const MIN_LISTED_PLAYOFFS = 0.001;

/** Whether the odds table lists a team: playoff contenders in the regular season, teams still playing afterward. */
export function isListed(team: TeamOdds, mode: PostseasonOdds['mode']): boolean {
  if (mode === 'regular') return team.playoffs >= MIN_LISTED_PLAYOFFS;
  return team.status?.kind !== 'out' && team.status?.kind !== 'eliminated';
}

const UNFIT = "The listed postseason games don't fit the configured playoff format.";
const PROBE_SEEDINGS = 2000;
const MAX_ATTEMPTS = 1000;

/** Listed postseason series, one per (round, unordered pair), in first-listed order. */
type Observed = {
  round: number[];
  a: number[];
  b: number[];
  winsA: number[];
  winsB: number[];
  /** `byRound[r * n + t]`: index of team t's round-r series, -1 if none. */
  byRound: Int32Array;
  /** Highest round of any series listing the team, -1 if none. */
  maxRound: Int32Array;
  /** Per play: 1 once the simulated bracket paired the series. */
  played: Uint8Array;
};

/** Buffers reused by every bracket play, so the simulation loop allocates nothing per season. */
type Bracket = {
  al: Alignment;
  s: Standings;
  hfa: number;
  size: number;
  conferenceRounds: number;
  order: number[];
  clinch: number[];
  /** `slots[c * size + k]`: conference c's team in slot k, -1 for an empty slot. */
  slots: Int32Array;
  champions: Int32Array;
  /** Per team: the last round it was alive at the start of, rounds.length for the champion, -1 if not seeded. */
  alive: Int32Array;
  probabilities: Probabilities;
  uniform: () => number;
  tie: TieState;
  observed: Observed | null;
};

/**
 * Returns the series winner, or -1 when the listed games rule out this pairing. Listed wins count toward the clinch
 * number and the venue pattern continues after them, so a series in progress resumes where it stands.
 */
function playSeries(b: Bracket, r: number, x: number, y: number, seeding: Seeding, theta: Float64Array): number {
  const o = b.observed;
  const clinch = b.clinch[r];
  let wx = 0;
  let wy = 0;
  if (o !== null) {
    const n = b.al.ids.length;
    const sx = o.byRound[r * n + x];
    if (sx !== o.byRound[r * n + y]) return -1;
    if (sx >= 0) {
      o.played[sx] = 1;
      wx = o.a[sx] === x ? o.winsA[sx] : o.winsB[sx];
      wy = o.a[sx] === x ? o.winsB[sx] : o.winsA[sx];
    }
    // A team listed in a later round must have won this one.
    const fx = o.maxRound[x] > r;
    const fy = o.maxRound[y] > r;
    if (fx && fy) return -1;
    if (fx) return wy >= clinch ? -1 : x;
    if (fy) return wx >= clinch ? -1 : y;
  }
  const round = b.al.format.rounds[r];
  let h: number;
  if (round.home === 'better_record') {
    const px = winPercentage(b.s, x);
    const py = winPercentage(b.s, y);
    h = px > py ? x : py > px ? y : pickBest(b.s, [x, y], b.al.format.tiebreakers.conference, b.tie);
  } else if (r < b.conferenceRounds) h = seeding.seedOf[x] < seeding.seedOf[y] ? x : y;
  else h = b.al.conferenceOf[x] < b.al.conferenceOf[y] ? x : y;
  const a = h === x ? y : x;
  let wh = h === x ? wx : wy;
  let wa = h === x ? wy : wx;
  const p = b.probabilities;
  while (wh < clinch && wa < clinch) {
    const venue = round.pattern[wh + wa];
    if (venue === 'A') {
      outcomeInto(theta[a] - theta[h] + b.hfa, 0, p);
      if (b.uniform() < p.home_win) wa++;
      else wh++;
    } else {
      outcomeInto(theta[h] - theta[a] + (venue === 'H' ? b.hfa : 0), 0, p);
      if (b.uniform() < p.home_win) wh++;
      else wa++;
    }
  }
  return wh >= clinch ? h : a;
}

/** Plays every round into `b.alive`; false when the listed games reject this play. */
function playBracket(b: Bracket, seeding: Seeding, theta: Float64Array): boolean {
  const { al, size, slots, champions, alive, observed } = b;
  const format = al.format;
  const conferences = al.conferences.length;
  alive.fill(-1);
  if (observed !== null) observed.played.fill(0);
  for (let c = 0; c < conferences; c++)
    for (let k = 0; k < size; k++) {
      const seed = b.order[k];
      const t = seed <= format.teams_per_conference ? seeding.seeds[c][seed - 1] : -1;
      slots[c * size + k] = t;
      if (t >= 0) alive[t] = 0;
    }
  for (let r = 0; r < format.rounds.length; r++) {
    if (r < b.conferenceRounds) {
      const m = size >> r;
      const reseed = r > 0 && format.reseed;
      for (let c = 0; c < conferences; c++) {
        const base = c * size;
        if (reseed)
          for (let i = 1; i < m; i++) {
            const t = slots[base + i];
            let j = i - 1;
            for (; j >= 0 && seeding.seedOf[slots[base + j]] > seeding.seedOf[t]; j--) slots[base + j + 1] = slots[base + j];
            slots[base + j + 1] = t;
          }
        for (let i = 0; i < m / 2; i++) {
          const x = slots[base + (reseed ? i : 2 * i)];
          const y = slots[base + (reseed ? m - 1 - i : 2 * i + 1)];
          // An empty slot is a bye: its partner advances without playing.
          const w = x < 0 ? y : y < 0 ? x : playSeries(b, r, x, y, seeding, theta);
          if (w < 0) return false;
          alive[w] = r + 1;
          slots[base + i] = w;
        }
        champions[c] = slots[base];
      }
    } else {
      const m = conferences >> (r - b.conferenceRounds);
      for (let i = 0; i < m / 2; i++) {
        const w = playSeries(b, r, champions[2 * i], champions[2 * i + 1], seeding, theta);
        if (w < 0) return false;
        alive[w] = r + 1;
        champions[i] = w;
      }
    }
    if (observed !== null)
      for (let k = 0; k < observed.round.length; k++) if (observed.round[k] === r && observed.played[k] === 0) return false;
  }
  return true;
}

function observeSeries(al: Alignment, games: readonly SimulationGame[], notes: string[]): Observed {
  const n = al.ids.length;
  const rounds = al.format.rounds;
  const o: Observed = {
    round: [],
    a: [],
    b: [],
    winsA: [],
    winsB: [],
    byRound: new Int32Array(rounds.length * n).fill(-1),
    maxRound: new Int32Array(n).fill(-1),
    played: new Uint8Array(0),
  };
  const unknown: string[] = [];
  for (const g of games) {
    if (g.phase !== 'postseason') continue;
    const r = rounds.findIndex((round) => round.round_label === g.round_label);
    if (r < 0) {
      if (!unknown.includes(g.round_label)) unknown.push(g.round_label);
      continue;
    }
    const h = al.index.get(g.home_team)!;
    const v = al.index.get(g.away_team)!;
    let k = o.byRound[r * n + h];
    if (k < 0 || (o.a[k] !== v && o.b[k] !== v)) {
      if (o.byRound[r * n + h] >= 0) throw new Error(`Conflicting ${rounds[r].name} series for ${g.home_team}`);
      if (o.byRound[r * n + v] >= 0) throw new Error(`Conflicting ${rounds[r].name} series for ${g.away_team}`);
      k = o.round.length;
      o.round.push(r);
      o.a.push(h);
      o.b.push(v);
      o.winsA.push(0);
      o.winsB.push(0);
      o.byRound[r * n + h] = k;
      o.byRound[r * n + v] = k;
      o.maxRound[h] = Math.max(o.maxRound[h], r);
      o.maxRound[v] = Math.max(o.maxRound[v], r);
    }
    if (g.status !== 'completed' || (g.result !== 'home_win' && g.result !== 'away_win')) continue;
    if ((g.result === 'home_win' ? h : v) === o.a[k]) o.winsA[k]++;
    else o.winsB[k]++;
  }
  for (const label of unknown) notes.push(`Ignored postseason games with unknown round label "${label}".`);
  for (let k = 0; k < o.round.length; k++) {
    const round = rounds[o.round[k]];
    const clinch = (round.pattern.length + 1) / 2;
    const [x, y] = [al.ids[o.a[k]], al.ids[o.b[k]]].sort((p, q) => (p < q ? -1 : p > q ? 1 : 0));
    if (o.winsA[k] > clinch || o.winsB[k] > clinch)
      throw new Error(`Too many wins in the ${round.name} series between ${x} and ${y}`);
    if (o.winsA[k] >= clinch && o.winsB[k] >= clinch)
      throw new Error(`Both teams clinched the ${round.name} series between ${x} and ${y}`);
  }
  o.played = new Uint8Array(o.round.length);
  return o;
}

function copySeeding(al: Alignment, from: Seeding): Seeding {
  const to = createSeeding(al);
  from.seeds.forEach((seeds, c) => to.seeds[c].set(seeds));
  to.divisionWinners.set(from.divisionWinners);
  to.divisionRank.set(from.divisionRank);
  to.seedOf.set(from.seedOf);
  to.usedRandom = from.usedRandom;
  return to;
}

/** Seedings that fit the listed games, kept with multiplicity so random tiebreaks keep their weights. */
function probeSeedings(b: Bracket, means: Float64Array, tiebreakers: Tiebreakers): Seeding[] {
  const draw = createSeeding(b.al);
  const accepted: Seeding[] = [];
  seedLeague(b.s, b.uniform, draw, tiebreakers);
  const count = draw.usedRandom ? PROBE_SEEDINGS : 1;
  for (let i = 0; i < count; i++) {
    if (i > 0) seedLeague(b.s, b.uniform, draw, tiebreakers);
    if (playBracket(b, draw, means)) accepted.push(copySeeding(b.al, draw));
  }
  return accepted;
}

function teamStatus(al: Alignment, o: Observed, t: number, playoffs: number, bye: number): TeamStatus {
  const r = o.maxRound[t];
  if (r < 0) {
    if (playoffs === 0) return { kind: 'out' };
    if (playoffs === 1) return bye === 1 ? { kind: 'bye' } : { kind: 'qualified' };
    return { kind: 'pending' };
  }
  const k = o.byRound[r * al.ids.length + t];
  const first = o.a[k] === t;
  const wins = first ? o.winsA[k] : o.winsB[k];
  const losses = first ? o.winsB[k] : o.winsA[k];
  const opponent = first ? o.b[k] : o.a[k];
  const rounds = al.format.rounds;
  const clinch = (rounds[r].pattern.length + 1) / 2;
  if (wins >= clinch) return r === rounds.length - 1 ? { kind: 'champion' } : { kind: 'advanced', round: r };
  if (losses >= clinch || o.maxRound[opponent] > r) return { kind: 'eliminated', round: r };
  return { kind: 'series', round: r, wins, losses, opponent: al.ids[opponent] };
}

export function simulatePostseason(model: Posterior, games: SimulationGame[], options: SimulationOptions = {}): PostseasonOdds {
  const simulations = options.simulations ?? POSTSEASON_SIMULATIONS;
  const al = alignLeague(model.config, model.ids, model.seed.target_season);
  const format = al.format;
  const n = al.ids.length;
  const notes: string[] = [];

  let regular = games.filter((g) => g.phase === 'regular');
  const unplayed = regular.filter((g) => g.status !== 'completed').length;
  if (unplayed > 0 && games.some((g) => g.phase === 'postseason' && g.status === 'completed')) {
    regular = regular.filter((g) => g.status === 'completed');
    notes.push(`Ignored ${unplayed} unplayed regular-season game${unplayed === 1 ? '' : 's'} because the postseason has started.`);
  }
  const mode = regular.some((g) => g.status !== 'completed') ? 'regular' : 'postseason';

  const s = createStandings(
    al,
    regular.map((g) => ({ home: al.index.get(g.home_team)!, away: al.index.get(g.away_team)! })),
  );
  const toPlay: number[] = [];
  regular.forEach((g, i) => {
    if (g.status !== 'completed') toPlay.push(i);
    else if (g.result !== null) recordGame(s, i, g.result);
  });
  saveBase(s);

  const uniform = sfc32(options.seed ?? SIMULATION_SEED);
  const normal = normalSource(uniform);
  const sample = posteriorSampler(model);
  const theta = new Float64Array(n);
  let size = 1;
  while (size < format.teams_per_conference) size *= 2;
  const byes = size - format.teams_per_conference;
  const rounds = format.rounds.length;
  const b: Bracket = {
    al,
    s,
    hfa: homeAdvantageLogit(model.config),
    size,
    conferenceRounds: Math.log2(size),
    order: bracketOrder(size),
    clinch: format.rounds.map((r) => (r.pattern.length + 1) / 2),
    slots: new Int32Array(al.conferences.length * size),
    champions: new Int32Array(al.conferences.length),
    alive: new Int32Array(n),
    probabilities: { home_win: 0, away_win: 0, tie: 0 },
    uniform,
    tie: { rng: uniform, usedRandom: false },
    observed: null,
  };

  const playoffs = new Int32Array(n);
  const winDivision = new Int32Array(n);
  const bye = new Int32Array(n);
  const seedSum = new Float64Array(n);
  const reach = new Int32Array(n * (rounds - 1));
  const title = new Int32Array(n);
  const tally = (seeding: Seeding): void => {
    for (let t = 0; t < n; t++) {
      const seed = seeding.seedOf[t];
      if (seed > 0) {
        playoffs[t]++;
        seedSum[t] += seed;
        if (seed <= byes) bye[t]++;
      }
      const last = b.alive[t];
      for (let i = 0; i < rounds - 1; i++) if (last >= i + 1) reach[t * (rounds - 1) + i]++;
      if (last === rounds) title[t]++;
    }
    for (let d = 0; d < seeding.divisionWinners.length; d++) winDivision[seeding.divisionWinners[d]]++;
  };

  if (mode === 'regular') {
    const remaining = Int32Array.from(toPlay);
    const neutral = Uint8Array.from(regular, (g) => (g.neutral ? 1 : 0));
    const tieWeight = tieWeightIn(model, 'regular');
    const p = b.probabilities;
    const seeding = createSeeding(al);
    for (let sim = 0; sim < simulations; sim++) {
      sample(normal, theta);
      resetToBase(s);
      for (let k = 0; k < remaining.length; k++) {
        const g = remaining[k];
        outcomeInto(theta[s.home[g]] - theta[s.away[g]] + (neutral[g] ? 0 : b.hfa), tieWeight, p);
        const u = uniform();
        recordGame(s, g, u < p.home_win ? 'home_win' : u < p.home_win + p.tie ? 'tie' : 'away_win');
      }
      seedLeague(s, uniform, seeding);
      playBracket(b, seeding, theta);
      tally(seeding);
    }
  } else {
    b.observed = observeSeries(al, games, notes);
    const means = Float64Array.from(model.means);
    let accepted = probeSeedings(b, means, format.tiebreakers);
    if (accepted.length === 0) {
      accepted = probeSeedings(b, means, { ...format.tiebreakers, division: [], conference: [] });
      notes.push('Computed tiebreakers disagree with the listed postseason games; seeds follow the listed games.');
    }
    if (accepted.length === 0) throw new Error(UNFIT);
    const distinct = new Set(accepted.map((seeding) => seeding.seeds.map((seeds) => seeds.join(',')).join('|')));
    if (distinct.size > 1) notes.push('Some seeds depend on random tiebreak draws.');
    for (let sim = 0; sim < simulations; sim++) {
      sample(normal, theta);
      for (let attempt = 1; ; attempt++) {
        const seeding = accepted[Math.floor(uniform() * accepted.length)];
        if (playBracket(b, seeding, theta)) {
          tally(seeding);
          break;
        }
        if (attempt === MAX_ATTEMPTS) throw new Error(UNFIT);
      }
    }
  }

  const observed = b.observed;
  const percentage = (r: TeamOdds['record']): number => {
    const g = r.wins + r.losses + r.ties;
    return g === 0 ? 0 : (2 * r.wins + r.ties) / (2 * g);
  };
  const odds = (t: number): TeamOdds => {
    const division = al.divisions[al.divisionOf[t]];
    const team = {
      id: al.ids[t],
      division: division.id,
      division_label: division.label,
      record: { wins: s.base.teamWins[t], losses: s.base.teamLosses[t], ties: s.base.teamTies[t] },
      mean_seed: playoffs[t] === 0 ? null : seedSum[t] / playoffs[t],
      playoffs: playoffs[t] / simulations,
      win_division: winDivision[t] / simulations,
      bye: bye[t] / simulations,
      reach: Array.from({ length: rounds - 1 }, (_, i) => reach[t * (rounds - 1) + i] / simulations),
      title: title[t] / simulations,
    };
    return { ...team, status: observed === null ? null : teamStatus(al, observed, t, team.playoffs, team.bye) };
  };
  return {
    status: 'ready',
    mode,
    simulations,
    playoff_spots: format.teams_per_conference,
    byes,
    rounds: format.rounds.map((r) => ({ name: r.name, short: r.short, games: r.pattern.length })),
    conferences: al.conferences.map((c) => ({
      id: c.id,
      name: c.name,
      teams: c.teams
        .map(odds)
        .sort(
          (x, y) =>
            y.playoffs - x.playoffs ||
            y.title - x.title ||
            percentage(y.record) - percentage(x.record) ||
            (x.id < y.id ? -1 : x.id > y.id ? 1 : 0),
        ),
    })),
    notes,
  };
}

export function postseasonState(model: Posterior, games: SimulationGame[], options?: SimulationOptions): PostseasonState {
  try {
    return simulatePostseason(model, games, options);
  } catch (e) {
    return { status: 'error', error: e instanceof Error ? e.message : String(e) };
  }
}
