import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, tempDir } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// DB Watch on REAL engines, no agent (desktop-browser only): MariaDB, Redis,
// MongoDB and Postgres containers (labelled like Testcontainers, so they are
// the run's own databases), the daemon's watch engine, and the panel:
//
//   • rapid stepping — 20 debugger stops (real delve) in ~2 s: no errors, the
//     checkpoints only move forward, every insert shows up;
//   • a 40-column table and a 200-row table (the window note), Redis keys with
//     TTLs (a second checkpoint shows no false "changed"), nested Mongo
//     documents, Postgres;
//   • screenshots of the panel at dock heights 260 / 400, light and dark
//     (OTTO_E2E_SHOTS_DIR, else the test's output folder).
//
// Needs Docker and, for the stepping part, dlv (OTTO_E2E_GO_DIR) + the Go
// module cache (GOMODCACHE, GOPROXY=off).
// ─────────────────────────────────────────────────────────────────────────────

const GO_DIR = process.env.OTTO_E2E_GO_DIR ?? '';
const RUN = Math.random().toString(36).slice(2, 8);
const FIXTURE = join(process.cwd(), 'e2e', 'fixtures', 'go-orders');
const SHOTS = process.env.OTTO_E2E_SHOTS_DIR ?? '';

const haveDocker = (): boolean => {
  try {
    execFileSync('docker', ['info', '--format', '{{.ServerVersion}}'], { stdio: 'ignore', timeout: 15_000 });
    return true;
  } catch {
    return false;
  }
};

test.describe.configure({ mode: 'serial' });
test.use({ actionTimeout: 30_000, serviceWorkers: 'block' });

const docker = (args: string[], timeout = 60_000): string => execFileSync('docker', args, { encoding: 'utf8', timeout }).trim();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const containers: Record<string, string> = {};
const ports: Record<string, number> = {};
let ws = '';
let root = '';
const timings: Record<string, number> = {};

function start(name: string, image: string, port: string, env: string[], cmd: string[] = []): void {
  const id = docker(['run', '-d', '--rm', '--name', `otto-e2e-dbw-${name}-${RUN}`, '--label', 'org.testcontainers=true',
    ...env.flatMap((e) => ['-e', e]), '-p', `127.0.0.1::${port}`, image, ...cmd]);
  containers[name] = id;
  ports[name] = Number(docker(['port', id, port]).split('\n')[0].split(':').pop());
}

async function until(what: string, fn: () => void): Promise<void> {
  for (let i = 0; ; i++) {
    try {
      fn();
      return;
    } catch (e) {
      if (i > 120) throw new Error(`${what}: ${e}`);
      await sleep(500);
    }
  }
}

const maria = (sql: string) => docker(['exec', containers.maria, 'mariadb', '-uroot', '-psecret', 'shop', '-e', sql]);
const redis = (...a: string[]) => docker(['exec', containers.redis, 'redis-cli', ...a]);
const mongo = (js: string) => docker(['exec', containers.mongo, 'mongosh', '--quiet', 'audit', '--eval', js]);
const pg = (sql: string) => docker(['exec', containers.pg, 'psql', '-U', 'postgres', '-d', 'shop', '-c', sql]);

