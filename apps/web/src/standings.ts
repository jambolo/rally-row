import { teamIdentity, type Game, type LeagueConfig, type PostseasonFormat } from './contracts.ts';

export type TiebreakRule = PostseasonFormat['tiebreakers']['division'][number];
export type Tiebreakers = PostseasonFormat['tiebreakers'];
export type Outcome = NonNullable<Game['result']>;

/** Team index = position in `ids`. Conference and division lists follow config order; team lists ascend by index. */
export type Alignment = {
  format: PostseasonFormat;
  ids: readonly string[];
  index: Map<string, number>;
  conferences: { id: string; name: string; divisions: number[]; teams: number[] }[];
  divisions: { id: string; name: string; label: string; conference: number; teams: number[] }[];
  conferenceOf: Int32Array;
  divisionOf: Int32Array;
};

export type StandingsCounts = {
  /** Per game: 0 unplayed, 1 home win, 2 away win, 3 tie. */
  outcome: Int8Array;
  /** `wins[i * n + j]`: wins of team i over team j. */
  wins: Int16Array;
  /** `ties[i * n + j] === ties[j * n + i]`: ties between teams i and j. */
  ties: Int16Array;
  teamWins: Int16Array;
  teamLosses: Int16Array;
  teamTies: Int16Array;
};

export type Standings = StandingsCounts & {
  alignment: Alignment;
  n: number;
  home: Int32Array;
  away: Int32Array;
  /** Team t's conference games, ascending: `conferenceGames[conferenceStart[t] .. conferenceStart[t + 1] - 1]`. */
  conferenceGames: Int32Array;
  conferenceStart: Int32Array;
  base: StandingsCounts;
};

export function alignLeague(config: LeagueConfig, ids: readonly string[], season: number): Alignment {
  const format = config.postseason;
  if (!format) throw new Error('League has no postseason format');
  const conferences: Alignment['conferences'] = [];
  const divisions: Alignment['divisions'] = [];
  const divisionIndex = new Map<string, number>();
  format.conferences.forEach((c, ci) => {
    const conference = { id: c.id, name: c.name, divisions: [] as number[], teams: [] as number[] };
    for (const d of c.divisions) {
      divisionIndex.set(d.id, divisions.length);
      conference.divisions.push(divisions.length);
      divisions.push({ id: d.id, name: d.name, label: d.short ?? d.name, conference: ci, teams: [] });
    }
    conferences.push(conference);
  });
  const conferenceOf = new Int32Array(ids.length);
  const divisionOf = new Int32Array(ids.length);
  const index = new Map<string, number>();
  ids.forEach((id, i) => {
    index.set(id, i);
    const division = teamIdentity(config, id, season).division;
    const d = division === undefined ? undefined : divisionIndex.get(division);
    if (d === undefined) throw new Error(`No division for ${id} in ${season}`);
    const c = divisions[d].conference;
    divisionOf[i] = d;
    conferenceOf[i] = c;
    divisions[d].teams.push(i);
    conferences[c].teams.push(i);
  });
  return { format, ids, index, conferences, divisions, conferenceOf, divisionOf };
}

function counts(games: number, n: number): StandingsCounts {
  return {
    outcome: new Int8Array(games),
    wins: new Int16Array(n * n),
    ties: new Int16Array(n * n),
    teamWins: new Int16Array(n),
    teamLosses: new Int16Array(n),
    teamTies: new Int16Array(n),
  };
}

export function createStandings(alignment: Alignment, games: readonly { home: number; away: number }[]): Standings {
  const n = alignment.ids.length;
  const home = Int32Array.from(games, (g) => g.home);
  const away = Int32Array.from(games, (g) => g.away);
  const perTeam: number[][] = Array.from({ length: n }, () => []);
  games.forEach((g, i) => {
    if (alignment.conferenceOf[g.home] !== alignment.conferenceOf[g.away]) return;
    perTeam[g.home].push(i);
    perTeam[g.away].push(i);
  });
  const conferenceStart = new Int32Array(n + 1);
  perTeam.forEach((list, t) => {
    conferenceStart[t + 1] = conferenceStart[t] + list.length;
  });
  const conferenceGames = Int32Array.from(perTeam.flat());
  return {
    ...counts(games.length, n),
    alignment,
    n,
    home,
    away,
    conferenceGames,
    conferenceStart,
    base: counts(games.length, n),
  };
}

export function recordGame(s: Standings, game: number, outcome: Outcome): void {
  const h = s.home[game];
  const a = s.away[game];
  if (outcome === 'home_win') {
    s.outcome[game] = 1;
    s.wins[h * s.n + a]++;
    s.teamWins[h]++;
    s.teamLosses[a]++;
  } else if (outcome === 'away_win') {
    s.outcome[game] = 2;
    s.wins[a * s.n + h]++;
    s.teamWins[a]++;
    s.teamLosses[h]++;
  } else {
    s.outcome[game] = 3;
    s.ties[h * s.n + a]++;
    s.ties[a * s.n + h]++;
    s.teamTies[h]++;
    s.teamTies[a]++;
  }
}

