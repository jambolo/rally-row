import { adapterFor, type SourceAdapter } from './adapters/index.ts';
import { startTimeUtc } from './time.ts';
import { gameSchema, type EloSeed, type Game, type GameFile, type LeagueConfig } from './contracts.ts';
import { fitPosterior, predict, teamEstimates, type Posterior, type Prediction } from './model.ts';
import { simulatePostseason, type PostseasonState } from './postseason.ts';
import { download, parseSource, usableResults } from './provider.ts';
import { currentCacheKey, readCache, readSeedWithHash, writeCache, type Store } from './storage.ts';
import { readSnapshot, writeSnapshot, type ModelSnapshot } from './snapshot.ts';
import { APP_VERSION } from './version.ts';

export type RefreshPhase = 'checking' | 'building' | 'rebuilding';

export type GameView = Game & {
  status: 'completed' | 'scheduled' | 'awaiting_result';
  prediction: Prediction | null;
};
export type PublicState = {
  status: 'loading' | 'ready' | 'error';
  error: string | null;
  warning: string | null;
  league: string;
  season: number;
  refreshed_at: string | null;
  checked_at: string | null;
  cached: boolean;
  training_games: number;
  historical_games: number;
  held_results: number;
  team_history: LeagueConfig['teams'];
  source: string;
  result_policy: string;
  result_policy_summary: string;
  history_start: number;
  display: LeagueConfig['display'];
  ties_allowed_in: LeagueConfig['ties_allowed_in'];
  teams: ReturnType<typeof teamEstimates>;
  games: GameView[];
  postseason: PostseasonState | null;
};

export class PredictionService {
  readonly config: LeagueConfig;
  readonly season: number;
  private readonly adapter: SourceAdapter;
  private model: Posterior | null = null;
  private seed: EloSeed | null = null;
  private state: PublicState;
  constructor(
    private options: {
      config: LeagueConfig;
      configHash: string;
      /** Base URL the published `data/<league>/` files are served from. */
      dataBase: string;
      season: number;
      /** Snapshot and current-season cache store; omitted means no persistence. */
      store?: Store | null;
      fetchSource?: typeof download;
      now?: () => Date;
      onProgress?: (phase: RefreshPhase) => void;
      /** Postseason simulations per rebuild; tests lower it to keep runtime low. */
      postseasonSimulations?: number;
    },
  ) {
    this.config = options.config;
    this.season = options.season;
    this.adapter = adapterFor(this.config);
    this.state = {
      ...this.configurationMetadata(),
      status: 'loading',
      error: null,
      warning: null,
      season: this.season,
      refreshed_at: null,
      checked_at: null,
      cached: false,
      training_games: 0,
      historical_games: 0,
      held_results: 0,
      teams: [],
      games: [],
      postseason: null,
    };
  }
  private configurationMetadata() {
    return {
      league: this.config.name,
      // The season-resolved download link; for single-document sources this is the configured URL.
      source: this.adapter.seasonUrls(this.config, this.season)[0],
      team_history: this.config.teams,
      history_start: this.config.history_start,
      result_policy: this.adapter.resultPolicy.detail,
      result_policy_summary: this.adapter.resultPolicy.summary,
      display: this.config.display,
      ties_allowed_in: this.config.ties_allowed_in,
    };
  }
  /** Odds from the current-season fit; a failed simulation never blocks predictions. */
  private postseasonOdds(): PostseasonState | null {
    if (this.config.postseason === undefined) return null;
    const simulations = this.options.postseasonSimulations;
    try {
      return simulatePostseason(this.model!, this.state.games, simulations === undefined ? {} : { simulations });
    } catch (e) {
      return { status: 'error', error: message(e) };
    }
  }