test.beforeAll(async ({}, info) => {
  if (info.project.name !== 'desktop-browser' || !haveDocker()) return;
  test.setTimeout(300_000);
  start('maria', 'mariadb:11.8.9', '3306/tcp', ['MARIADB_ROOT_PASSWORD=secret', 'MARIADB_DATABASE=shop']);
  start('redis', 'redis:7-alpine', '6379/tcp', []);
  start('mongo', 'mongo:8', '27017/tcp', []);
  start('pg', 'postgres:17', '5432/tcp', ['POSTGRES_PASSWORD=secret', 'POSTGRES_DB=shop']);
  await until('mariadb', () => maria('SELECT 1'));
  await until('postgres', () => pg('SELECT 1'));
  await until('mongo', () => mongo('db.runCommand({ ping: 1 })'));
  const cols = Array.from({ length: 39 }, (_, i) => `c${String(i + 1).padStart(2, '0')} VARCHAR(32) NOT NULL DEFAULT 'v${i + 1}'`).join(', ');
  maria(`CREATE TABLE wide (id INT PRIMARY KEY, ${cols}); INSERT INTO wide (id) VALUES (1),(2),(3),(4),(5);
    CREATE TABLE big (id INT PRIMARY KEY, label VARCHAR(32), qty INT);
    INSERT INTO big SELECT seq, CONCAT('item-', seq), seq FROM seq_1_to_200;`);
  for (let i = 1; i <= 5; i++) redis('SET', `sess:${i}`, `player-${i}`, 'EX', '600');
  redis('SET', 'sess:keep', 'forever');
  mongo(`db.events.insertMany([1,2,3].map(i => ({ _id: 'ev' + i, kind: 'bet', player: { id: i, geo: { city: 'Valletta', country: 'MT' } }, tags: ['a'] })))`);
  pg("CREATE TABLE accounts (id INT PRIMARY KEY, owner TEXT, balance NUMERIC(10,2), status TEXT); INSERT INTO accounts VALUES (1,'ava',10.00,'open'),(2,'bo',20.00,'open')");

  const { ctx, base } = await apiCtx();
  root = tempDir('otto-dbw-real-');
  cpSync(FIXTURE, root, { recursive: true });
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: `DB Watch real ${RUN}`, root_path: root } });
  ws = ((await w.json()) as { id: string }).id;
  const goEnv = ['GOMODCACHE', 'GOPROXY', 'GOFLAGS', 'GOCACHE', 'GOTOOLCHAIN'].filter((k) => process.env[k]).map((k) => ({ key: k, value: process.env[k] }));
  const env = await ctx.put(`${base}/api/v1/ide/environments`, {
    data: { target: { kind: 'local', workspace_id: ws, root, connection_id: null }, extra_path: [GO_DIR, '/opt/homebrew/bin'].filter(existsSync).join(':'), env_vars: goEnv },
  });
  expect(env.ok(), await env.text()).toBe(true);
  await ctx.dispose();
});

test.afterAll(() => {
  for (const id of Object.values(containers)) {
    try {
      docker(['rm', '-f', id]);
    } catch {
      /* gone */
    }
  }
  console.log(`[dbwatch timings] ${JSON.stringify(timings)}`);
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  test.skip(!haveDocker(), 'needs Docker');
});

async function openIde(page: Page, opts: { scheme?: string; dockH?: number } = {}): Promise<void> {
  await page.addInitScript(([w, scheme, h]) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_firstrun_dismissed', '1');
    if (scheme) localStorage.setItem('otto_scheme', scheme as string);
    if (h) localStorage.setItem('otto_ide_dock_h', String(h));
  }, [ws, opts.scheme ?? '', opts.dockH ?? 0]);
  await page.goto('/#/ide');
  await page.evaluate(async () => {
    const [s, i] = await Promise.all(['/src/modules/ide/dbwatch/ideDbWatchState.svelte.ts', '/src/modules/ide/ideStore.svelte.ts'].map((u) => import(/* @vite-ignore */ u)));
    (window as any).__dbw = { db: s.ideDbWatch, ide: i.ide };
  });
  // The dock (lazy, always mounted with a project) hands DB Watch its root.
  await expect.poll(() => page.evaluate(() => (window as any).__dbw.db.root), { timeout: 30_000 }).toBeTruthy();
}

