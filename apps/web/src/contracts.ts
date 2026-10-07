import { z } from 'zod';
import { DateTime } from 'luxon';
import { adapterFor, isSourceKind } from './adapters/index.ts';
import { startTimeUtc } from './time.ts';

const finite = z.number().finite();
const phase = z.enum(['regular', 'postseason']);
export const teamSchema = z.object({
  id: z.string().min(1),
  name: z.string().min(1),
  location: z.string().min(1),
  eras: z
    .array(
      z.object({
        from_season: z.number().int(),
        through_season: z.number().int().nullable(),
        name: z.string().min(1),
        location: z.string().min(1),
        /** Display abbreviation; when absent, the first source id serves. */
        abbreviation: z.string().min(1).optional(),
        division: z.string().min(1).optional(),
        source_ids: z.array(z.string().min(1)).min(1),
      }),
    )
    .min(1),
});
const eloSchema = z.object({
  initial: finite,
  scale: finite.positive(),
  k: finite.positive(),
  home_advantage: finite,
  offseason_regression: finite.min(0).max(1),
});
const nonEmpty = z.string().min(1);
export const displaySchema = z.object({
  start_time_label: nonEmpty,
  round_name: nonEmpty.nullable(),
  schedule_filter: z.object({
    unit: z.enum(['round', 'date']),
    label: nonEmpty,
    all_label: nonEmpty,
  }),
  postseason_label: nonEmpty,
  postseason_round_labels: z.record(nonEmpty, nonEmpty),
  postseason_tie_note: nonEmpty,
});
const daysInMonth = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
/** Day of year (1-365) in a non-leap year; null unless `value` is a valid "MM-DD". */
export function monthDayOrdinal(value: string): number | null {
  const m = /^(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  const month = Number(m[1]);
  const day = Number(m[2]);
  if (month < 1 || month > 12 || day < 1 || day > daysInMonth[month - 1]) return null;
  return daysInMonth.slice(0, month - 1).reduce((sum, days) => sum + days, 0) + day;
}
const monthDay = z.string().refine((s) => monthDayOrdinal(s) !== null, 'Invalid month-day');
const monthDayWindow = z.object({ start: monthDay, end: monthDay });
export const windowsSchema = z.object({ season: monthDayWindow, postseason: monthDayWindow }).superRefine((w, ctx) => {
  const ss = monthDayOrdinal(w.season.start);
  const se = monthDayOrdinal(w.season.end);
  const ps = monthDayOrdinal(w.postseason.start);
  const pe = monthDayOrdinal(w.postseason.end);
  if (ss === null || se === null || ps === null || pe === null) return;
  if (ss === se) {
    ctx.addIssue({ code: 'custom', message: 'Season window start and end must be different' });
    return;
  }
  // Days after season.start, wrapping at the year boundary.
  const span = (to: number) => (to + 365 - ss) % 365;
  if (!(span(ps) <= span(pe) && span(pe) <= span(se)))
    ctx.addIssue({ code: 'custom', message: 'Postseason window must lie within the season window' });
});
const tiebreakRule = z.object({
  rule: z.enum([
    'head_to_head',
    'head_to_head_sweep',
    'division_record',
    'conference_record',
    'common_games',
    'strength_of_victory',
    'strength_of_schedule',
    'last_half_conference',
  ]),
  min_games: z.number().int().positive('min_games must be positive').optional(),
});
const postseasonSchema = z.object({
  conferences: z
    .array(
      z.object({
        id: z.string(),
        name: z.string(),
        divisions: z.array(z.object({ id: z.string(), name: z.string(), short: z.string().optional() })).min(1),
      }),
    )
    .min(1),
  teams_per_conference: z.number().int().min(2, 'Invalid playoff field size'),
  division_winners_first: z.boolean(),
  reseed: z.boolean(),
  rounds: z
    .array(
      z.object({
        name: z.string(),
        short: z.string(),
        round_label: z.string(),
        pattern: z.string().regex(/^(?:[HAN]{2})*[HAN]$/, 'Invalid series pattern'),
        home: z.enum(['higher_seed', 'better_record']),
      }),
    )
    .min(1),
  tiebreakers: z.object({ one_per_division: z.boolean(), division: z.array(tiebreakRule), conference: z.array(tiebreakRule) }),
});
export const configSchema = z
  .object({
    schema_version: z.literal(2),
    id: z.string().regex(/^[a-zA-Z0-9-]+$/),
    name: z.string().min(1),
    history_start: z.number().int().min(1900).max(2200),
    season_rollover_month: z.number().int().min(1).max(12),
    source: z.object({
      kind: z.string().refine(isSourceKind, 'Unknown source adapter'),
      url: z.url().refine((s) => s.startsWith('https://')),
    }),
    teams: z.array(teamSchema).min(2),
    aliases: z.record(z.string(), z.string()),
    ties_allowed_in: z.array(phase),
    display: displaySchema,
    windows: windowsSchema,
    postseason: postseasonSchema.optional(),
    elo: eloSchema,
    bayesian: z.object({
      prior_sd_elo: finite.positive(),
      tie_prior_games: finite.positive(),
      tie_prior_rate: finite.gt(0).lt(1),
    }),
  })
  .superRefine((c, ctx) => {
    if (isSourceKind(c.source.kind)) {
      const problem = adapterFor(c).validateConfig?.(c);
      if (problem) ctx.addIssue({ code: 'custom', message: problem });
    }
    const ids = new Set(c.teams.map((t) => t.id));
    if (ids.size !== c.teams.length) ctx.addIssue({ code: 'custom', message: 'Duplicate team ids' });
    for (const [from, to] of Object.entries(c.aliases)) {
      if (!ids.has(to) || (ids.has(from) && from !== to)) ctx.addIssue({ code: 'custom', message: 'Invalid team alias' });
    }
    for (const team of c.teams) {
      const fail = (message: string) => ctx.addIssue({ code: 'custom', message: `${team.id}: ${message}` });
      if (team.eras[0].from_season > c.history_start) fail('Identity history starts too late');
      team.eras.forEach((era, i) => {
        if (era.through_season !== null && era.through_season < era.from_season) fail('Invalid identity range');
        if (era.source_ids.some((id) => (c.aliases[id] ?? id) !== team.id))
          fail('Historical abbreviation points to another franchise');
        const next = team.eras[i + 1];
        if (next) {
          if (era.through_season !== next.from_season - 1) fail('Identity eras overlap or have a gap');
        } else if (era.through_season !== null || era.name !== team.name || era.location !== team.location)
          fail('Latest identity does not match current team metadata');
      });
    }
    for (const message of postseasonProblems(c)) ctx.addIssue({ code: 'custom', message });
  });
export type LeagueConfig = z.infer<typeof configSchema>;
export type PostseasonFormat = NonNullable<LeagueConfig['postseason']>;
/** Mirrors the Rust validation: returns at most one message, the first failing check. */
export function postseasonProblems(c: LeagueConfig): string[] {
  const p = c.postseason;
  const withId = (id: string, message: string) => [`${id}: ${message}`];
  if (!p) {
    for (const team of c.teams)
      if (team.eras.some((e) => e.division !== undefined)) return withId(team.id, 'Team division needs a postseason format');
    return [];
  }
  if (c.ties_allowed_in.includes('postseason')) return ['Postseason format requires postseason ties to be disallowed'];
  const n = p.conferences.length;
  if ((n & (n - 1)) !== 0) return ['Conference count must be a power of two'];
  const seen = new Set<string>();
  for (const conf of p.conferences) {
    for (const id of [conf.id, ...conf.divisions.map((d) => d.id)]) {
      if (seen.has(id)) return ['Duplicate conference or division id'];
      seen.add(id);
    }
  }
  if (p.teams_per_conference < 2) return ['Invalid playoff field size'];
  if (p.division_winners_first && p.conferences.some((x) => x.divisions.length > p.teams_per_conference))
    return ['More divisions than playoff spots'];
  let bracket = 0;
  while (2 ** bracket < p.teams_per_conference) bracket++;
  if (p.rounds.length !== bracket + Math.log2(n)) return ['Round count does not match the bracket'];
  if (new Set(p.rounds.map((r) => r.round_label)).size !== p.rounds.length) return ['Duplicate round label'];
  if (p.rounds.some((r) => !/^(?:[HAN]{2})*[HAN]$/.test(r.pattern))) return ['Invalid series pattern'];
  if (p.rounds.some((r, i) => i >= bracket && r.home === 'higher_seed' && /[HA]/.test(r.pattern)))
    return ['Rounds between conferences cannot give home advantage by seed'];
  const lists = [p.tiebreakers.division, p.tiebreakers.conference];
  const rules = lists.flat();
  if (rules.some((r) => r.min_games !== undefined && r.rule !== 'common_games')) return ['min_games applies only to common_games'];
  if (rules.some((r) => r.min_games === 0)) return ['min_games must be positive'];
  if (lists.some((l) => new Set(l.map((r) => r.rule)).size !== l.length)) return ['Duplicate tiebreaker'];
  const divisions = new Set(p.conferences.flatMap((x) => x.divisions.map((d) => d.id)));
  for (const team of c.teams)
    for (const era of team.eras)
      if (era.division !== undefined && !divisions.has(era.division)) return withId(team.id, 'Unknown division');
  for (const team of c.teams)
    if (team.eras[team.eras.length - 1].division === undefined) return withId(team.id, 'Current division missing');
  const current = c.teams.map((t) => t.eras[t.eras.length - 1].division);
  for (const conf of p.conferences)
    for (const d of conf.divisions) if (!current.includes(d.id)) return withId(d.id, 'Division has no current teams');
  for (const conf of p.conferences) {
    const ids = new Set(conf.divisions.map((d) => d.id));
    if (current.filter((d) => d !== undefined && ids.has(d)).length < p.teams_per_conference)
      return withId(conf.id, 'Conference has fewer teams than playoff spots');
  }
  return [];
}
export const gameSchema = z.object({
  id: z.string().min(1),
  league: z.string(),
  season: z.number().int(),
  date: z
    .string()
    .regex(/^\d{4}-\d{2}-\d{2}$/)
    .refine((s) => DateTime.fromISO(s).isValid),
  time: z
    .string()
    .regex(/^(?:(?:[01]\d|2[0-3]):[0-5]\d|\s*)$/)
    .nullable()
    .default(null),
  start_time_utc: z.iso.datetime().optional(),
  timezone: z.string().refine((s) => DateTime.now().setZone(s).isValid),
  phase,
  round_label: z.string(),
  round: z.number().int().positive(),
  home_team: z.string(),
  away_team: z.string(),
  neutral: z.boolean(),
  home_source_id: z.string().min(1),
  away_source_id: z.string().min(1),
  result: z.enum(['home_win', 'away_win', 'tie']).nullable(),
});
export type Game = z.infer<typeof gameSchema>;
export const gameFileSchema = z.object({
  schema_version: z.literal(2),
  league: z.string(),
  fetched_at: z.iso.datetime({ offset: true }),
  source_url: z.string(),
  from_season: z.number().int(),
  through_season: z.number().int(),
  teams: z.array(teamSchema),
  games: z.array(gameSchema),
});
export type GameFile = z.infer<typeof gameFileSchema>;
export const seedSchema = z.object({
  schema_version: z.literal(1),
  league: z.string(),
  target_season: z.number().int(),
  through_season: z.number().int(),
  generated_at: z.iso.datetime({ offset: true }),
  history_sha256: z.string().length(64),
  config_sha256: z.string().length(64),
  settings: eloSchema,
  completed_games: z.number().int().nonnegative(),
  tied_games: z.number().int().nonnegative(),
  tie_weight: finite.positive(),
  ratings: z.array(
    z.object({
      team: z.string(),
      elo: finite,
      games: z.number().int().nonnegative(),
    }),
  ),
});
export type EloSeed = z.infer<typeof seedSchema>;

/** Dates, times and ids are ASCII, so code-unit order matches the byte order used elsewhere. */
const order = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

export function validateGames(input: unknown, config: LeagueConfig): Game[] {
  const games = z.array(gameSchema).parse(input);
  const { strictSourceIds, sourceStartTimes } = adapterFor(config);
  const teams = new Set(config.teams.map((t) => t.id));
  const seen = new Set<string>();
  for (const g of games) {
    if (seen.has(g.id)) throw new Error(`Duplicate game id: ${g.id}`);
    seen.add(g.id);
    if (g.league !== config.id || !teams.has(g.home_team) || !teams.has(g.away_team) || g.home_team === g.away_team)
      throw new Error(`Invalid league or teams for ${g.id}`);
    for (const [id, source] of [
      [g.home_team, g.home_source_id],
      [g.away_team, g.away_source_id],
    ]) {
      if ((config.aliases[source] ?? source) !== id) throw new Error(`Source id does not match franchise: ${g.id}`);
      const era = teamIdentity(config, id, g.season);
      if (strictSourceIds && !era.source_ids.includes(source))
        throw new Error(`Source abbreviation invalid for season ${g.season}: ${g.id}`);
    }
    if (!sourceStartTimes) g.start_time_utc = startTimeUtc(g);
    else if (!g.start_time_utc) throw new Error(`Missing start time for ${g.id}`);
    if (g.result === 'tie' && !config.ties_allowed_in.includes(g.phase)) throw new Error(`Tie prohibited for ${g.id}`);
  }
  return games.sort((a, b) => a.season - b.season || order(a.start_time_utc!, b.start_time_utc!) || order(a.id, b.id));
}
export function currentSeason(config: LeagueConfig, now = new Date()): number {
  return now.getUTCFullYear() - Number(now.getUTCMonth() + 1 < config.season_rollover_month);
}
export function teamIdentity(config: LeagueConfig, id: string, season: number) {
  const era = config.teams
    .find((t) => t.id === id)
    ?.eras.find((e) => season >= e.from_season && (e.through_season === null || season <= e.through_season));
  if (!era) throw new Error(`No historical identity for ${id} in ${season}`);
  return era;
}

/** Consecutive eras with the same name and location, merged; division-only changes are not identity changes. */
export function identityRuns(team: LeagueConfig['teams'][number]) {
  const runs: { from_season: number; through_season: number | null; name: string; location: string }[] = [];
  for (const era of team.eras) {
    const last = runs.at(-1);
    if (last && last.name === era.name && last.location === era.location) last.through_season = era.through_season;
    else runs.push({ from_season: era.from_season, through_season: era.through_season, name: era.name, location: era.location });
  }
  return runs;
}
