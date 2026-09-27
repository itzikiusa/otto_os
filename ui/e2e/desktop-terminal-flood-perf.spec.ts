import { test, expect, type APIRequestContext, type Page, type WebSocketRoute } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Terminal flood regression gates (GAPS_TO_9_5 §4, I3 A3/A4/A8).
//
// A mocked `/ws/term` (Playwright routeWebSocket) plays the daemon's side of
// the flow-control contract (docs/contracts/ws.md §1): `scrollback` → snapshot,
// `pause`/`resume` (+ one snapshot when output was held back), `resync` → drop
// the queue + ONE snapshot. It floods the Terminal fixture and asserts:
//   • the client pauses and its backlog (flow.pending) never tops 2.5 MB;
//   • a full 20 MB flood ends on the final line, with only the attach
//     `scrollback` request (no redundant rebuild requests);
//   • ^C mid-flood is on screen in < 150 ms (A3: queue dropped + `resync`);
//   • a 15-tile TiledView keeps ≤ 2k scrollback per tile, the maximized tile
//     10k (A4).
// Numbers come from an in-page probe (`window.__ottoTermProbe`, installed
// before load; Terminal.svelte registers into it only when present).
// ─────────────────────────────────────────────────────────────────────────────

test.setTimeout(120_000);

const MB = 1024 * 1024;
const FRAME = 64 * 1024;
const LINE = 'y'.repeat(78) + '\r\n';
const FLOOD_FRAME = Buffer.from(LINE.repeat(Math.floor(FRAME / LINE.length)));

type Probe = { sessionId: () => string; pending: () => number; queued: () => number; scrollback: () => number; disposed: boolean };

async function installProbe(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const w = window as unknown as { __ottoTermProbe: Probe[]; __ottoMaxPending: number };
    w.__ottoTermProbe = [];
    w.__ottoMaxPending = 0;
    // Sample the backlog every frame AND on every WS message tick.
    const sample = (): void => {
      for (const p of w.__ottoTermProbe) if (!p.disposed) w.__ottoMaxPending = Math.max(w.__ottoMaxPending, p.pending());
    };
    const loop = (): void => {
      sample();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
    setInterval(sample, 5);
  });
}

async function fixturePage(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('otto_base', location.origin);
    localStorage.setItem('otto_token', 'fixture');
  });
  await page.route('**/api/v1/**', (route) => route.fulfill({ json: [] }));
  await page.goto('/e2e/fixtures/terminal-links.html');
  await expect(page.locator('.xterm-rows')).toContainText('READY$', { timeout: 20_000 });
}

/** The daemon's side of `/ws/term` for one flood. */
function mockDaemon(page: Page, opts: { totalBytes: number; onCtrlC?: string }) {
  const stats = { clientFrames: [] as string[], sent: 0, snapshots: 0, interruptedAt: 0 };
  let screenTail = 'READY$ ';
  let paused = false;
  let skipped = false;
  let interrupted = false;
  let flooding = false;
  const snapshot = (ws: WebSocketRoute): void => {
    stats.snapshots++;
    ws.send(JSON.stringify({ type: 'scrollback', data: Buffer.from(screenTail).toString('base64'), epoch: 1 }));
  };
  const flood = async (ws: WebSocketRoute): Promise<void> => {
    if (flooding) return;
    flooding = true;
    while (stats.sent < opts.totalBytes && !interrupted) {
      if (paused) {
        // A paused viewer's output is held back (and dropped by the ring).
        skipped = true;
        stats.sent += FRAME;
      } else {
        ws.send(FLOOD_FRAME);
        stats.sent += FLOOD_FRAME.byteLength;
      }
      screenTail = LINE.repeat(3);
      await new Promise((r) => setTimeout(r, 0));
    }
    if (!interrupted) {
      screenTail = `${LINE}FLOOD-END$ `;
      if (!paused) ws.send(Buffer.from('FLOOD-END$ '));
      else skipped = true;
    }
  };
  return {
    stats,
    install: () =>
      page.routeWebSocket(/\/ws\/term\//, (ws) => {
        ws.onMessage((message) => {
          const frame = JSON.parse(String(message));
          stats.clientFrames.push(frame.type);
          switch (frame.type) {
            case 'scrollback':
              snapshot(ws);
              // Start once the attach snapshot has painted (READY$ visible).
              setTimeout(() => void flood(ws), 500);
              break;
            case 'pause':
              paused = true;
              break;
            case 'resume':
              if (paused && skipped) snapshot(ws);
              paused = false;
              skipped = false;
              break;
            case 'resync':
              // Drop this viewer's queue, leave the pause, ONE snapshot.
              paused = false;
              skipped = false;
              snapshot(ws);
              break;
            case 'input': {
              const text = Buffer.from(frame.data, 'base64').toString('latin1');
              if (text.includes('\x03') && opts.onCtrlC) {
                interrupted = true;
                stats.interruptedAt = Date.now();
                screenTail = `${LINE}^C\r\n${opts.onCtrlC}`;
                if (!paused) ws.send(Buffer.from(`^C\r\n${opts.onCtrlC}`));
                else skipped = true;
              }
              break;
            }
          }
        });
      }),
  };
}

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('a 20 MB flood pauses, keeps the backlog ≤ 2.5 MB, and ends on the last line', async ({ page }) => {
  await installProbe(page);
  const daemon = mockDaemon(page, { totalBytes: 20 * MB });
  await daemon.install();
  await fixturePage(page);
  await expect(page.locator('.xterm-rows')).toContainText('FLOOD-END$', { timeout: 60_000 });
  const f = daemon.stats.clientFrames;
  expect(f.filter((t) => t === 'pause').length, 'the client asked the daemon to pause').toBeGreaterThan(0);
  expect(f.filter((t) => t === 'scrollback'), 'only the attach snapshot is requested').toHaveLength(1);
  expect(f.filter((t) => t === 'resync'), 'no input → no resync').toHaveLength(0);
  const maxPending = await page.evaluate(() => (window as unknown as { __ottoMaxPending: number }).__ottoMaxPending);
  expect(maxPending, `peak backlog ${(maxPending / MB).toFixed(2)} MB`).toBeLessThanOrEqual(2.5 * MB);
  // Every held-back window was replaced by exactly one snapshot, never more.
  const resumes = f.filter((t) => t === 'resume').length;
  expect(daemon.stats.snapshots).toBeLessThanOrEqual(1 + resumes);
});

