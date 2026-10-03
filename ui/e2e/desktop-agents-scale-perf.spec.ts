import { test, expect, type APIRequestContext, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { budgetMs, isDesktopProject, longTasks, watchFatalUiErrors, watchLongTasks } from './perf';

// Agents page at real-data scale (perf section 13, F1/F3/F5; round 2 R1/R2/R4/R6). The main
// workspace in the field holds ~1.9 k hidden background review agents; the
// sidebar must never download them. Route-boundary fixtures answer the
// session list exactly like the daemon's SQL filter (foreground / ids /
// archived paging) and the event socket is a mock, so nothing is launched.

let ctx: APIRequestContext;
let base = '';
let wsA = '';
let wsB = '';

const BG = 2000;
const ARCHIVED = 300;
const SHOWN = 60;
// = BACKGROUND_SESSION_SOURCES (domain.rs) — the fixture only uses a few.
const BACKGROUND = new Set(['channel', 'review', 'review_summarizer', 'workflow', 'swarm']);

type Row = Record<string, unknown> & { id: string; workspace_id: string; kind: string; archived: boolean; created_at: string; meta: Record<string, unknown> };

function at(i: number): string {
  return new Date(Date.UTC(2026, 6, 1) + i * 60_000).toISOString();
}

function row(ws: string, id: string, i: number, extra: Partial<Row> = {}): Row {
  return {
    id,
    workspace_id: ws,
    kind: 'agent',
    provider: 'claude',
    title: `Title ${id}`,
    status: 'idle',
    cwd: '/tmp/otto-fixture',
    provider_session_id: null,
    connection_id: null,
    created_by: 'fixture',
    created_at: at(i),
    last_active_at: at(i),
    archived: false,
    meta: {},
    live: false,
    viewers: 0,
    ...extra,
  };
}

function rowsFor(ws: string, prefix: string, bg: number, archived: number, shown: number): Row[] {
  const out: Row[] = [];
  let i = 0;
  for (let k = 0; k < bg; k++) out.push(row(ws, `${prefix}-bg-${k}`, i++, { meta: { source: 'review' }, title: `Review agent ${k}` }));
  for (let k = 0; k < archived; k++) out.push(row(ws, `${prefix}-arch-${k}`, i++, { archived: true, title: `Archived ${prefix} ${k}` }));
  for (let k = 0; k < shown; k++) out.push(row(ws, `${prefix}-fg-${k}`, i++, { title: `Agent ${prefix} ${k}` }));
  return out;
}

/** The daemon's `list_filtered`, for the fixture rows. */
function filter(rows: Row[], q: URLSearchParams, archivedDefault: boolean | null): Row[] {
  const ids = q.get('ids');
  const idSet = ids != null ? new Set(ids.split(',').filter(Boolean)) : null;
  const a = q.get('archived');
  const archived = a === 'true' ? true : a === 'false' ? false : idSet ? null : archivedDefault;
  const fg = q.get('foreground');
  const withSources = new Set((q.get('with_sources') ?? '').split(',').filter(Boolean));
  const shown = (r: Row) => {
    const src = r.meta.source;
    return r.kind !== 'agent' || typeof src !== 'string' || !BACKGROUND.has(src) || withSources.has(src);
  };
  let out = rows.filter(
    (r) =>
      (archived === null || r.archived === archived) &&
      (!idSet || idSet.has(r.id)) &&
      (fg == null || (fg === 'true' ? shown(r) : !shown(r))),
  );
  const before = q.get('before');
  if (before) out = out.filter((r) => r.created_at < before);
  const limit = Number(q.get('limit') ?? 0);
  if (limit > 0) out = out.slice(-limit);
  return out;
}

test.beforeEach(async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop fixture');
  ({ ctx, base } = await apiCtx());
  wsA = await seedWorkspace(ctx, base);
  wsB = await seedWorkspace(ctx, base);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsA);
});
test.afterEach(async () => {
  await ctx?.dispose();
});

