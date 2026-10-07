import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { configSchema, type LeagueConfig } from '../src/contracts.ts';
import {
  alignLeague,
  createStandings,
  recordGame,
  resetToBase,
  saveBase,
  pickBest,
  rankTeams,
  createSeeding,
  seedLeague,
  winPercentage,
  type Alignment,
  type Standings,
  type TiebreakRule,
} from '../src/standings.ts';
import { config as nfl } from './helpers.ts';

const mlb = configSchema.parse(JSON.parse(readFileSync(new URL('../../../config/mlb.json', import.meta.url), 'utf8')));
const idsOf = (c: LeagueConfig) => c.teams.map((t) => t.id);
const NFL = alignLeague(nfl, idsOf(nfl), 2026);
const MLB = alignLeague(mlb, idsOf(mlb), 2026);
const codes = { H: 'home_win', A: 'away_win', T: 'tie' } as const;
type Row = [home: string, away: string, outcome: keyof typeof codes];

function team(al: Alignment, id: string): number {
  const i = al.index.get(id);
  if (i === undefined) throw new Error(`Unknown team ${id}`);
  return i;
}
/** Standings over `rows` (row k is game k) with every row recorded. */
function league(al: Alignment, rows: Row[]): Standings {
  const s = createStandings(
    al,
    rows.map(([home, away]) => ({ home: team(al, home), away: team(al, away) })),
  );
  rows.forEach(([, , outcome], g) => recordGame(s, g, codes[outcome]));
  return s;
}
function names(al: Alignment, teams: ArrayLike<number>): string[] {
  return Array.from(teams, (t) => al.ids[t]);
}

const sample = (): Standings =>
  league(NFL, [
    ['SEA', 'SF', 'H'],
    ['LAR', 'SEA', 'A'],
    ['ARI', 'SEA', 'T'],
    ['SF', 'SEA', 'H'],
  ]);

describe('league alignment', () => {
  it('aligns NFL teams into 2 conferences of 4 divisions of 4 teams', () => {
    expect(NFL.format).toBe(nfl.postseason);
    expect(NFL.ids).toEqual(idsOf(nfl));
    expect(NFL.conferences.map((c) => c.id)).toEqual(['AFC', 'NFC']);
    expect(NFL.conferences.map((c) => c.divisions)).toEqual([
      [0, 1, 2, 3],
      [4, 5, 6, 7],
    ]);
    expect(NFL.conferences.map((c) => c.teams.length)).toEqual([16, 16]);
    expect(NFL.divisions.map((d) => d.id)).toEqual(['AFC-E', 'AFC-N', 'AFC-S', 'AFC-W', 'NFC-E', 'NFC-N', 'NFC-S', 'NFC-W']);
    expect(NFL.divisions.map((d) => d.teams.length)).toEqual([4, 4, 4, 4, 4, 4, 4, 4]);
    expect(NFL.divisions.map((d) => d.conference)).toEqual([0, 0, 0, 0, 1, 1, 1, 1]);
    expect(names(NFL, NFL.divisions[0].teams)).toEqual(['BUF', 'MIA', 'NE', 'NYJ']);
    expect(names(NFL, NFL.divisions[7].teams)).toEqual(['ARI', 'LAR', 'SEA', 'SF']);
    expect(team(NFL, 'SEA')).toBe(27);
    expect(NFL.divisionOf[27]).toBe(7);
    expect(NFL.conferenceOf[27]).toBe(1);
    expect(NFL.divisions[0].label).toBe('East');
  });

  it('aligns MLB teams into 2 conferences of 3 divisions of 5 teams', () => {
    expect(MLB.conferences.map((c) => c.id)).toEqual(['AL', 'NL']);
    expect(MLB.conferences.map((c) => c.name)).toEqual(['American League', 'National League']);
    expect(MLB.divisions.map((d) => d.id)).toEqual(['AL-E', 'AL-C', 'AL-W', 'NL-E', 'NL-C', 'NL-W']);
    expect(MLB.divisions.map((d) => d.teams.length)).toEqual([5, 5, 5, 5, 5, 5]);
    expect(names(MLB, MLB.divisions[2].teams)).toEqual(['ATH', 'HOU', 'LAA', 'SEA', 'TEX']);
    expect(MLB.divisionOf[team(MLB, 'HOU')]).toBe(2);
    expect(MLB.conferenceOf[team(MLB, 'HOU')]).toBe(0);
    expect(MLB.divisions[1].label).toBe('Central');
  });

  it('labels a division by its short name, else its name', () => {
    const clone = structuredClone(nfl);
    delete clone.postseason!.conferences[0].divisions[0].short;
    const al = alignLeague(clone, idsOf(clone), 2026);
    expect(al.divisions[0].label).toBe('AFC East');
    expect(al.divisions[1].label).toBe('North');
  });

  it('throws without a postseason format', () => {
    const clone = structuredClone(nfl);
    delete clone.postseason;
    expect(() => alignLeague(clone, idsOf(clone), 2026)).toThrow('League has no postseason format');
  });

  it('throws when a team has no division', () => {
    const clone = structuredClone(nfl);
    const sea = clone.teams.find((t) => t.id === 'SEA')!;
    delete sea.eras[sea.eras.length - 1].division;
    expect(() => alignLeague(clone, idsOf(clone), 2026)).toThrow('No division for SEA in 2026');
  });
});

