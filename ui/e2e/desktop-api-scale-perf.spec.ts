import { expect, test, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openApiEditor, openPage } from './helpers';
import { dist, domCount, frameDeltas } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// API client at scale (perf sweep B: SB-01, SB-02, SB-05). Budgets are DOM /
// socket counts, not timings, so they hold on any machine:
//   • a 3,000-request workspace keeps the collections sidebar windowed — big
//     workspaces open with folders collapsed, and a search that matches hundreds of
//     requests still mounts only a viewport's worth of rows;
//   • the API client's scratch editors (body, scripts, docs, response) never
//     open a language-server socket (each one used to spawn a server process).
// Plus the GAPS §5 gates (perf batch I5) — timings where the gap IS a timing:
//   • search keystroke frame p95 < 50 ms at 3k requests;
//   • URL keystroke p95 < 4 ms, body keystroke p95 < 16 ms on 200 KB of
//     minified JSON (keydown → handlers + reactive flush, see `keyCosts`);
//   • the automation editor stays < 3k nodes at 20 steps × 3k requests;
//   • Body↔Headers / Pretty↔Tree in the response creates 0 new `.cm-editor`;
//   • a splitter drag writes localStorage 0 times until mouseup.
// Mounted production components; only the api-client LIST transport is mocked.
//
// Desktop-browser project only.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try {
    workspaceId = await seedWorkspace(ctx, base);
  } finally {
    await ctx.dispose().catch(() => {});
  }
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** 3,000 requests spread over 30 collections × 10 folders. */
function bigTree(wid: string) {
  const now = '2026-09-01T00:00:00Z';
  const cols: Record<string, unknown>[] = [];
  const leaves: string[] = [];
  for (let r = 0; r < 30; r++) {
    const rid = `col-${r}`;
    cols.push({ id: rid, workspace_id: wid, name: `Service ${r}`, parent_id: null, position: r, created_at: now });
    for (let s = 0; s < 10; s++) {
      const sid = `col-${r}-${s}`;
      cols.push({ id: sid, workspace_id: wid, name: `Folder ${r}.${s}`, parent_id: rid, position: s, created_at: now });
      leaves.push(sid);
    }
  }
  const reqs = Array.from({ length: 3000 }, (_, i) => ({
    id: `req-${i}`, workspace_id: wid, collection_id: leaves[i % leaves.length],
    name: `${i % 3 === 0 ? 'Create' : 'Get'} resource ${i}`, method: i % 3 === 0 ? 'POST' : 'GET',
    url: `https://api.example.com/v1/resource/${i}`, headers: [], query: [], body_mode: 'none', body: '',
    auth: { type: 'none' }, extras: null, position: i, created_at: now, updated_at: now,
  }));
  return { cols, reqs };
}

async function mockBigWorkspace(page: Page): Promise<void> {
  const { cols, reqs } = bigTree(workspaceId);
  const base = `/api/v1/workspaces/${workspaceId}/api-client`;
  await page.route(`**${base}/collections`, (route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: cols }) : route.fallback());
  await page.route(`**${base}/requests`, (route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: reqs }) : route.fallback());
}

test('3,000 saved requests: the sidebar stays windowed while browsing and searching', async ({ page }) => {
  await mockBigWorkspace(page);
  await openPage(page, 'api');
  const tree = page.locator('.tree-wrap');
  const rows = tree.locator('.req-row');
  const heads = tree.locator('.col-head');

  // Big workspaces open collapsed: collection rows only, no request rows.
  await expect(heads.first()).toBeVisible({ timeout: 30_000 });
  await expect(rows).toHaveCount(0);
  expect(await heads.count()).toBeLessThanOrEqual(60);

  // Hundreds of matches force their branches open — still only a window is mounted.
  const input = page.getByLabel('Search collections and requests');
  await input.fill('resource 12');
  await expect(rows.first()).toBeVisible();
  expect(await rows.count()).toBeLessThanOrEqual(150);
  expect(await tree.locator('.req-row, .col-head').count()).toBeLessThanOrEqual(150);

  // A precise search finds a request deep in the list.
  await input.fill('resource 2999');
  await expect(rows).toHaveCount(1);
  await expect(tree.getByText('Get resource 2999', { exact: true })).toBeVisible();

  // Opening one folder shows only its requests.
  await input.fill('');
  await expect(rows).toHaveCount(0);
  const folder = (name: string) => tree.locator('.col-toggle', { has: page.locator('.col-name', { hasText: new RegExp(`^${name.replace('.', '\\.')}$`) }) });
  await folder('Service 0').click();
  await folder('Folder 0.0').click();
  await expect(rows.first()).toBeVisible();
  expect(await rows.count()).toBeLessThanOrEqual(150);
});