/** Start a watch on the container `name` (found by its port) with `items`. */
async function watch(page: Page, engine: string, port: number, items: unknown[], database?: string): Promise<void> {
  const err = await page.evaluate(async ({ engine, port, items, database }) => {
    const { db } = (window as any).__dbw;
    await db.discover();
    const t = db.ephemeralTargets.find((x: any) => x.engine === engine && x.port === port);
    if (!t) return `no ${engine} target on :${port} in ${JSON.stringify(db.ephemeralTargets)}`;
    const w = await db.start({ target: { kind: 'ephemeral', ephemeral_id: t.ephemeral_id }, database: database ?? t.database ?? null, items, bind: null });
    return w ? '' : db.error ?? 'not started';
  }, { engine, port, items, database });
  expect(err).toBe('');
}

const snapshot = (page: Page) => page.evaluate(async () => {
  const { db } = (window as any).__dbw;
  const r = await db.snapshotNow();
  return r ? JSON.parse(JSON.stringify(r)) : { error: db.error };
});

async function showPanel(page: Page): Promise<void> {
  await page.evaluate(() => {
    const { ide } = (window as any).__dbw;
    ide.bottomDockOpen = true;
    ide.activeBottomTab = 'dbwatch';
  });
  await expect(page.getByTestId('ide-dbwatch-panel')).toBeVisible({ timeout: 20_000 });
}

/** The panel at 260 / 400 px, light and dark, after a reload (the watch is the daemon's). */
async function shots(page: Page, name: string, ready: (p: Page) => Promise<void>): Promise<void> {
  for (const scheme of ['light', 'dark']) {
    for (const h of [260, 400]) {
      await page.evaluate(([s, hh]) => {
        localStorage.setItem('otto_scheme', s as string);
        localStorage.setItem('otto_ide_dock_h', String(hh));
      }, [scheme, h]);
      await page.reload();
      await page.evaluate(async () => {
        const [s, i] = await Promise.all(['/src/modules/ide/dbwatch/ideDbWatchState.svelte.ts', '/src/modules/ide/ideStore.svelte.ts'].map((u) => import(/* @vite-ignore */ u)));
        (window as any).__dbw = { db: s.ideDbWatch, ide: i.ide };
      });
      await expect.poll(() => page.evaluate(() => (window as any).__dbw.db.watch?.id ?? null), { timeout: 30_000 }).toBeTruthy();
      await showPanel(page);
      await ready(page);
      const file = `e2e-dbwatch-${name}-${h}-${scheme}.png`;
      if (SHOTS) mkdirSync(SHOTS, { recursive: true });
      await page.screenshot({ path: SHOTS ? join(SHOTS, file) : test.info().outputPath(file), animations: 'disabled' });
    }
  }
}

test('MySQL: a 40-column table and a 200-row table — changed cells, the window note, readable at 260 / 400', async ({ page }) => {
  test.setTimeout(240_000);
  await openIde(page);
  await watch(page, 'mysql', ports.maria, [{ object: 'wide' }, { object: 'big' }], 'shop');
  maria("UPDATE wide SET c07 = 'changed-7', c33 = 'changed-33' WHERE id = 2; INSERT INTO wide (id) VALUES (6); UPDATE big SET qty = 999 WHERE id = 200; DELETE FROM big WHERE id = 199;");
  const t0 = Date.now();
  const r = await snapshot(page);
  timings.mysql_snapshot_ms = Date.now() - t0;
  const wide = r.items.find((i: any) => i.object === 'wide');
  const big = r.items.find((i: any) => i.object === 'big');
  expect(wide.columns).toHaveLength(40);
  expect(wide.added_count).toBe(1);
  expect(wide.modified_count).toBe(1);
  expect(wide.diffs.find((d: any) => d.kind === 'modified').changes.map((c: any) => c.column).sort()).toEqual(['c07', 'c33']);
  expect(big.window_note).toMatch(/150/);
  expect(big.modified_count).toBe(1);
  expect(big.removed_count).toBe(1);
  expect(big.added_count, 'the row the delete pulled into the window is not new').toBe(0);
  await showPanel(page);
  await expect(page.getByTestId('ide-dbw-table').first()).toBeVisible();
  await shots(page, 'mysql', async (p) => {
    await expect(p.getByTestId('ide-dbw-diff-row').first()).toBeVisible({ timeout: 20_000 });
    await expect(p.getByTestId('ide-dbwatch-panel')).toContainText('changed-33');
  });
});