describe('standings', () => {
  it('records wins, losses and ties into the matrices and totals', () => {
    const s = sample();
    const [sea, sf, lar, ari] = ['SEA', 'SF', 'LAR', 'ARI'].map((id) => team(NFL, id));
    expect(Array.from(s.outcome)).toEqual([1, 2, 3, 1]);
    expect(s.n).toBe(32);
    expect(s.wins.length).toBe(1024);
    expect(s.wins[sea * 32 + sf]).toBe(1);
    expect(s.wins[sf * 32 + sea]).toBe(1);
    expect(s.wins[sea * 32 + lar]).toBe(1);
    expect(s.wins[lar * 32 + sea]).toBe(0);
    expect(s.ties[sea * 32 + ari]).toBe(1);
    expect(s.ties[ari * 32 + sea]).toBe(1);
    const tot = (t: number) => [s.teamWins[t], s.teamLosses[t], s.teamTies[t]];
    expect(tot(sea)).toEqual([2, 1, 1]);
    expect(tot(sf)).toEqual([1, 1, 0]);
    expect(tot(lar)).toEqual([0, 1, 0]);
    expect(tot(ari)).toEqual([0, 0, 1]);
    expect(s.home[1]).toBe(lar);
    expect(s.away[1]).toBe(sea);
  });

  it('computes win percentage as (2W + T) / (2G)', () => {
    const s = sample();
    expect(winPercentage(s, team(NFL, 'SEA'))).toBe(0.625);
    expect(winPercentage(s, team(NFL, 'SF'))).toBe(0.5);
    expect(winPercentage(s, team(NFL, 'LAR'))).toBe(0);
    expect(winPercentage(s, team(NFL, 'ARI'))).toBe(0.5);
    expect(winPercentage(s, team(NFL, 'KC'))).toBe(0);
  });

  it('resets to the saved base without reallocating', () => {
    const [sea, sf, lar, ari] = ['SEA', 'SF', 'LAR', 'ARI'].map((id) => team(NFL, id));
    const s = createStandings(NFL, [
      { home: sea, away: sf },
      { home: lar, away: sea },
      { home: ari, away: sea },
    ]);
    recordGame(s, 0, 'home_win');
    recordGame(s, 1, 'away_win');
    saveBase(s);
    const wins = s.wins;
    const outcome = s.outcome;
    recordGame(s, 2, 'tie');
    expect(s.teamTies[sea]).toBe(1);
    resetToBase(s);
    expect(s.wins).toBe(wins);
    expect(s.outcome).toBe(outcome);
    expect(Array.from(s.outcome)).toEqual([1, 2, 0]);
    expect(s.teamWins[sea]).toBe(2);
    expect(s.teamTies[sea]).toBe(0);
    expect(s.ties[sea * 32 + ari]).toBe(0);
    expect(Array.from(s.wins)).toEqual(Array.from(s.base.wins));
    recordGame(s, 2, 'tie');
    expect(s.teamTies[sea]).toBe(1);
    resetToBase(s);
    expect(s.teamTies[sea]).toBe(0);
  });

  it('lists conference games per team in schedule order', () => {
    const [sea, sf, lar, kc] = ['SEA', 'SF', 'LAR', 'KC'].map((id) => team(NFL, id));
    const s = createStandings(NFL, [
      { home: sea, away: sf },
      { home: sea, away: kc },
      { home: lar, away: sea },
    ]);
    const slice = (t: number) => Array.from(s.conferenceGames.subarray(s.conferenceStart[t], s.conferenceStart[t + 1]));
    expect(slice(sea)).toEqual([0, 2]);
    expect(slice(sf)).toEqual([0]);
    expect(slice(lar)).toEqual([2]);
    expect(slice(kc)).toEqual([]);
    expect(s.conferenceStart.length).toBe(33);
    expect(s.conferenceStart[32]).toBe(4);
    expect(Array.from(s.outcome)).toEqual([0, 0, 0]);
  });
});