test('API client editors never open a language-server socket', async ({ page }) => {
  const lspSockets: string[] = [];
  page.on('websocket', (ws) => {
    if (ws.url().includes('/ws/lsp')) lspSockets.push(ws.url());
  });
  await openApiEditor(page);
  for (const name of [/^Body/, /^Scripts/, /^Docs/, /^Body/]) {
    const tab = page.getByRole('tab', { name });
    if (await tab.count()) await tab.first().click();
    await page.waitForTimeout(150);
  }
  // Give a (wrongly) attached server time to connect before asserting.
  await page.waitForTimeout(800);
  expect(lspSockets).toEqual([]);
});

// ── GAPS §5 gates ─────────────────────────────────────────────────────────────

/** Per-keystroke main-thread cost: keydown (capture) → a microtask queued by
 *  the window's bubble-phase `input` listener (or the bubble-phase keydown,
 *  for a key an editor keymap consumes — Backspace/Enter in CodeMirror fire
 *  no `input`). keydown→input run in ONE task and microtasks drain after
 *  every listener, so by then every sync handler and the reactive flush the
 *  key caused have run — no Playwright IPC gap, no paint.
 *  (A MessageChannel task used to end the sample; the browser may render a
 *  frame before that task, so every other keystroke at a 15 ms cadence
 *  counted a full style/layout/paint — p95 ~9 ms with ~0.8 ms of script.) */
async function watchKeyCosts(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as { __kc: number[]; __kcOn?: boolean };
    w.__kc = [];
    if (w.__kcOn) return;
    w.__kcOn = true;
    let t0 = 0;
    let armed = false;
    const end = () =>
      queueMicrotask(() => {
        if (!armed) return;
        armed = false;
        w.__kc.push(performance.now() - t0);
      });
    window.addEventListener('keydown', () => {
      t0 = performance.now();
      armed = true;
    }, true);
    window.addEventListener('input', end);
    window.addEventListener('keydown', (e) => {
      if (e.defaultPrevented) end();
    });
  });
}
async function keyCosts(page: Page): Promise<number[]> {
  return page.evaluate(() => (window as unknown as { __kc?: number[] }).__kc ?? []);
}

test('search keystrokes at 3,000 requests keep frames under 50 ms (p95)', async ({ page }) => {
  await mockBigWorkspace(page);
  await openPage(page, 'api');
  const input = page.getByLabel('Search collections and requests');
  await expect(page.locator('.tree-wrap .col-head').first()).toBeVisible({ timeout: 30_000 });
  const frames = await frameDeltas(page, () => input.pressSequentially('resource 12', { delay: 40 }));
  expect(frames.n).toBeGreaterThan(5);
  expect(frames.p95, `search frame deltas ${JSON.stringify(frames)}`).toBeLessThan(50);
});

test('URL keystrokes cost < 4 ms (p95) at 3,000 requests', async ({ page }) => {
  await mockBigWorkspace(page);
  await openApiEditor(page);
  const url = page.getByLabel('Request URL', { exact: true });
  await url.fill('');
  await url.focus();
  await watchKeyCosts(page);
  await url.pressSequentially('https://api.example.com/v1/{{tenant}}/orders?page=2', { delay: 15 });
  const d = dist(await keyCosts(page));
  expect(d.n).toBeGreaterThan(30);
  expect(d.p95, `URL keystroke cost ${JSON.stringify(d)}`).toBeLessThan(4);
});

test('body keystrokes cost < 16 ms (p95) on 200 KB of minified JSON', async ({ page }) => {
  await openApiEditor(page);
  const bodyTab = page.getByRole('tab', { name: /^Body/ }).first();
  await bodyTab.click();
  await page.getByLabel('Body type').selectOption('json');
  const content = page.locator('.body-editor .cm-content').first();
  await expect(content).toBeVisible();
  await content.click();
  const item = '{"id":123456,"name":"resource","tags":["a","b","c"],"ok":true},';
  const big = `[${item.repeat(Math.ceil((200 * 1024) / item.length))}{}]`;
  await page.keyboard.insertText(big);
  await page.keyboard.press('End');
  await watchKeyCosts(page);
  await page.keyboard.type('"typed"', { delay: 20 });
  for (let i = 0; i < 20; i++) await page.keyboard.press('Backspace', { delay: 20 });
  const d = dist(await keyCosts(page));
  expect(d.n).toBeGreaterThan(20);
  expect(d.p95, `body keystroke cost ${JSON.stringify(d)}`).toBeLessThan(16);
});