test('^C mid-flood shows up in under 150 ms (queue dropped, one resync)', async ({ page }) => {
  await installProbe(page);
  const daemon = mockDaemon(page, { totalBytes: 400 * MB, onCtrlC: 'INTERRUPTED$ ' });
  await daemon.install();
  await fixturePage(page);
  // Let the flood build a real backlog first.
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __ottoMaxPending: number }).__ottoMaxPending), { timeout: 20_000 })
    .toBeGreaterThan(512 * 1024);
  await page.locator('.xterm-helper-textarea').focus();
  // Measure in the page: keydown → marker painted into the rows.
  await page.evaluate(() => {
    const w = window as unknown as { __ctrlC?: { down: number; seen: number } };
    w.__ctrlC = { down: 0, seen: 0 };
    document.addEventListener('keydown', (e) => {
      if (e.ctrlKey && e.key === 'c' && !w.__ctrlC!.down) w.__ctrlC!.down = performance.now();
    }, true);
    const rows = document.querySelector('.xterm-rows')!;
    new MutationObserver(() => {
      if (!w.__ctrlC!.seen && rows.textContent?.includes('INTERRUPTED$')) w.__ctrlC!.seen = performance.now();
    }).observe(rows, { childList: true, subtree: true, characterData: true });
  });
  await page.keyboard.press('Control+c');
  await expect(page.locator('.xterm-rows')).toContainText('INTERRUPTED$', { timeout: 10_000 });
  const t = await page.evaluate(() => (window as unknown as { __ctrlC: { down: number; seen: number } }).__ctrlC);
  const ms = t.seen - t.down;
  expect(ms, `^C visible after ${ms.toFixed(0)} ms`).toBeLessThan(150);
  expect(daemon.stats.clientFrames.filter((x) => x === 'resync').length).toBeLessThanOrEqual(1);
  expect(daemon.stats.snapshots, 'attach + at most one resync/resume rebuild').toBeLessThanOrEqual(3);
});

test.describe('tiled scrollback budget', () => {
  let ctx: APIRequestContext;
  let base = '';
  let wsId = '';

  test.afterEach(async () => {
    await ctx?.dispose();
  });

  test('15 live tiles keep ≤ 2k scrollback each; the maximized tile gets 10k', async ({ page }) => {
    const c = await apiCtx();
    ctx = c.ctx;
    base = c.base;
    wsId = await seedWorkspace(ctx, base);
    const titles = Array.from({ length: 15 }, (_, i) => `Tile${String(i + 1).padStart(2, '0')}`);
    for (const title of titles) {
      const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
        data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
      });
      if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    }
    await installProbe(page);
    await page.addInitScript((id) => {
      localStorage.setItem('otto_workspace', id as string);
      localStorage.setItem('otto_firstrun_dismissed', '1');
      localStorage.setItem('otto_nav_all_ws', '0');
    }, wsId);
    await page.setViewportSize({ width: 1800, height: 1200 });
    await page.goto('/#/agents');
    await page.locator('.nav-item.nested-item', { hasText: titles[0] }).first().click();
    await expect(page.locator('[data-pane-key]').first()).toBeVisible({ timeout: 20_000 });
    await page.locator('button[aria-label="Tiled view"]').click();
    await expect(page.locator('[data-tile-id]')).toHaveCount(15, { timeout: 30_000 });
    const live = () =>
      page.evaluate(() =>
        (window as unknown as { __ottoTermProbe: Probe[] }).__ottoTermProbe
          .filter((p) => !p.disposed)
          .map((p) => p.scrollback()),
      );
    await expect.poll(async () => (await live()).length, { timeout: 30_000 }).toBeGreaterThan(1);
    const depths = await live();
    expect(depths.every((d) => d <= 2000), `tile depths ${depths.join(',')}`).toBe(true);

    await page.locator('[data-tile-id]').first().locator('button[title="Zoom in on this session"]').click();
    await expect(page.locator('.tiled.single .pane')).toHaveCount(1);
    await expect.poll(async () => (await live()).includes(10_000), { timeout: 15_000 }).toBe(true);
  });
});