  getState(): PublicState {
    return this.state;
  }
  getModel(): Posterior | null {
    return this.model;
  }
  predict(home: string, away: string, neutral: boolean, phase: Game['phase']) {
    if (!this.model) throw new Error('Predictions are not ready');
    return predict(this.model, home, away, neutral, phase);
  }
  async initialize(): Promise<void> {
    const dir = `${this.options.dataBase.replace(/\/$/, '')}/${this.config.id}`;
    const store = this.options.store ?? null;
    const cacheKey = currentCacheKey(this.config.id, this.season);
    const now = (this.options.now ?? (() => new Date()))();
    const saved = readSnapshot(store, this.config.id);
    const snapshot = saved?.file.league === this.config.id && saved.file.from_season === this.season ? saved : null;
    if (snapshot) {
      this.model = snapshot.model;
      this.seed = snapshot.model.seed;
      this.state = { ...snapshot.state, cached: true };
    }
    try {
      this.options.onProgress?.('checking');
      let cache: GameFile | null = snapshot?.file ?? null,
        cacheProblem = '';
      try {
        cache ??= readCache(store, cacheKey, this.config, this.season);
      } catch (e) {
        cacheProblem = `Existing cache could not be read: ${message(e)}. `;
      }
      let file: GameFile;
      try {
        const fetchSource = this.options.fetchSource ?? download;
        const texts: string[] = [];
        for (const url of this.adapter.seasonUrls(this.config, this.season)) texts.push(await fetchSource(url));
        const games = gameSchema.array().parse(parseSource(texts, this.config).filter((g) => g.season === this.season));
        if (!games.length) throw new Error(`The source has no games for season ${this.season} yet`);
        if (cache) {
          const incoming = new Map(games.map((g) => [g.id, g]));
          for (const old of usableResults(cache.games, this.config, now)) {
            if (!incoming.get(old.id)?.result)
              throw new Error(`Refresh lost a previously completed game (${old.id}); cached data retained`);
          }
        }
        file = {
          schema_version: 2,
          league: this.config.id,
          fetched_at: now.toISOString(),
          source_url: this.config.source.url,
          from_season: this.season,
          through_season: this.season,
          teams: this.config.teams,
          games,
        };
        // Preserve validated games until the replacement model is complete, including during app upgrades.
        if (!cache) writeCache(store, cacheKey, file);
        if (cacheProblem) this.state.warning = `${cacheProblem}Replaced it with a valid download.`;
      } catch (e) {
        if (snapshot) throw e;
        if (!cache) throw new Error(`${cacheProblem}${message(e)}. No valid current-season cache is available.`, { cause: e });
        file = cache;
        this.state.cached = true;
        this.state.warning = `Refresh failed. Using cached data from ${cache.fetched_at}. ${message(e)}`;
      }
      const results = usableResults(file.games, this.config, now);
      const unchanged =
        snapshot !== null &&
        snapshot.model.seed.config_sha256 === this.options.configHash &&
        JSON.stringify(file.games) === JSON.stringify(snapshot.file.games) &&
        results.length === snapshot.state.training_games;
      if (!unchanged) this.options.onProgress?.(snapshot ? 'rebuilding' : 'building');
      let published: Awaited<ReturnType<typeof readSeedWithHash>>;
      try {
        published = await readSeedWithHash(
          `${dir}/elo-${this.season}.json`,
          `${dir}/history.sha256`,
          this.config,
          this.options.configHash,
          this.season,
        );
      } catch (e) {
        throw new Error(
          `Current-season data is available, but the published initial ratings could not be loaded. ${message(e)}. Run import-history followed by generate-preseason-seed, then rebuild the site.`,
          { cause: e },
        );
      }
      if (unchanged && snapshot.seed_sha256 === published.hash) {
        this.state = {
          ...snapshot.state,
          ...this.configurationMetadata(),
          cached: false,
          warning: null,
          checked_at: now.toISOString(),
        };
        writeSnapshot(store, { ...snapshot, state: this.state });
        return;
      }
      if (unchanged) this.options.onProgress?.('rebuilding');
      this.seed = published.seed;
      this.state.refreshed_at = file.fetched_at;
      this.state.checked_at = this.state.cached && !snapshot ? null : now.toISOString();
      const completed = new Set(results.map((g) => g.id));
      this.model = fitPosterior(this.seed, results, this.config);
      this.state.teams = teamEstimates(this.model);
      this.state.training_games = results.length;
      this.state.historical_games = this.seed.completed_games;
      this.state.held_results = file.games.filter((g) => g.result !== null && !completed.has(g.id)).length;
      const pregameModels = new Map<string, Posterior>();
      this.state.games = file.games.map((g) => {
        const startTime = Date.parse(g.start_time_utc ?? startTimeUtc(g));
        const status = completed.has(g.id) ? 'completed' : startTime <= now.getTime() ? 'awaiting_result' : 'scheduled';
        let model = this.model!;
        if (status === 'completed') {
          // Without final timestamps, exclude same-day outcomes and reuse each day's model.
          const day = this.adapter.pregameDay(g);
          let pregame = pregameModels.get(day);
          if (!pregame) {
            const priorResults = results.filter((r) => this.adapter.pregameDay(r) < day);
            pregame = fitPosterior(this.seed!, priorResults, this.config);
            pregameModels.set(day, pregame);
          }
          model = pregame;
        }
        return {
          ...g,
          result: completed.has(g.id) ? g.result : null,
          status,
          prediction: predict(model, g.home_team, g.away_team, g.neutral, g.phase),
        };
      });
      this.state.postseason = this.postseasonOdds();
      this.state = { ...this.state, ...this.configurationMetadata(), status: 'ready' };
      // Commit the source and fitted output only after the complete build succeeds.
      if (snapshot) this.state.cached = false;
      const builtSnapshot: ModelSnapshot = {
        app_version: APP_VERSION,
        file,
        model: this.model,
        state: { ...this.state, warning: null },
        seed_sha256: published.hash,
      };
      writeSnapshot(store, builtSnapshot);
      writeCache(store, cacheKey, file);
    } catch (e) {
      if (snapshot) {
        this.model = snapshot.model;
        this.seed = snapshot.model.seed;
        this.state = {
          ...snapshot.state,
          cached: true,
          warning: `Update failed. Using cached data from ${snapshot.file.fetched_at}. ${message(e)}`,
        };
        return;
      }
      this.model = null;
      this.seed = null;
      this.state.status = 'error';
      this.state.error = message(e);
    }
  }
}
export const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
