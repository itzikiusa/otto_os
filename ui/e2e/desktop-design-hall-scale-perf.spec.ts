import { expect, test, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { budgetMs, isDesktopProject, percentile } from './perf';

// Design Hall at scale (perf2/11 R4): a 2k-artifact project in the isolated
// daemon. API-level budgets (scaled by OTTO_PERF_BUDGET_SCALE): the
// CollectionView's first keyset page and its "Load more" pages, a 4 MB content
// save (the autosave a big HTML board makes), and the Settings storage gauge —
// whose auto-tidy dry run walks every candidate artifact — at this size.
// Needs a daemon built from this branch (OTTO_E2E_BIN=target/debug/ottod).

const ARTIFACTS = 2_000;
const PAGE = 120; // CollectionView's page size

let ctx: APIRequestContext, base = '', ws = '', project = '';

async function timed<T>(fn: () => Promise<T>): Promise<[number, T]> {
  const t0 = performance.now();
  const out = await fn();
  return [performance.now() - t0, out];
}

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  test.setTimeout(300_000);
  ({ ctx, base } = await apiCtx());
  ws = await seedWorkspace(ctx, base);
  const p = await ctx.post(`${base}/api/v1/design/projects`, { data: { workspace_id: ws, name: 'Scale' } });
  expect(p.ok()).toBeTruthy();
  project = (await p.json()).id;
  const t0 = performance.now();
  for (let i = 0; i < ARTIFACTS; i += 25) {
    await Promise.all(
      Array.from({ length: Math.min(25, ARTIFACTS - i) }, (_, k) =>
        ctx
          .post(`${base}/api/v1/design/artifacts`, {
            data: {
              workspace_id: ws,
              project_id: project,
              format: 'html',
              title: `Board ${i + k}`,
              content: `<h1>Board ${i + k}</h1>`,
            },
          })
          .then((r) => expect(r.status()).toBe(201)),
      ),
    );
  }
  console.log(`[design-scale] seeded ${ARTIFACTS} artifacts in ${(performance.now() - t0).toFixed(0)} ms`);
});

test.afterAll(async () => { await ctx?.dispose(); });

test('collection first page and keyset pages stay under p95 150 ms at 2k artifacts', async ({}, info) => {
  test.skip(!isDesktopProject(info.project.name));
  const first: number[] = [];
  const next: number[] = [];
  for (let i = 0; i < 20; i++) {
    const [a, r1] = await timed(() => ctx.get(`${base}/api/v1/design/search?q=&project_id=${project}&limit=${PAGE}`));
    expect(r1.ok()).toBeTruthy();
    const page = (await r1.json()) as { artifact: { id: string; updated_at: string } }[];
    expect(page.length).toBe(PAGE);
    first.push(a);
    const last = page[page.length - 1].artifact;
    const cursor = encodeURIComponent(`${last.updated_at}|${last.id}`);
    const [b, r2] = await timed(() =>
      ctx.get(`${base}/api/v1/design/search?q=&project_id=${project}&limit=${PAGE}&cursor=${cursor}`),
    );
    expect(r2.ok()).toBeTruthy();
    const page2 = (await r2.json()) as { artifact: { id: string } }[];
    expect(page2.length).toBe(PAGE);
    expect(new Set([...page, ...page2].map((h) => h.artifact.id)).size, 'keyset pages never overlap').toBe(2 * PAGE);
    next.push(b);
  }
  console.log(`[design-scale] p95 first=${percentile(first, 95).toFixed(1)} next=${percentile(next, 95).toFixed(1)} ms`);
  expect(percentile(first, 95)).toBeLessThan(budgetMs(150));
  expect(percentile(next, 95)).toBeLessThan(budgetMs(150));
});

test('a 4 MB content save stays under budget', async ({}, info) => {
  test.skip(!isDesktopProject(info.project.name));
  const created = await ctx.post(`${base}/api/v1/design/artifacts`, {
    data: { workspace_id: ws, project_id: project, format: 'html', title: 'Big', content: '<p>0</p>' },
  });
  expect(created.status()).toBe(201);
  const aid = (await created.json()).artifact.id as string;
  const times: number[] = [];
  for (let i = 0; i < 5; i++) {
    const html = `<div data-i="${i}">${'x'.repeat(4 * 1024 * 1024)}</div>`;
    const [ms, r] = await timed(() => ctx.put(`${base}/api/v1/design/artifacts/${aid}/content`, { data: { content: html } }));
    expect(r.status()).toBe(200);
    times.push(ms);
  }
  console.log(`[design-scale] 4 MB PUT ms: ${times.map((t) => t.toFixed(0)).join(', ')}`);
  expect(percentile(times, 50)).toBeLessThan(budgetMs(400));
});

test('the storage gauge (auto-tidy dry run) answers fast and auto-tidy stays off', async ({}, info) => {
  test.skip(!isDesktopProject(info.project.name));
  const [ms, r] = await timed(() => ctx.get(`${base}/api/v1/design/admin/storage`));
  expect(r.ok()).toBeTruthy();
  const st = await r.json();
  console.log(`[design-scale] storage gauge ${ms.toFixed(0)} ms, ${st.version_count} versions`);
  expect(st.auto_tidy, 'opt-in: off on a fresh install').toBe(false);
  expect(st.reclaimable.versions, 'nothing older than a week yet').toBe(0);
  expect(ms).toBeLessThan(budgetMs(500));
});