test('Redis keys with TTLs: a change shows, a quiet checkpoint shows nothing changed', async ({ page }) => {
  test.setTimeout(240_000);
  await openIde(page);
  await watch(page, 'redis', ports.redis, [{ object: 'sess:*' }]);
  redis('SET', 'sess:2', 'player-2-moved', 'KEEPTTL');
  redis('SET', 'sess:9', 'player-9', 'EX', '600');
  const r1 = await snapshot(page);
  const it1 = r1.items[0];
  expect(it1.modified_count, JSON.stringify(it1)).toBe(1);
  expect(it1.added_count).toBe(1);
  await sleep(2_000);
  const r2 = await snapshot(page);
  const it2 = r2.items[0];
  expect(it2.modified_count + it2.added_count + it2.removed_count, 'TTLs tick down: not a change').toBe(0);
  redis('SET', 'sess:3', 'player-3-moved', 'KEEPTTL');
  await snapshot(page);
  await shots(page, 'redis', async (p) => {
    await expect(p.getByTestId('ide-dbw-diff-row').first()).toBeVisible({ timeout: 20_000 });
  });
});

test('Mongo nested documents and Postgres rows', async ({ page }) => {
  test.setTimeout(300_000);
  await openIde(page);
  await watch(page, 'mongodb', ports.mongo, [{ object: 'events' }], 'audit');
  mongo("db.events.updateOne({ _id: 'ev2' }, { $set: { 'player.geo.city': 'Sliema' }, $push: { tags: 'b' } }); db.events.insertOne({ _id: 'ev4', kind: 'win', player: { id: 4, geo: { city: 'Rabat', country: 'MT' } } })");
  const m = (await snapshot(page)).items[0];
  expect(m.modified_count, JSON.stringify(m).slice(0, 2000)).toBe(1);
  expect(m.added_count).toBe(1);
  expect(JSON.stringify(m.diffs.find((d: any) => d.kind === 'modified').changes)).toContain('Sliema');
  await shots(page, 'mongo', async (p) => {
    await expect(p.getByTestId('ide-dbw-diff-row').first()).toBeVisible({ timeout: 20_000 });
  });

  await watch(page, 'postgres', ports.pg, [{ object: 'accounts' }], 'public');
  pg("UPDATE accounts SET balance = 7.50, status = 'closed' WHERE id = 1; INSERT INTO accounts VALUES (3,'cy',0,'open')");
  const p = (await snapshot(page)).items[0];
  expect(p.modified_count, JSON.stringify(p).slice(0, 2000)).toBe(1);
  expect(p.added_count).toBe(1);
  await shots(page, 'postgres', async (pp) => {
    await expect(pp.getByTestId('ide-dbw-diff-row').first()).toBeVisible({ timeout: 20_000 });
  });
});

