import { z } from 'zod';
import { configSchema, displaySchema, gameFileSchema, gameSchema, seedSchema, validateGames } from './contracts.ts';
import type { Posterior } from './model.ts';
import type { PostseasonState } from './postseason.ts';
import type { PublicState } from './service.ts';
import type { Store } from './storage.ts';
import { APP_VERSION } from './version.ts';

/** Storage key of a league's saved model snapshot. */
export const snapshotKey = (league: string) => `game-results-prediction:${league}:model-v1`;
export type ModelSnapshot = {
  app_version: string;
  file: z.infer<typeof gameFileSchema>;
  model: Posterior;
  state: PublicState;
  seed_sha256?: string | undefined;
};

const finite = z.number().finite();
const probability = finite.min(0).max(1);
const prediction = z.object({
  home_win: probability,
  away_win: probability,
  tie: probability,
  home_probability_interval: z.tuple([probability, probability]),
});
const round = z.number().int().nonnegative();
const count = z.number().int().nonnegative();
const teamStatus = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('out') }),
  z.object({ kind: z.literal('pending') }),
  z.object({ kind: z.literal('qualified') }),
  z.object({ kind: z.literal('bye') }),
  z.object({ kind: z.literal('champion') }),
  z.object({ kind: z.literal('series'), round, wins: count, losses: count, opponent: z.string() }),
  z.object({ kind: z.literal('advanced'), round }),
  z.object({ kind: z.literal('eliminated'), round }),
]);
export const postseasonStateSchema: z.ZodType<PostseasonState> = z.discriminatedUnion('status', [
  z.object({
    status: z.literal('ready'),
    mode: z.enum(['regular', 'postseason']),
    simulations: z.number().int().positive(),
    playoff_spots: z.number().int().positive(),
    byes: count,
    rounds: z.array(z.object({ name: z.string(), short: z.string(), games: z.number().int().positive() })),
    conferences: z.array(
      z.object({
        id: z.string(),
        name: z.string(),
        teams: z.array(
          z.object({
            id: z.string(),
            division: z.string(),
            division_label: z.string(),
            record: z.object({ wins: count, losses: count, ties: count }),
            mean_seed: finite.nullable(),
            playoffs: probability,
            win_division: probability,
            bye: probability,
            reach: z.array(probability),
            title: probability,
            status: teamStatus.nullable(),
          }),
        ),
      }),
    ),
    notes: z.array(z.string()),
  }),
  z.object({ status: z.literal('error'), error: z.string() }),
]);
const snapshotSchema = z.object({
  app_version: z.literal(APP_VERSION),
  seed_sha256: z
    .string()
    .regex(/^[a-f0-9]{64}$/)
    .optional(),
  file: gameFileSchema,
  model: z.object({
    ids: z.array(z.string()).min(2),
    means: z.array(finite),
    covariance: z.array(z.array(finite)),
    seed: seedSchema,
    config: configSchema,
    games_used: z.number().int().nonnegative(),
    iterations: z.number().int().nonnegative(),
  }),
  state: z.object({
    status: z.literal('ready'),
    error: z.null(),
    warning: z.string().nullable(),
    league: z.string(),
    season: z.number().int(),
    refreshed_at: z.iso.datetime({ offset: true }),
    checked_at: z.iso.datetime({ offset: true }).nullable(),
    cached: z.boolean(),
    training_games: z.number().int().nonnegative(),
    historical_games: z.number().int().nonnegative(),
    held_results: z.number().int().nonnegative(),
    team_history: configSchema.shape.teams,
    source: z.string(),
    result_policy: z.string(),
    result_policy_summary: z.string(),
    history_start: z.number().int(),
    display: displaySchema,
    ties_allowed_in: configSchema.shape.ties_allowed_in,
    teams: z.array(
      z.object({
        id: z.string(),
        name: z.string(),
        location: z.string(),
        abbreviation: z.string(),
        initial_elo: finite,
        rating: finite,
        sd: finite.nonnegative(),
      }),
    ),
    games: z.array(
      gameSchema.extend({ status: z.enum(['completed', 'scheduled', 'awaiting_result']), prediction: prediction.nullable() }),
    ),
    postseason: postseasonStateSchema.nullable(),
  }),
});

export function readSnapshot(store: Store | null, league: string): ModelSnapshot | null {
  try {
    const raw = store?.getItem(snapshotKey(league));
    if (!raw) return null;
    const snapshot = snapshotSchema.parse(JSON.parse(raw));
    const { model, file, state } = snapshot;
    const size = model.ids.length;
    if (
      model.config.id !== league ||
      new Set(model.ids).size !== size ||
      model.means.length !== size ||
      model.covariance.length !== size ||
      model.covariance.some((row, i) => row.length !== size || row[i] < 0) ||
      model.ids.some((id) => !model.config.teams.some((t) => t.id === id)) ||
      state.teams.length !== size ||
      state.teams.some((t) => !model.ids.includes(t.id)) ||
      file.league !== model.config.id ||
      file.from_season !== state.season ||
      file.through_season !== state.season ||
      model.seed.target_season !== state.season ||
      file.source_url !== model.config.source.url ||
      state.games.length !== file.games.length ||
      (state.postseason === null) !== (model.config.postseason === undefined)
    )
      return null;
    file.games = validateGames(file.games, model.config);
    if (!file.games.length || file.games.some((g) => g.season !== state.season)) return null;
    return snapshot;
  } catch {
    return null;
  }
}

export function writeSnapshot(store: Store | null, snapshot: ModelSnapshot): void {
  try {
    store?.setItem(snapshotKey(snapshot.file.league), JSON.stringify(snapshot));
  } catch {
    // Storage failure must not discard a successfully fitted model.
  }
}
