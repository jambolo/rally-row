import { readFileSync } from 'node:fs';
import { configSchema } from '../src/contracts.ts';
import type { Game, LeagueConfig } from '../src/contracts.ts';
import type { Posterior } from '../src/model.ts';
import { alignLeague } from '../src/standings.ts';
import { seed } from './helpers.ts';

/** Same shape as the engine's `SimulationGame`, declared here so the helpers do not depend on the engine. */
export type TestGame = Pick<Game, 'id' | 'phase' | 'round_label' | 'home_team' | 'away_team' | 'neutral' | 'result'> & {
  status: 'completed' | 'scheduled' | 'awaiting_result';
};
export type Row = [home: string, away: string, outcome: 'H' | 'A' | 'T' | null];

function load(id: string): LeagueConfig {
  return configSchema.parse(JSON.parse(readFileSync(new URL(`../../../config/${id}.json`, import.meta.url), 'utf8')));
}

export const nfl: LeagueConfig = load('nfl');
export const mlb: LeagueConfig = load('mlb');

export function posterior(config: LeagueConfig, means: Record<string, number> = {}, variance = 0.01): Posterior {
  const ids = config.teams.map((t) => t.id);
  const n = ids.length;
  return {
    ids,
    means: ids.map((id) => means[id] ?? 0),
    covariance: Array.from({ length: n }, (_, i) => Array.from({ length: n }, (_, j) => (i === j ? variance : 0))),
    seed: { ...seed(), league: config.id },
    config,
    games_used: 0,
    iterations: 0,
  };
}

export function withHomeAdvantage(config: LeagueConfig, homeAdvantage: number): LeagueConfig {
  return { ...config, elo: { ...config.elo, home_advantage: homeAdvantage } };
}

// Ids must stay unique across every call in the process.
let next = 0;

function build(phase: Game['phase'], roundLabel: string, rows: Row[]): TestGame[] {
  return rows.map(([home, away, outcome]) => ({
    id: `t${next++}`,
    phase,
    round_label: roundLabel,
    home_team: home,
    away_team: away,
    neutral: false,
    result: outcome === 'H' ? 'home_win' : outcome === 'A' ? 'away_win' : outcome === 'T' ? 'tie' : null,
    status: outcome === null ? 'scheduled' : 'completed',
  }));
}

export function regular(rows: Row[]): TestGame[] {
  return build('regular', 'REG', rows);
}

export function postseason(roundLabel: string, rows: Row[]): TestGame[] {
  return build('postseason', roundLabel, rows);
}

export function roundRobin(config: LeagueConfig, rounds: number): TestGame[] {
  const ids = config.teams.map((t) => t.id);
  const n = ids.length;
  const rows: Row[] = [];
  for (let r = 0; r < rounds; r++) {
    const order = [0];
    for (let p = 1; p < n; p++) order.push(1 + ((p - 1 + r) % (n - 1)));
    for (let i = 0; i < n / 2; i++) {
      const x = ids[order[i]!]!;
      const y = ids[order[n - 1 - i]!]!;
      rows.push(r % 2 === 0 ? [x, y, null] : [y, x, null]);
    }
  }
  return regular(rows);
}

export function transitive(config: LeagueConfig): TestGame[] {
  const ids = config.teams.map((t) => t.id);
  const al = alignLeague(config, ids, seed().target_season);
  const rows: Row[] = [];
  for (const conference of al.conferences) {
    const teams = conference.teams;
    for (let i = 0; i < teams.length; i++) {
      for (let j = i + 1; j < teams.length; j++) rows.push([ids[teams[i]!]!, ids[teams[j]!]!, 'H']);
    }
  }
  return regular(rows);
}