export function saveBase(s: Standings): void {
  s.base.outcome.set(s.outcome);
  s.base.wins.set(s.wins);
  s.base.ties.set(s.ties);
  s.base.teamWins.set(s.teamWins);
  s.base.teamLosses.set(s.teamLosses);
  s.base.teamTies.set(s.teamTies);
}

export function resetToBase(s: Standings): void {
  s.outcome.set(s.base.outcome);
  s.wins.set(s.base.wins);
  s.ties.set(s.base.ties);
  s.teamWins.set(s.base.teamWins);
  s.teamLosses.set(s.base.teamLosses);
  s.teamTies.set(s.base.teamTies);
}

export function winPercentage(s: Standings, team: number): number {
  const w = s.teamWins[team];
  const t = s.teamTies[team];
  const g = w + s.teamLosses[team] + t;
  return g === 0 ? 0 : (2 * w + t) / (2 * g);
}

/** Mutable so a seeding pass can report whether any tie fell through to a random draw. */
export type TieState = { rng: () => number; usedRandom: boolean };

const ratio = (a: number, b: number): number => (b === 0 ? 0 : a / b);
const gp = (s: Standings, i: number, j: number): number => s.wins[i * s.n + j] + s.wins[j * s.n + i] + s.ties[i * s.n + j];

/** Win percentage of `i` against the teams in `set` as one integer division. */
function pctVs(s: Standings, i: number, set: readonly number[]): number {
  let num = 0;
  let den = 0;
  for (const j of set) {
    if (j === i) continue;
    num += 2 * s.wins[i * s.n + j] + s.ties[i * s.n + j];
    den += 2 * gp(s, i, j);
  }
  return ratio(num, den);
}

function topByValue(group: readonly number[], value: (m: number) => number): number[] {
  const values = group.map(value);
  const max = Math.max(...values);
  return group.filter((_, k) => values[k] === max);
}

function opponentStrength(s: Standings, weight: (j: number) => number): number {
  let num = 0;
  let den = 0;
  for (let j = 0; j < s.n; j++) {
    const w = weight(j);
    if (w === 0) continue;
    num += w * (2 * s.teamWins[j] + s.teamTies[j]);
    den += w * 2 * (s.teamWins[j] + s.teamLosses[j] + s.teamTies[j]);
  }
  return ratio(num, den);
}

function playedConferenceGames(s: Standings, t: number): number {
  let count = 0;
  for (let p = s.conferenceStart[t]; p < s.conferenceStart[t + 1]; p++) if (s.outcome[s.conferenceGames[p]] !== 0) count++;
  return count;
}

/** Record over the last `size` played conference games of `t`, walking the schedule backwards. */
function lastGamesValue(s: Standings, t: number, size: number): number {
  let seen = 0;
  let points = 0;
  for (let p = s.conferenceStart[t + 1] - 1; p >= s.conferenceStart[t] && seen < size; p--) {
    const g = s.conferenceGames[p];
    const o = s.outcome[g];
    if (o === 0) continue;
    seen++;
    if (o === 3) points += 1;
    else if ((o === 1) === (s.home[g] === t)) points += 2;
  }
  return ratio(points, 2 * size);
}

function compareLastHalf(s: Standings, x: number, y: number): number {
  const lx = playedConferenceGames(s, x);
  const ly = playedConferenceGames(s, y);
  const hx = Math.ceil(lx / 2);
  const hy = Math.ceil(ly / 2);
  for (let k = 0; k <= lx - hx && k <= ly - hy; k++) {
    const a = lastGamesValue(s, x, hx + k);
    const b = lastGamesValue(s, y, hy + k);
    if (a !== b) return a > b ? 1 : -1;
  }
  return 0;
}

function topClass(s: Standings, rule: TiebreakRule, group: readonly number[]): number[] | null {
  const a = s.alignment;
  switch (rule.rule) {
    case 'head_to_head': {
      for (const m of group) {
        let games = 0;
        for (const j of group) if (j !== m) games += gp(s, m, j);
        if (games === 0) return null;
      }
      return topByValue(group, (m) => pctVs(s, m, group));
    }
    case 'head_to_head_sweep': {
      if (group.length === 2) return topClass(s, { rule: 'head_to_head' }, group);
      const beat = (i: number, j: number) => s.wins[i * s.n + j] >= 1 && s.wins[j * s.n + i] === 0 && s.ties[i * s.n + j] === 0;
      const sweeper = group.find((m) => group.every((j) => j === m || beat(m, j)));
      if (sweeper !== undefined) return [sweeper];
      const swept = group.find((m) => group.every((j) => j === m || beat(j, m)));
      return swept === undefined ? null : group.filter((m) => m !== swept);
    }
    case 'division_record':
      return topByValue(group, (m) => pctVs(s, m, a.divisions[a.divisionOf[m]].teams));
    case 'conference_record':
      return topByValue(group, (m) => pctVs(s, m, a.conferences[a.conferenceOf[m]].teams));
    case 'common_games': {
      const common: number[] = [];
      for (let j = 0; j < s.n; j++) {
        if (group.includes(j)) continue;
        if (group.every((m) => gp(s, m, j) >= 1)) common.push(j);
      }
      const need = Math.max(1, rule.min_games ?? 0);
      for (const m of group) {
        let games = 0;
        for (const j of common) games += gp(s, m, j);
        if (games < need) return null;
      }
      return topByValue(group, (m) => pctVs(s, m, common));
    }
    case 'strength_of_victory':
      return topByValue(group, (m) => opponentStrength(s, (j) => s.wins[m * s.n + j]));
    case 'strength_of_schedule':
      return topByValue(group, (m) => opponentStrength(s, (j) => gp(s, m, j)));
    case 'last_half_conference':
      return group.filter((m) => !group.some((x) => x !== m && compareLastHalf(s, x, m) > 0));
  }
}

