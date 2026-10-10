import { configSchema, gameFileSchema, seedSchema, validateGames, type GameFile, type LeagueConfig } from './contracts.ts';

/** SHA-256 over the exact bytes served, so hashes match the ones the Rust programs wrote. */
export async function digest(value: string | ArrayBuffer | Uint8Array): Promise<string> {
  const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value;
  const hash = await crypto.subtle.digest('SHA-256', bytes as BufferSource);
  return Array.from(new Uint8Array(hash), (b) => b.toString(16).padStart(2, '0')).join('');
}

export class NotFoundError extends Error {}

/** Published data files are static assets; a 404 means the generator has not run. */
async function fetchBytes(url: string, signal?: AbortSignal, cache: RequestCache = 'no-store'): Promise<ArrayBuffer> {
  const response = await fetch(url, { cache, ...(signal ? { signal } : {}) });
  if (response.status === 404) throw new NotFoundError(`Not published: ${url}`);
  if (!response.ok) throw new Error(`Could not read ${url}: HTTP ${response.status}`);
  return await response.arrayBuffer();
}
const decode = (bytes: ArrayBuffer) => new TextDecoder().decode(bytes);

export async function readConfig(url: string, signal?: AbortSignal) {
  const bytes = await fetchBytes(url, signal);
  return { config: configSchema.parse(JSON.parse(decode(bytes))), hash: await digest(bytes) };
}

/** Reads the seed at `url` and checks it against the configuration and the published SHA-256 of its history,
 * `historyHashUrl`.
 */
export async function readSeed(
  url: string,
  historyHashUrl: string,
  config: LeagueConfig,
  configHash: string,
  season: number,
  signal?: AbortSignal,
) {
  return (await readSeedWithHash(url, historyHashUrl, config, configHash, season, signal)).seed;
}

export async function readSeedWithHash(
  url: string,
  historyHashUrl: string,
  config: LeagueConfig,
  configHash: string,
  season: number,
  signal?: AbortSignal,
) {
  // Revalidate cached responses so unchanged published files need not transfer their bodies again.
  const bytes = await fetchBytes(url, signal, 'no-cache');
  const seed = seedSchema.parse(JSON.parse(decode(bytes)));
  if (seed.league !== config.id || seed.target_season !== season || seed.through_season !== season - 1)
    throw new Error('Elo seed is for the wrong league or season; run the two Rust programs');
  if (seed.config_sha256 !== configHash)
    throw new Error('Configuration changed since Elo was calculated; rerun generate-preseason-seed');
  if (decode(await fetchBytes(historyHashUrl, signal, 'no-cache')).trim() !== seed.history_sha256)
    throw new Error('History changed since Elo was calculated; rerun generate-preseason-seed');
  return { seed, hash: await digest(bytes) };
}

/** Synchronous key-value store: the page's small-key store, or the refresh worker's in-memory copy of a
 * league's persisted entries, which the page saves once it receives the complete update.
 */
export interface Store {
  keys(): string[];
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}
export const memoryStore = (): Store => {
  const map = new Map<string, string>();
  return {
    keys: () => [...map.keys()],
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => void map.set(k, v),
    removeItem: (k) => void map.delete(k),
  };
};

const currentCachePrefix = (league: string) => `game-results-prediction:${league}:current-`;
/** Storage key of a league's current-season game cache. */
export const currentCacheKey = (league: string, season: number) => `${currentCachePrefix(league)}${season}`;
/** Whether `key` is a current-season game cache key of `league`. */
export function isCurrentCacheKey(key: string, league: string): boolean {
  const prefix = currentCachePrefix(league);
  return key.startsWith(prefix) && /^\d+$/.test(key.slice(prefix.length));
}

export function readCache(store: Store | null, key: string, config: LeagueConfig, season: number): GameFile | null {
  const raw = store?.getItem(key);
  if (raw == null) return null;
  const file = gameFileSchema.parse(JSON.parse(raw));
  if (
    file.league !== config.id ||
    file.from_season !== season ||
    file.through_season !== season ||
    file.source_url !== config.source.url
  )
    throw new Error('Cache belongs to another source or season');
  if (JSON.stringify(file.teams) !== JSON.stringify(config.teams))
    throw new Error('Cache team identity history does not match the configuration');
  file.games = validateGames(file.games, config);
  if (!file.games.length || file.games.some((g) => g.season !== season)) throw new Error('Invalid current-season cache');
  return file;
}

export function writeCache(store: Store | null, key: string, value: GameFile): void {
  try {
    store?.setItem(key, JSON.stringify(value));
  } catch {
    // A full or unavailable quota only costs the offline fallback, never the prediction.
  }
}
