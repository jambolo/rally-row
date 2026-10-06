/** Public surface of the browser prediction app: the league contracts, the provider
 * adapters, the Bayesian model, and the service that assembles them.
 */
export {
  configSchema,
  displaySchema,
  windowsSchema,
  gameSchema,
  gameFileSchema,
  seedSchema,
  teamSchema,
  validateGames,
  currentSeason,
  teamIdentity,
  type EloSeed,
  type Game,
  type GameFile,
  type LeagueConfig,
} from './contracts.ts';
export { adapterFor, isSourceKind, type SourceAdapter } from './adapters/index.ts';
export {
  fitPosterior,
  formatTwoWayMoneyline,
  outcomeProbabilities,
  parseMoneyline,
  predict,
  teamEstimates,
  twoWayExpectedValue,
  twoWayMoneylines,
  type Posterior,
  type Prediction,
  type Probabilities,
} from './model.ts';
export { download, parseSource, usableResults } from './provider.ts';
export { PredictionService, message, type GameView, type PublicState } from './service.ts';
export { indexedDbPersistence, type Persistence } from './persistence.ts';
export { rememberedLeagueKey, removeLegacyEntries, smallStore } from './small-store.ts';
export { postseasonLabel, scheduleKey, scheduleOptions, type ScheduleUnit } from './schedule.ts';
export {
  currentCacheKey,
  digest,
  memoryStore,
  readCache,
  readConfig,
  readSeed,
  writeCache,
  NotFoundError,
  type Store,
} from './storage.ts';
