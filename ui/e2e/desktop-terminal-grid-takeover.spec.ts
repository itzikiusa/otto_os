import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Grid feed (docs/contracts/ws.md "Grid feed"): when ANOTHER viewer (a remote
// browser, a phone tab) takes the session's PTY size, the open pane must say
// so and take it back on request, and take it back by itself once that viewer
// leaves. Before the feed, the pane kept painting the other grid's output
// (stale bottom rows, broken TUI frames) until the session was restarted.
//
// The "other window" is a raw /ws/term socket from the test process.
// Desktop-browser project only.

let ctx: APIRequestContext;
let base: string;
let token: string;

interface Sock {
  grids: Array<{ cols: number; rows: number; owner: string }>;
  send: (frame: unknown) => void;
  close: () => void;
}

async function attach(id: string): Promise<Sock> {
  const ws = new WebSocket(`${base.replace('http', 'ws')}/ws/term/${id}?view=1`, ['otto-bearer', token]);
  const grids: Sock['grids'] = [];
  ws.onmessage = (ev) => {
    if (typeof ev.data !== 'string') return;
    const f = JSON.parse(ev.data) as { type?: string; cols?: number; rows?: number; owner?: string };
    if (f.type === 'grid') grids.push({ cols: f.cols ?? 0, rows: f.rows ?? 0, owner: f.owner ?? '' });
  };
  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve();
    ws.onerror = () => reject(new Error('ws error'));
  });
  return { grids, send: (f) => ws.send(JSON.stringify(f)), close: () => ws.close() };
}

const last = (s: Sock) => s.grids[s.grids.length - 1];

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  token = c.token;
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('another window sizing the PTY: the pane offers a take-over and reclaims it when that window leaves', async ({ page }) => {
  const wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'GridTakeover', cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = ((await r.json()) as { id: string }).id;

  await page.goto(`/#/agents/${id}`);
  const host = page.locator('.term-host');
  await expect(host).toBeVisible({ timeout: 30_000 });
  const cols = () => host.getAttribute('data-cols').then((v) => Number(v ?? '0'));
  await expect.poll(cols, { timeout: 15_000 }).toBeGreaterThan(80);
  const paneCols = await cols();

  // The other window: attaches (told the current grid), claims + resizes small.
  const remote = await attach(id);
  await expect.poll(() => remote.grids.length, { timeout: 10_000 }).toBeGreaterThan(0);
  remote.send({ type: 'claim', cols: 60, rows: 15 });
  await expect.poll(() => last(remote), { timeout: 10_000 }).toEqual({ cols: 60, rows: 15, owner: 'self' });

  const chip = page.locator('.term-overlay[data-foreign-grid="60x15"]');
  await expect(chip).toBeVisible({ timeout: 10_000 });
  await expect(chip).toContainText('Sized for another window');

  // One click takes the grid back at the pane's own size.
  await chip.getByRole('button', { name: 'Use this pane' }).click();
  await expect(chip).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => last(remote), { timeout: 10_000 }).toMatchObject({ cols: paneCols, owner: 'other' });

  // The other window takes it again, then closes: the visible pane takes the
  // grid back by itself (authority is sticky server-side; only a claim moves it).
  remote.send({ type: 'claim', cols: 70, rows: 18 });
  await expect(page.locator('.term-overlay[data-foreign-grid="70x18"]')).toBeVisible({ timeout: 10_000 });
  remote.close();
  await expect(page.locator('.term-overlay[data-foreign-grid]')).toHaveCount(0, { timeout: 10_000 });
  const probe = await attach(id);
  await expect.poll(() => last(probe), { timeout: 10_000 }).toMatchObject({ cols: paneCols, owner: 'other' });
  probe.close();
});