test('rapid stepping: 20 real debugger stops in a row — no errors, checkpoints only move forward, every insert shows', async ({ page }) => {
  test.skip(!GO_DIR || !existsSync(join(GO_DIR, 'dlv')), 'needs dlv (OTTO_E2E_GO_DIR)');
  test.setTimeout(300_000);
  const line = readFileSync(join(FIXTURE, 'rapid_test.go'), 'utf8').split('\n').findIndex((l) => l.includes('insertOrder(d,')) + 1;
  const dsn = `root:secret@tcp(127.0.0.1:${ports.maria})/shop`;
  maria('CREATE TABLE IF NOT EXISTS orders (id INT AUTO_INCREMENT PRIMARY KEY, customer VARCHAR(64) NOT NULL, amount INT NOT NULL, status VARCHAR(16) NOT NULL, api_token VARCHAR(64) NOT NULL)');
  await openIde(page);
  // The person's gutter Debug on TestRapid, with one breakpoint in the loop.
  const err = await page.evaluate(async ({ line, dsn }) => {
    const { ide } = (window as any).__dbw;
    const portUrl = '/src/lib/uiCommands/idePort.ts';
    const portMod = await import(/* @vite-ignore */ portUrl);
    const port = await portMod.idePort.get(new AbortController().signal, 30_000);
    const path = ide.resolvePath('rapid_test.go');
    ide.breakpointsByPath = { ...ide.breakpointsByPath, [path]: [{ path, line, enabled: true }] };
    const plan = await port.resolveTarget({ kind: 'test', file: 'rapid_test.go', name: 'TestRapid' }, { env: { ORDERS_DSN: dsn } });
    await ide.startDebug(plan.plan);
    return ide.activeDebug ? '' : 'no debug session';
  }, { line, dsn });
  expect(err).toBe('');
  const t0 = Date.now();
  await expect.poll(() => page.evaluate(() => (window as any).__dbw.ide.activeDebug?.state), { timeout: 120_000 }).toBe('paused');
  timings.rapid_launch_to_first_stop_ms = Date.now() - t0;
  await watch(page, 'mysql', ports.maria, [{ object: 'orders' }], 'shop');
  await page.evaluate(async () => {
    const { db, ide } = (window as any).__dbw;
    await db.bind({ debug_session_id: ide.activeDebug.id });
  });
  await showPanel(page);
  // 20 Continues as fast as the debugger answers; every checkpoint seq seen must only grow.
  const t1 = Date.now();
  const seen = await page.evaluate(async () => {
    const { db, ide } = (window as any).__dbw;
    const apiUrl = '/src/lib/api/client.ts';
    const api = (await import(/* @vite-ignore */ apiUrl)).api;
    const seqs: number[] = [];
    const id = ide.activeDebug.id;
    for (let i = 0; i < 20; i++) {
      const before = ide.activeDebug?.stop_seq ?? 0;
      const r = await api.post(`/ide/debug/sessions/${encodeURIComponent(id)}/command`, { action: 'continue', thread_id: ide.activeDebug?.active_thread_id ?? undefined });
      ide.applyDebugSnapshot(r.session);
      for (let k = 0; k < 200 && (ide.activeDebug?.state === 'running' || (ide.activeDebug?.stop_seq ?? 0) === before) && ide.activeDebug?.state !== 'terminated'; k++) {
        await new Promise((res) => setTimeout(res, 10));
      }
      if (db.watch?.latest?.seq != null) seqs.push(db.watch.latest.seq);
      if (ide.activeDebug?.state === 'terminated') break;
    }
    return { seqs, error: db.error, state: ide.activeDebug?.state };
  });
  timings.rapid_20_continues_ms = Date.now() - t1;
  console.log(`[rapid] ${JSON.stringify(seen)}`);
  expect(seen.error).toBeNull();
  for (let i = 1; i < seen.seqs.length; i++) expect(seen.seqs[i]).toBeGreaterThanOrEqual(seen.seqs[i - 1]);
  // The last checkpoint (the run's end) has every insert, since the start.
  await expect.poll(async () => {
    const r = await page.evaluate(async () => JSON.parse(JSON.stringify(await (window as any).__dbw.db.diffSince('start'))));
    return r?.items?.[0]?.added_count ?? -1;
  }, { timeout: 30_000 }).toBe(20);
  await page.getByTestId('ide-dbw-since-start').click();
  await expect(page.getByTestId('ide-dbw-table-counts').first()).toContainText('20');
  const file = 'e2e-dbwatch-rapid-400-light.png';
  await page.screenshot({ path: SHOTS ? join(SHOTS, file) : test.info().outputPath(file), animations: 'disabled' });
});