test('automation editor: 20 steps over 3,000 requests stays under 3k DOM nodes', async ({ page }) => {
  await mockBigWorkspace(page);
  const base = `/api/v1/workspaces/${workspaceId}/api-client`;
  const steps = Array.from({ length: 20 }, (_, i) => ({ request_id: `req-${i * 150}`, assertions: [], extract: [] }));
  await page.route(`**${base}/automations`, (route) =>
    route.request().method() === 'GET'
      ? route.fulfill({ json: [{ id: 'auto-big', workspace_id: workspaceId, name: 'Big flow', steps, created_at: '2026-09-01T00:00:00Z' }] })
      : route.fallback());
  await page.route(`**${base}/automation-runs**`, (route) =>
    route.request().method() === 'GET' ? route.fulfill({ json: [] }) : route.fallback());
  await page.addInitScript(() => localStorage.setItem('otto_api_side', 'automations'));
  await openPage(page, 'api');
  await page.locator('.auto-pick', { hasText: 'Big flow' }).click();
  await expect(page.getByLabel('Request for step 20')).toBeVisible({ timeout: 30_000 });
  const nodes = await domCount(page, '.auto *');
  expect(nodes, 'automation editor DOM nodes').toBeLessThan(3000);
});

test('response Body↔Headers and Pretty↔Tree keep the one response editor', async ({ page }) => {
  const lspSockets: string[] = [];
  page.on('websocket', (ws) => {
    if (ws.url().includes('/ws/lsp')) lspSockets.push(ws.url());
  });
  const rows = Array.from({ length: 800 }, (_, i) => ({ id: i, name: `row ${i}`, ok: i % 2 === 0 }));
  const body = JSON.stringify({ rows });
  await page.route('**/api-client/execute', (r) => r.fulfill({ json: {
    status: 200, status_text: 'OK', headers: [{ key: 'Content-Type', value: 'application/json' }],
    body, body_base64: '', truncated: false, too_large: false, duration_ms: 12, size_bytes: body.length,
    content_type: 'application/json', trace: [{ label: 'Response', detail: 'fixture', ms: 12, level: 'success' }],
  } }));
  await openApiEditor(page);
  await page.getByLabel('Request URL', { exact: true }).fill('https://fixture.invalid/rows');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  const resp = page.locator('.resp-editor .cm-editor');
  await expect(resp).toBeVisible();
  await page.evaluate(() => document.querySelectorAll('.cm-editor').forEach((e) => e.setAttribute('data-perf-seen', '')));

  const tabs = page.getByRole('tablist', { name: 'Response' });
  for (let i = 0; i < 3; i++) {
    await tabs.getByRole('tab', { name: /^Headers/ }).click();
    await expect(resp).toBeHidden();
    await tabs.getByRole('tab', { name: /^Body/ }).click();
    await expect(resp).toBeVisible();
    await page.getByRole('button', { name: 'Tree', exact: true }).click();
    await expect(resp).toBeHidden();
    await page.getByRole('button', { name: 'Pretty', exact: true }).click();
    await expect(resp).toBeVisible();
  }
  expect(await domCount(page, '.cm-editor:not([data-perf-seen])'), 'new CodeMirror views').toBe(0);
  expect(lspSockets).toEqual([]);
});

test('dragging the API sidebar splitter persists once, on release', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __lsWrites: string[] };
    w.__lsWrites = [];
    const orig = Storage.prototype.setItem;
    Storage.prototype.setItem = function (k: string, v: string) {
      if (k === 'otto_api_side_width' || k === 'otto_api_builder_h') w.__lsWrites.push(k);
      return orig.call(this, k, v);
    };
  });
  await openApiEditor(page);
  const handle = page.getByLabel('Resize the sidebar');
  const box = await handle.boundingBox();
  expect(box).not.toBeNull();
  const x = box!.x + box!.width / 2;
  const y = box!.y + box!.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 12; i++) await page.mouse.move(x + i * 6, y, { steps: 2 });
  const during = await page.evaluate(() => (window as unknown as { __lsWrites: string[] }).__lsWrites.length);
  expect(during, 'localStorage writes while dragging').toBe(0);
  await page.mouse.up();
  const after = await page.evaluate(() => (window as unknown as { __lsWrites: string[] }).__lsWrites);
  expect(after).toEqual(['otto_api_side_width']);
});