test('2,000 background + 300 archived sessions: list, archive paging, event storm, workspace switch', async ({ page, browserName }) => {
  test.setTimeout(120_000);
  const fatal = watchFatalUiErrors(page);
  const all: Row[] = [...rowsFor(wsA, 'a', BG, ARCHIVED, SHOWN), ...rowsFor(wsB, 'b', 0, 0, SHOWN)];
  const lists: { path: string; q: URLSearchParams; bytes: number }[] = [];

  const sockets: { send(data: string): void }[] = [];
  await page.routeWebSocket('**/ws/events*', (socket) => {
    sockets.push(socket);
  });
  const answer = (route: Route) => {
    if (route.request().method() !== 'GET') return route.fallback();
    const url = new URL(route.request().url());
    const m = /\/api\/v1\/workspaces\/([^/]+)\/sessions$/.exec(url.pathname);
    let rows: Row[];
    if (m) rows = filter(all.filter((r) => r.workspace_id === m[1]), url.searchParams, null);
    else rows = filter(all, url.searchParams, false);
    const body = JSON.stringify(rows);
    lists.push({ path: url.pathname, q: url.searchParams, bytes: body.length });
    return route.fulfill({ contentType: 'application/json', body });
  };
  await page.route(/\/api\/v1\/workspaces\/[^/]+\/sessions(\?.*)?$/, answer);
  await page.route(/\/api\/v1\/sessions(\?.*)?$/, answer);

  await page.goto('/#/agents');
  // The newest session also opens as the selected tab/pane (list/detail opens
  // on an item), so its title renders up to three times: check the list row.
  await expect(page.getByText(`Agent a ${SHOWN - 1}`, { exact: true }).first()).toBeVisible({ timeout: 45_000 });

  // F1 — the main list asks for shown rows only; nothing pulls the full table.
  const main = lists.filter((l) => l.path === `/api/v1/workspaces/${wsA}/sessions` && l.q.get('archived') === 'false');
  expect(main.length).toBeGreaterThan(0);
  for (const l of main) {
    expect(l.q.get('foreground')).toBe('true');
    expect(l.bytes, 'main list payload').toBeLessThan(100_000);
  }
  const unfiltered = lists.filter((l) => /\/workspaces\//.test(l.path) && !l.q.has('foreground') && !l.q.has('archived') && !l.q.has('ids'));
  expect(unfiltered.map((l) => l.path), 'no unfiltered session list').toEqual([]);
  expect(await page.locator('.nav-item', { hasText: /^Review agent/ }).count()).toBe(0);

  // F3 — Archived loads lazily, 100 at a time, with Load more.
  expect(lists.some((l) => l.q.get('archived') === 'true' && l.q.get('limit') === '100')).toBe(false);
  await page.getByTestId('archived-toggle').click();
  await expect.poll(() => lists.some((l) => l.q.get('archived') === 'true' && l.q.get('limit') === '100')).toBe(true);
  await expect(page.locator('.nested-item.archived')).toHaveCount(100);
  await page.getByTestId('archived-load-more').click();
  await expect(page.locator('.nested-item.archived')).toHaveCount(200);
  expect(lists.some((l) => l.q.get('archived') === 'true' && l.q.has('before'))).toBe(true);
  await page.getByTestId('archived-toggle').click();

  // R4 — the archived "any?" probe runs once per selection, not per refresh.
  const probes = () => lists.filter((l) => l.q.get('archived') === 'true' && l.q.get('limit') === '1').length;
  const probesBefore = probes();
  await page.evaluate(async () => {
    const path = '/src/lib/stores/workspace.svelte.ts';
    const { ws } = await import(/* @vite-ignore */ path);
    await ws.refreshSessions();
    await ws.refreshSessions();
  });
  expect(probes(), 'archived probe re-ran on a plain refresh').toBe(probesBefore);

  // F2/F5/R6 — 200 status events (background + shown ids) through the real
  // socket path: on Chromium no long task (the PerformanceObserver entry type
  // WebKit lacks).
  expect(sockets.length).toBeGreaterThan(0);
  if (browserName === 'chromium') await watchLongTasks(page);
  for (let k = 0; k < 200; k++) {
    const id = k % 4 === 0 ? `a-fg-${k % SHOWN}` : `a-bg-${k}`;
    const status = k % 2 === 0 ? 'working' : 'idle';
    const frame = JSON.stringify({ type: 'session_status', session_id: id, workspace_id: wsA, status });
    for (const s of sockets) s.send(frame);
  }
  await page.waitForTimeout(500);
  if (browserName === 'chromium') {
    const lt = await longTasks(page);
    expect(Math.max(0, ...lt), `long tasks: ${lt.map((x) => x.toFixed(0)).join(', ')}`).toBeLessThan(50);
  }
  // Engine-neutral (R1 — CI runs desktop-webkit): time 200 store updates
  // synchronously with performance.now(), then the frames that flush the
  // coalesced row write (R6) and repaint the sidebar.
  const storm = await page.evaluate(
    async ({ ws: wsId, shown }) => {
      const path = '/src/lib/stores/workspace.svelte.ts';
      const { ws } = await import(/* @vite-ignore */ path);
      const frame = () => new Promise<number>((r) => requestAnimationFrame(r));
      await frame();
      const t0 = performance.now();
      for (let k = 0; k < 200; k++) {
        const id = k % 4 === 0 ? `a-fg-${k % shown}` : `a-bg-${1000 + k}`;
        const status = k % 4 === 0 ? 'idle' : k % 2 === 0 ? 'working' : 'idle';
        ws.applyEvent({ type: 'session_status', session_id: id, workspace_id: wsId, status });
      }
      const syncMs = performance.now() - t0;
      let last = await frame();
      let maxGap = last - t0;
      for (let i = 0; i < 3; i++) {
        const t = await frame();
        maxGap = Math.max(maxGap, t - last);
        last = t;
      }
      return { syncMs, maxGap, flushed: ws.getSession('a-fg-0')?.status ?? null };
    },
    { ws: wsA, shown: SHOWN },
  );
  expect(storm.syncMs, `200 status events took ${storm.syncMs.toFixed(1)} ms`).toBeLessThan(budgetMs(50));
  expect(storm.maxGap, `frame gap after the storm ${storm.maxGap.toFixed(1)} ms`).toBeLessThan(budgetMs(100));
  expect(storm.flushed, 'the coalesced status write landed').toBe('idle');

  // R2 — a background row picked up live leaves the list once it exits; the
  // all-workspaces view never takes another workspace's background rows.
  const growth = await page.evaluate(
    async ({ a, b }) => {
      const path = '/src/lib/stores/workspace.svelte.ts';
      const { ws } = await import(/* @vite-ignore */ path);
      const mk = (id: string, w: string, meta: Record<string, unknown>) => ({
        id,
        workspace_id: w,
        kind: 'agent',
        provider: 'claude',
        title: id,
        status: 'working',
        cwd: '/tmp',
        provider_session_id: null,
        connection_id: null,
        created_by: 'fixture',
        created_at: new Date().toISOString(),
        last_active_at: new Date().toISOString(),
        archived: false,
        meta,
        live: true,
        viewers: 0,
      });
      ws.allWorkspaces = true;
      ws.applyEvent({ type: 'session_created', session: mk('live-bg', a, { source: 'review' }) });
      const added = ws.getSession('live-bg') != null;
      ws.applyEvent({ type: 'session_status', session_id: 'live-bg', workspace_id: a, status: 'exited' });
      // Dropped after a grace period (a draft dialog still shows its agent);
      // run the timer's body now.
      const kept = ws.getSession('live-bg') != null;
      ws.dropExitedBackground('live-bg');
      const dropped = kept && ws.getSession('live-bg') == null;
      ws.applyEvent({ type: 'session_created', session: mk('other-bg', b, { source: 'review' }) });
      ws.applyEvent({ type: 'session_created', session: mk('other-fg', b, {}) });
      const other = (ws.otherWsSessions as { id: string }[]).map((s) => s.id);
      ws.allWorkspaces = false;
      return { added, dropped, other };
    },
    { a: wsA, b: wsB },
  );
  expect(growth).toEqual({ added: true, dropped: true, other: ['other-fg'] });

  // Workspace switch reaches layout restore fast (the list is ~60 rows now).
  const switchMs = await page.evaluate(async (target) => {
    const path = '/src/lib/stores/workspace.svelte.ts';
    const { ws } = await import(/* @vite-ignore */ path);
    const t0 = performance.now();
    await ws.select(target);
    return performance.now() - t0;
  }, wsB);
  expect(switchMs, `workspace switch ${switchMs.toFixed(0)} ms`).toBeLessThan(budgetMs(300));
  await expect(page.getByText(`Agent b ${SHOWN - 1}`, { exact: true }).first()).toBeVisible();
  expect(fatal).toEqual([]);
});