function pick(al: Alignment, rows: Row[], ids: string[], rules: TiebreakRule[], r = 0): [string, boolean] {
  const tie = { rng: () => r, usedRandom: false };
  const best = pickBest(
    league(al, rows),
    ids.map((id) => team(al, id)),
    rules,
    tie,
  );
  return [al.ids[best], tie.usedRandom];
}

describe('tiebreakers', () => {
  const h2h: TiebreakRule[] = [{ rule: 'head_to_head' }];
  const sweep: TiebreakRule[] = [{ rule: 'head_to_head_sweep' }];
  const sov: TiebreakRule = { rule: 'strength_of_victory' };
  const conf: TiebreakRule = { rule: 'conference_record' };
  const lastHalf: TiebreakRule = { rule: 'last_half_conference' };
  const cg8: Row[] = [
    ['KC', 'BUF', 'H'],
    ['BUF', 'DAL', 'H'],
    ['BUF', 'PHI', 'H'],
    ['MIA', 'KC', 'H'],
  ];

  it('head_to_head decides by the combined record among tied teams', () => {
    expect(pick(NFL, [['MIA', 'BUF', 'H']], ['BUF', 'MIA'], h2h)).toEqual(['MIA', false]);
    const rows: Row[] = [
      ['MIA', 'BUF', 'H'],
      ['MIA', 'NE', 'H'],
      ['BUF', 'NE', 'T'],
    ];
    expect(pick(NFL, rows, ['BUF', 'MIA', 'NE'], h2h)).toEqual(['MIA', false]);
  });

  it('head_to_head is skipped unless every tied team played another', () => {
    const rows: Row[] = [
      ['MIA', 'BUF', 'H'],
      ['KC', 'NE', 'H'],
    ];
    expect(pick(NFL, rows, ['BUF', 'MIA', 'NE'], h2h, 0)).toEqual(['BUF', true]);
  });

  it('head_to_head_sweep equals head_to_head for two teams', () => {
    expect(pick(NFL, [['MIA', 'BUF', 'H']], ['BUF', 'MIA'], sweep)).toEqual(['MIA', false]);
  });

  it('head_to_head_sweep puts a three-team sweeper first', () => {
    const rows: Row[] = [
      ['NE', 'BUF', 'H'],
      ['MIA', 'NE', 'A'],
      ['BUF', 'MIA', 'H'],
      ['MIA', 'BUF', 'H'],
    ];
    expect(pick(NFL, rows, ['BUF', 'MIA', 'NE'], sweep)).toEqual(['NE', false]);
  });

  it('head_to_head_sweep eliminates a team swept by the others', () => {
    const rows: Row[] = [
      ['MIA', 'BUF', 'H'],
      ['BUF', 'NE', 'A'],
      ['MIA', 'NE', 'H'],
      ['NE', 'MIA', 'H'],
    ];
    expect(pick(NFL, rows, ['BUF', 'MIA', 'NE'], sweep, 0)).toEqual(['MIA', true]);
    expect(pick(NFL, rows, ['BUF', 'MIA', 'NE'], sweep, 0.99)).toEqual(['NE', true]);
  });

  it('head_to_head_sweep is skipped without a sweep', () => {
    const rows: Row[] = [
      ['BUF', 'MIA', 'H'],
      ['MIA', 'NE', 'H'],
      ['NE', 'BUF', 'H'],
    ];
    expect(pick(NFL, rows, ['BUF', 'MIA', 'NE'], sweep, 0.5)).toEqual(['MIA', true]);
  });

  it('division_record decides', () => {
    const rows: Row[] = [
      ['MIA', 'NE', 'H'],
      ['MIA', 'NYJ', 'H'],
      ['BUF', 'NE', 'H'],
      ['NYJ', 'BUF', 'H'],
    ];
    expect(pick(NFL, rows, ['BUF', 'MIA'], [{ rule: 'division_record' }])).toEqual(['MIA', false]);
  });

  it('conference_record decides', () => {
    expect(pick(NFL, cg8, ['BUF', 'MIA'], [conf])).toEqual(['MIA', false]);
  });

  it('common_games decides and is skipped below min_games', () => {
    expect(pick(NFL, cg8, ['BUF', 'MIA'], [{ rule: 'common_games' }])).toEqual(['MIA', false]);
    expect(pick(NFL, cg8, ['BUF', 'MIA'], [{ rule: 'common_games', min_games: 2 }], 0)).toEqual(['BUF', true]);
  });

  it('strength_of_victory decides', () => {
    const rows: Row[] = [
      ['MIA', 'NE', 'H'],
      ['BUF', 'NYJ', 'H'],
      ['NE', 'KC', 'H'],
      ['NE', 'LAC', 'H'],
      ['DEN', 'NYJ', 'H'],
    ];
    expect(pick(NFL, rows, ['BUF', 'MIA'], [sov])).toEqual(['MIA', false]);
  });

  it('strength_of_schedule decides after strength_of_victory ties', () => {
    const rows: Row[] = [
      ['NE', 'MIA', 'H'],
      ['NYJ', 'BUF', 'H'],
      ['NE', 'KC', 'H'],
      ['NE', 'LAC', 'H'],
      ['DEN', 'NYJ', 'H'],
      ['LAC', 'NYJ', 'H'],
    ];
    expect(pick(NFL, rows, ['BUF', 'MIA'], [sov], 0)).toEqual(['BUF', true]);
    expect(pick(NFL, rows, ['BUF', 'MIA'], [sov, { rule: 'strength_of_schedule' }])).toEqual(['MIA', false]);
  });

  it('last_half_conference decides on the last half of conference games', () => {
    const rows: Row[] = [
      ['TOR', 'NYY', 'H'],
      ['BOS', 'TOR', 'H'],
      ['NYY', 'TB', 'H'],
      ['TB', 'BOS', 'H'],
    ];
    expect(pick(MLB, rows, ['BOS', 'NYY'], [lastHalf])).toEqual(['NYY', false]);
  });

  it('last_half_conference widens the window by k games to break a tie', () => {
    const rows: Row[] = [
      ['TOR', 'NYY', 'H'],
      ['BOS', 'TOR', 'H'],
      ['NYY', 'TB', 'H'],
      ['TB', 'BOS', 'H'],
      ['BAL', 'NYY', 'A'],
      ['BAL', 'BOS', 'H'],
      ['DET', 'NYY', 'H'],
      ['DET', 'BOS', 'A'],
      ['NYM', 'NYY', 'H'],
    ];
    expect(pick(MLB, rows, ['BOS', 'NYY'], [conf], 0)).toEqual(['BOS', true]);
    expect(pick(MLB, rows, ['BOS', 'NYY'], [conf, lastHalf])).toEqual(['NYY', false]);
  });

  it('pickBest restarts at the first rule when a tie shrinks', () => {
    const rows: Row[] = [
      ['MIA', 'BUF', 'H'],
      ['BUF', 'NE', 'H'],
      ['NE', 'MIA', 'H'],
      ['NYJ', 'NE', 'H'],
      ['BUF', 'NYJ', 'H'],
      ['MIA', 'NYJ', 'H'],
      ['BUF', 'KC', 'H'],
    ];
    const rules: TiebreakRule[] = [{ rule: 'head_to_head' }, { rule: 'division_record' }, conf];
    expect(pick(NFL, rows, ['BUF', 'MIA', 'NE'], rules)).toEqual(['MIA', false]);
    expect(pick(NFL, rows, ['BUF', 'MIA'], [conf])).toEqual(['BUF', false]);
  });

  it('falls back to a uniform draw over the sorted group', () => {
    const rules = nfl.postseason!.tiebreakers.division;
    const group = ['NE', 'BUF', 'MIA'];
    expect(pick(NFL, [], group, rules, 0)).toEqual(['BUF', true]);
    expect(pick(NFL, [], group, rules, 0.4)).toEqual(['MIA', true]);
    expect(pick(NFL, [], group, rules, 0.8)).toEqual(['NE', true]);
    const ids = group.map((id) => team(NFL, id));
    pickBest(league(NFL, []), ids, rules, { rng: () => 0, usedRandom: false });
    expect(ids).toEqual(group.map((id) => team(NFL, id)));
    const tie = {
      rng: (): number => {
        throw new Error('rng called');
      },
      usedRandom: false,
    };
    expect(pickBest(league(NFL, []), [ids[0]], rules, tie)).toBe(ids[0]);
    expect(tie.usedRandom).toBe(false);
  });

  it('rankTeams repeats pickBest on the remaining teams', () => {
    const rules = nfl.postseason!.tiebreakers.division;
    const tie = { rng: () => 0.5, usedRandom: false };
    const ids = ['NE', 'BUF', 'MIA'].map((id) => team(NFL, id));
    expect(names(NFL, rankTeams(league(NFL, []), ids, rules, tie))).toEqual(['MIA', 'NE', 'BUF']);
    expect(tie.usedRandom).toBe(true);
  });
});

