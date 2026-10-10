import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { adapterFor } from '../src/adapters/index.ts';
import { configSchema, currentSeason, seedSchema, type LeagueConfig } from '../src/contracts.ts';
import { fitPosterior, predict } from '../src/model.ts';
import { download, parseSource, usableResults } from '../src/provider.ts';
import { digest } from '../src/storage.ts';
import { utcDateBatches } from './backtest-batches.ts';

const root = resolve(import.meta.dirname, '../../..');
const argv = process.argv.slice(2);
const { values } = parseArgs({
  // pnpm forwards the `--` separator of `pnpm backtest -- --league <id>` to the script.
  args: argv[0] === '--' ? argv.slice(1) : argv,
  options: {
    league: { type: 'string' },
    'config-dir': { type: 'string' },
    'data-dir': { type: 'string' },
    season: { type: 'string' },
  },
});
if (values.league === undefined) {
  console.error('Missing required option --league <id>');
  console.error('Usage: pnpm -C apps/web backtest -- --league <id> [--config-dir <dir>] [--data-dir <dir>] [--season <year>]');
  process.exit(2);
}
const league = values.league;
if (!/^[a-zA-Z0-9-]+$/.test(league)) throw new Error(`Unsafe league id: ${league}`);

// The browser app reads these files over HTTP; the backtest reads the same files from disk.
const configBytes = await readFile(resolve(root, values['config-dir'] ?? 'config', `${league}.json`));
const config: LeagueConfig = configSchema.parse(JSON.parse(configBytes.toString('utf8')));
if (config.id !== league) throw new Error(`Configuration id ${config.id} does not match requested league ${league}`);
const season = Number(values.season ?? currentSeason(config));
const dir = resolve(root, values['data-dir'] ?? 'data', config.id);

const seed = seedSchema.parse(JSON.parse(await readFile(resolve(dir, `elo-${season}.json`), 'utf8')));
if (seed.league !== config.id || seed.target_season !== season || seed.through_season !== season - 1)
  throw new Error('Elo seed is for the wrong league or season; run the two Rust programs');
if (seed.config_sha256 !== (await digest(configBytes)))
  throw new Error('Configuration changed since Elo was calculated; rerun generate-preseason-seed');
if ((await digest(await readFile(resolve(dir, 'history.json')))) !== seed.history_sha256)
  throw new Error('History changed since Elo was calculated; rerun generate-preseason-seed');

const texts: string[] = [];
for (const url of adapterFor(config).seasonUrls(config, season)) texts.push(await download(url));
const downloaded = parseSource(texts, config).filter((g) => g.season === season);
const games = usableResults(downloaded, config);
if (!games.length) throw new Error('No completed games to evaluate');
let logLoss = 0,
  brier = 0,
  baselineLogLoss = 0,
  baselineBrier = 0;
// Batch by UTC date: no same-UTC-day or later outcome can leak into a prediction.
for (const { training, testing } of utcDateBatches(games)) {
  const model = fitPosterior(seed, training, config);
  for (const g of testing) {
    const p = predict(model, g.home_team, g.away_team, g.neutral, g.phase);
    const classes = ['home_win', 'away_win', 'tie'] as const;
    const q = config.ties_allowed_in.includes(g.phase) ? seed.tie_weight / (2 + seed.tie_weight) : 0;
    const base = { home_win: (1 - q) / 2, away_win: (1 - q) / 2, tie: q };
    logLoss -= Math.log(Math.max(p[g.result!], 1e-15));
    baselineLogLoss -= Math.log(Math.max(base[g.result!], 1e-15));
    for (const outcome of classes) {
      brier += (p[outcome] - Number(g.result === outcome)) ** 2;
      baselineBrier += (base[outcome] - Number(g.result === outcome)) ** 2;
    }
  }
}
console.log(
  JSON.stringify(
    {
      season,
      games: games.length,
      method: 'Predict using only results from earlier UTC dates; multiclass Brier score, natural-log loss',
      bayesian: { log_loss: logLoss / games.length, brier: brier / games.length },
      equal_strength_baseline: {
        log_loss: baselineLogLoss / games.length,
        brier: baselineBrier / games.length,
      },
      note: 'These are diagnostics, not proof of calibration or superiority. Hyperparameters must be chosen on separate earlier seasons.',
    },
    null,
    2,
  ),
);
