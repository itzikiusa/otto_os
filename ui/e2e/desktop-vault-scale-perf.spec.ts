import { expect, test, type APIRequestContext } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { budgetMs, isDesktopProject, percentile } from './perf';

// Vault at scale (perf/11 F11): a 10k-note vault in the isolated daemon's temp
// dir. API-level budgets — counts first, then p95 timings (scaled by
// OTTO_PERF_BUDGET_SCALE). Files belong to seedVaultDir's own temp fixture.
const NOTES = 10_000;

let ctx: APIRequestContext, base = '', ws = '', id = 0, dir = '';
const v = () => `${base}/api/v1/workspaces/${ws}/vault/vaults/${id}`;

async function timed<T>(fn: () => Promise<T>): Promise<[number, T]> {
  const t0 = performance.now();
  const out = await fn();
  return [performance.now() - t0, out];
}

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  ({ ctx, base } = await apiCtx());
  ws = await seedWorkspace(ctx, base);
  ({ vaultId: id, dir } = await seedVaultDir(ctx, base, ws, { notes: NOTES }));
  test.setTimeout(240_000);
  // Cold index of the whole vault (rescan is synchronous).
  const [ms, res] = await timed(() => ctx.post(`${v()}/rescan`, { data: {}, timeout: 200_000 }));
  expect(res.ok()).toBeTruthy();
  console.log(`[vault-scale] cold index of ${NOTES} notes: ${ms.toFixed(0)} ms`);
});

test.afterAll(async () => { await ctx?.dispose(); });

test('switcher, search and note open stay under p95 budgets at 10k notes', async ({}, info) => {
  test.skip(!isDesktopProject(info.project.name));
  const sw: number[] = [], search: number[] = [], open: number[] = [];
  for (let i = 0; i < 30; i++) {
    const n = (i * 331) % NOTES;
    const [a, r1] = await timed(() => ctx.get(`${v()}/switcher?q=${encodeURIComponent(`Note ${n}`)}`));
    expect(r1.ok()).toBeTruthy();
    sw.push(a);
    const [b, r2] = await timed(() => ctx.post(`${v()}/search`, { data: { query: `Synthetic ${n}`, limit: 20 } }));
    expect(r2.ok()).toBeTruthy();
    search.push(b);
    const [c, r3] = await timed(() => ctx.get(`${v()}/note?path=${encodeURIComponent(`bulk/note-${n}.md`)}`));
    expect(r3.ok()).toBeTruthy();
    open.push(c);
  }
  console.log(`[vault-scale] p95 switcher=${percentile(sw, 95).toFixed(1)} search=${percentile(search, 95).toFixed(1)} open=${percentile(open, 95).toFixed(1)} ms`);
  expect(percentile(sw, 95)).toBeLessThan(budgetMs(150));
  expect(percentile(search, 95)).toBeLessThan(budgetMs(150));
  expect(percentile(open, 95)).toBeLessThan(budgetMs(150));
});

test('status polls do not rescan an unchanged vault; an external edit is picked up', async ({}, info) => {
  test.skip(!isDesktopProject(info.project.name));
  const first = await (await ctx.get(`${v()}/status`)).json();
  const times: number[] = [];
  for (let i = 0; i < 20; i++) {
    const [ms, res] = await timed(() => ctx.get(`${v()}/status`));
    expect(res.ok()).toBeTruthy();
    times.push(ms);
  }
  const after = await (await ctx.get(`${v()}/status`)).json();
  // Nothing changed on disk → the index generation did not move.
  expect(after.generation).toBe(first.generation);
  expect(percentile(times, 95)).toBeLessThan(budgetMs(50));
  // F2: the FSEvents watcher indexes an external note without a manual rescan.
  writeFileSync(join(dir, 'bulk', 'external-new.md'), '# External new\n\nwatched.\n');
  await expect.poll(async () => (await ctx.get(`${v()}/note?path=bulk/external-new.md`)).status(), { timeout: 10_000 }).toBe(200);
  await expect.poll(async () => {
    const hits = await (await ctx.get(`${v()}/switcher?q=${encodeURIComponent('External new')}`)).json();
    return Array.isArray(hits) && hits.some((h: { path: string }) => h.path === 'bulk/external-new.md');
  }, { timeout: 10_000 }).toBe(true);
});

test('editor autosaves coalesce into one history revision', async ({}, info) => {
  test.skip(!isDesktopProject(info.project.name));
  const path = 'bulk/note-7.md';
  let hash = (await (await ctx.get(`${v()}/note?path=${path}`)).json()).meta.hash as string;
  for (let i = 0; i < 20; i++) {
    const res = await ctx.put(`${v()}/note`, { data: { path, content: `# Note 7\n\nedit ${i}\n`, if_hash: hash, autosave: true } });
    expect(res.ok()).toBeTruthy();
    hash = (await res.json()).hash;
  }
  const revs = await (await ctx.get(`${v()}/history?path=${encodeURIComponent(path)}`)).json();
  expect(revs).toHaveLength(1);
  const detail = await (await ctx.get(`${v()}/history/${revs[0].id}`)).json();
  expect(detail.after).toContain('edit 19');
  expect(detail.before).toContain('Synthetic note 7');
});