describe('seeding', () => {
  const transitive = (al: Alignment): Row[] => {
    const rows: Row[] = [];
    for (const conf of al.conferences) {
      for (let i = 0; i < conf.teams.length; i++) {
        for (let j = i + 1; j < conf.teams.length; j++) rows.push([al.ids[conf.teams[i]], al.ids[conf.teams[j]], 'H']);
      }
    }
    return rows;
  };
  const throwing = (): number => {
    throw new Error('unexpected random draw');
  };
  const DW: Row[] = [
    ['BUF', 'NYG', 'H'],
    ['BUF', 'DAL', 'H'],
    ['BUF', 'WAS', 'H'],
    ['BAL', 'CHI', 'H'],
    ['BAL', 'DET', 'H'],
    ['BAL', 'GB', 'H'],
    ['HOU', 'ATL', 'H'],
    ['HOU', 'CAR', 'H'],
    ['HOU', 'NO', 'H'],
    ['KC', 'ARI', 'H'],
    ['LAR', 'KC', 'H'],
    ['SEA', 'KC', 'H'],
    ['NE', 'MIA', 'H'],
    ['PIT', 'NE', 'H'],
    ['NE', 'SF', 'H'],
    ['MIA', 'DEN', 'H'],
    ['MIA', 'LV', 'H'],
    ['CIN', 'PIT', 'H'],
    ['CIN', 'CLE', 'H'],
    ['MIN', 'CIN', 'H'],
  ];
  const AFC = ['BAL', 'BUF', 'DEN', 'HOU', 'CIN', 'CLE', 'IND'];
  const NFCT = ['ARI', 'ATL', 'CHI', 'DAL', 'CAR', 'DET', 'GB'];

  it('seeds a transitive NFL league without random draws', () => {
    const out = seedLeague(league(NFL, transitive(NFL)), throwing, createSeeding(NFL));
    expect(out.usedRandom).toBe(false);
    expect(names(NFL, out.seeds[0])).toEqual(AFC);
    expect(names(NFL, out.seeds[1])).toEqual(NFCT);
    expect(names(NFL, out.divisionWinners)).toEqual(['BUF', 'BAL', 'HOU', 'DEN', 'DAL', 'CHI', 'ATL', 'ARI']);
    for (const [id, r] of [
      ['BUF', 1],
      ['MIA', 2],
      ['NE', 3],
      ['NYJ', 4],
    ] as const)
      expect(out.divisionRank[team(NFL, id)]).toBe(r);
    expect(out.seedOf[team(NFL, 'BAL')]).toBe(1);
    expect(out.seedOf[team(NFL, 'IND')]).toBe(7);
    expect(out.seedOf[team(NFL, 'MIA')]).toBe(0);
  });

  it('seeds a transitive MLB league without random draws', () => {
    const out = seedLeague(league(MLB, transitive(MLB)), throwing, createSeeding(MLB));
    expect(out.usedRandom).toBe(false);
    expect(names(MLB, out.seeds[0])).toEqual(['ATH', 'BAL', 'CLE', 'BOS', 'CWS', 'DET']);
    expect(names(MLB, out.seeds[1])).toEqual(['ATL', 'AZ', 'CHC', 'CIN', 'COL', 'LAD']);
    expect(names(MLB, out.divisionWinners)).toEqual(['BAL', 'CLE', 'ATH', 'ATL', 'CHC', 'AZ']);
  });

  it('seeds division winners before better wild cards', () => {
    const s = league(NFL, DW);
    const out = seedLeague(s, () => 0, createSeeding(NFL));
    expect(names(NFL, out.seeds[0])).toEqual(['BAL', 'BUF', 'HOU', 'KC', 'CIN', 'NE', 'MIA']);
    expect(out.seedOf[team(NFL, 'KC')]).toBe(4);
    expect(out.seedOf[team(NFL, 'MIA')]).toBe(7);
    expect(out.seedOf[team(NFL, 'PIT')]).toBe(0);
    expect(winPercentage(s, team(NFL, 'MIA'))).toBeGreaterThan(winPercentage(s, team(NFL, 'KC')));
    expect(out.divisionWinners[3]).toBe(team(NFL, 'KC'));
    expect(out.divisionRank[team(NFL, 'NE')]).toBe(2);
    expect(out.divisionRank[team(NFL, 'MIA')]).toBe(3);
    expect(out.usedRandom).toBe(true);
  });

  it('keeps one team per division in a wild-card tie', () => {
    const s = league(NFL, DW);
    const run = (one: boolean) =>
      names(
        NFL,
        seedLeague(s, () => 0, createSeeding(NFL), {
          one_per_division: one,
          division: [{ rule: 'head_to_head' }],
          conference: [{ rule: 'conference_record' }],
        }).seeds[0],
      );
    expect(run(true)).toEqual(['BAL', 'BUF', 'HOU', 'KC', 'CIN', 'NE', 'MIA']);
    expect(run(false)).toEqual(['BAL', 'BUF', 'HOU', 'KC', 'CIN', 'MIA', 'NE']);
  });

  it('reports random draws and resets the flag', () => {
    const out = createSeeding(NFL);
    seedLeague(league(NFL, []), () => 0, out);
    expect(out.usedRandom).toBe(true);
    expect(names(NFL, out.seeds[0])).toEqual(AFC);
    expect(names(NFL, out.seeds[1])).toEqual(NFCT);
    const again = seedLeague(league(NFL, transitive(NFL)), throwing, out);
    expect(again.usedRandom).toBe(false);
    expect(again).toBe(out);
  });
});
