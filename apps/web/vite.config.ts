import { createHash } from 'node:crypto';
import { readdir, readFile } from 'node:fs/promises';
import { extname, join, posix, relative, resolve } from 'node:path';
import type { Connect, Plugin } from 'vite';
import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

const projectRoot = resolve(import.meta.dirname, '../..');

/** The source file under the project root and the published bytes for each site path the app reads, or null for a
 * path it doesn't. The app checks a seed against its history by hash alone, so the history publishes as its SHA-256.
 * Everything else in `data/`, such as Elo audits and simulated seasons, is for the offline tools.
 */
function source(path: string): { file: string; publish: (bytes: Buffer) => Buffer | string } | null {
  if (/^config\/[a-zA-Z0-9-]+\.json$/.test(path) || /^data\/[a-zA-Z0-9-]+\/elo-\d+\.json$/.test(path))
    return { file: path, publish: (bytes) => bytes };
  const league = /^data\/([a-zA-Z0-9-]+)\/history\.sha256$/.exec(path)?.[1];
  if (!league) return null;
  return { file: `data/${league}/history.json`, publish: (bytes) => `${createHash('sha256').update(bytes).digest('hex')}\n` };
}

/** The league configuration and the Rust programs' output live at the project root so the
 * CLIs, the backtest and the site all read one copy. This publishes what the app reads as static assets.
 */
function projectData(): Plugin {
  const mounts = ['config', 'data'];
  const types: Record<string, string> = {
    '.json': 'application/json; charset=utf-8',
    '.sha256': 'text/plain; charset=utf-8',
  };
  async function* walk(dir: string): AsyncGenerator<string> {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) yield* walk(path);
      else yield path;
    }
  }
  /** Serves `<base><path>` for published paths as `read(path)` returns it. A path under a mount that isn't published, or
   * whose file is missing, is a 404, as on Pages, rather than the SPA fallback's index.html, so the app reports the file
   * as unpublished instead of failing to parse HTML.
   */
  function serve(base: string, read: (path: string) => Promise<Buffer | string>): Connect.NextHandleFunction {
    return (req, res, next) => {
      const url = decodeURIComponent((req.url ?? '').split('?')[0] ?? '');
      if (!url.startsWith(base)) return next();
      const path = url.slice(base.length);
      if (!mounts.some((m) => path.startsWith(`${m}/`))) return next();
      // Only published paths are read, and their patterns confine reads to the mounted directories.
      (source(path) ? read(path) : Promise.reject(new Error('Not published'))).then(
        (bytes) => {
          res.setHeader('Content-Type', types[extname(path)] ?? 'application/octet-stream');
          res.setHeader('Cache-Control', 'no-store');
          res.end(bytes);
        },
        () => {
          res.statusCode = 404;
          res.end();
        },
      );
    };
  }
  async function published(path: string): Promise<Buffer | string> {
    const { file, publish } = source(path)!;
    return publish(await readFile(join(projectRoot, file)));
  }
  return {
    name: 'project-data',
    configureServer(server) {
      server.middlewares.use(serve(server.config.base, published));
    },
    configurePreviewServer(server) {
      const outDir = resolve(server.config.root, server.config.build.outDir);
      server.middlewares.use(serve(server.config.base, (path) => readFile(join(outDir, path))));
    },
    async generateBundle() {
      for (const mount of mounts) {
        try {
          for await (const file of walk(join(projectRoot, mount))) {
            const path = relative(projectRoot, file).split(/[\\/]/).join(posix.sep);
            const site = path.replace(/\/history\.json$/, '/history.sha256');
            if (source(site)?.file === path) this.emitFile({ type: 'asset', fileName: site, source: await published(site) });
          }
        } catch {
          this.warn(`No ${mount} directory to publish; run the Rust programs before building.`);
        }
      }
    },
  };
}

// Project Pages sites are served from a subdirectory, so asset URLs must be relative to it.
export default defineConfig({
  base: process.env.VITE_BASE ?? '/rally-row/',
  plugins: [react(), projectData()],
  test: { environment: 'node', include: ['test/**/*.test.ts'] },
});