export function pickBest(s: Standings, group: readonly number[], rules: readonly TiebreakRule[], tie: TieState): number {
  if (group.length === 1) return group[0];
  for (const rule of rules) {
    const top = topClass(s, rule, group);
    if (top !== null && top.length < group.length) return pickBest(s, top, rules, tie);
  }
  const sorted = [...group].sort((x, y) => x - y);
  tie.usedRandom = true;
  return sorted[Math.floor(tie.rng() * sorted.length)];
}

export function rankTeams(s: Standings, group: readonly number[], rules: readonly TiebreakRule[], tie: TieState): number[] {
  const rest = [...group];
  const ranked: number[] = [];
  while (rest.length > 0) {
    const best = pickBest(s, rest, rules, tie);
    ranked.push(best);
    rest.splice(rest.indexOf(best), 1);
  }
  return ranked;
}

export type Seeding = {
  /** `seeds[c][k]`: team index of seed k + 1 in conference c; length `teams_per_conference`. */
  seeds: Int32Array[];
  /** Per division (alignment order): the team ranked first. */
  divisionWinners: Int32Array;
  /** Per team: 1-based rank within its division. */
  divisionRank: Int32Array;
  /** Per team: seed (1-based), 0 when not seeded. */
  seedOf: Int32Array;
  usedRandom: boolean;
};

export function createSeeding(alignment: Alignment): Seeding {
  const { format, conferences, divisions, ids } = alignment;
  return {
    seeds: conferences.map(() => new Int32Array(format.teams_per_conference)),
    divisionWinners: new Int32Array(divisions.length),
    divisionRank: new Int32Array(ids.length),
    seedOf: new Int32Array(ids.length),
    usedRandom: false,
  };
}

export function seedLeague(
  s: Standings,
  rng: () => number,
  out: Seeding,
  tiebreakers: Tiebreakers = s.alignment.format.tiebreakers,
): Seeding {
  const al = s.alignment;
  const tie: TieState = { rng, usedRandom: false };
  out.seedOf.fill(0);

  al.divisions.forEach((div, d) => {
    const ordered = [...div.teams].sort((a, b) => winPercentage(s, b) - winPercentage(s, a));
    let rank = 1;
    for (let i = 0; i < ordered.length;) {
      const pct = winPercentage(s, ordered[i]);
      let j = i;
      while (j < ordered.length && winPercentage(s, ordered[j]) === pct) j++;
      const group = ordered.slice(i, j);
      for (const t of group.length === 1 ? group : rankTeams(s, group, tiebreakers.division, tie)) {
        out.divisionRank[t] = rank++;
        if (rank === 2) out.divisionWinners[d] = t;
      }
      i = j;
    }
  });

  const select = (pool: number[]): number => {
    let best = -1;
    for (const t of pool) best = Math.max(best, winPercentage(s, t));
    let group = pool.filter((t) => winPercentage(s, t) === best);
    if (tiebreakers.one_per_division) {
      group = group.filter((t) =>
        group.every((u) => al.divisionOf[u] !== al.divisionOf[t] || out.divisionRank[u] >= out.divisionRank[t]),
      );
    }
    const pick = pickBest(s, group, tiebreakers.conference, tie);
    pool.splice(pool.indexOf(pick), 1);
    return pick;
  };

  const limit = al.format.teams_per_conference;
  al.conferences.forEach((conf, c) => {
    let seed = 1;
    const place = (t: number): void => {
      out.seeds[c][seed - 1] = t;
      out.seedOf[t] = seed;
      seed++;
    };
    let pool: number[];
    if (al.format.division_winners_first) {
      pool = conf.divisions.map((d) => out.divisionWinners[d]);
      while (pool.length > 0) place(select(pool));
      pool = conf.teams.filter((t) => out.seedOf[t] === 0);
    } else pool = [...conf.teams];
    while (seed <= limit && pool.length > 0) place(select(pool));
  });

  out.usedRandom = tie.usedRandom;
  return out;
}
