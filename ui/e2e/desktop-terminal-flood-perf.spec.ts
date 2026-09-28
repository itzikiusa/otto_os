import { test, expect, type APIRequestContext, type Page, type WebSocketRoute } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { isWebkitProject } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Terminal flood regression gates (GAPS_TO_9_5 §4, I3 A3/A4/A8).
//
// A mocked `/ws/term` (Playwright routeWebSocket) plays the daemon's side of
// the flow-control contract (docs/contracts/ws.md §1): `scrollback` → snapshot,
// `credit`/`ack` (a window of unacknowledged bytes, one held window, then a
// skip to ONE snapshot — the same rules as ws.rs `CreditGate`), the legacy
// `pause`/`resume` (+ one snapshot when output was held back) for a daemon
// that does not grant credit, `resync` → drop the queue + ONE snapshot. It
// floods the Terminal fixture and asserts:
//   • with credit, the backlog (flow.pending) never tops 2.5 MB at 2 AND 8
//     frames per tick — bounded by the window, not by the send rate;
//   • without credit (older daemon), the pause/resume fallback still works;
//   • a full 20 MB flood ends on the final line, with only the attach
//     `scrollback` request (no redundant rebuild requests);
//   • ^C mid-flood is on screen in < 150 ms (A3: queue dropped + `resync`);
//   • a 15-tile TiledView keeps ≤ 2k scrollback per tile, the maximized tile
//     10k (A4).
// Numbers come from an in-page probe (`window.__ottoTermProbe`, installed
// before load; Terminal.svelte registers into it only when present). The
// screen is read through the probe too (the parsed buffer, `onRender` for
// "painted"): terminals render on WebGL where available (3027df89), so there
// is no `.xterm-rows` DOM to read — the old DOM reads timed out on WebKit
// although the flood rendered fine.
// ─────────────────────────────────────────────────────────────────────────────

test.setTimeout(120_000);
// WebKit: the app's engine (xterm parses ~5.5–8 MB/s there, termFlow.ts).
// WebKit budgets: runs in the `desktop-webkit` project (engine from the project).

const MB = 1024 * 1024;
const FRAME = 64 * 1024;
const LINE = 'y'.repeat(78) + '\r\n';
const FLOOD_FRAME = Buffer.from(LINE.repeat(Math.floor(FRAME / LINE.length)));
/** Frames per mock tick. One 64 KB frame per macrotask is ~13 MB/s through
 *  Playwright's WS route, which xterm kept up with (peak backlog 0.4–0.9 MB,
 *  never FLOW_HIGH), so nothing paused; a PTY `cat` outruns the parser.
 *  Pause-mode peak backlog by BURST (WebKit, merge-2): 2 → 3.0–4.4 MB,
 *  4 → 9.7–10.8 MB, 8 → 14.1–14.6 MB — `pause` stops the producer, but bytes
 *  already sent keep landing, so the overshoot scaled with the send rate.
 *  Credit mode bounds it by the window at any BURST. */
const BURST = 2;

/** ws.rs `CreditGate`, byte for byte: ≤ `window` unacknowledged bytes out,
 *  ≤ one more window held, then skip → ONE snapshot at ≤ window/4 unacked. */
class MockCredit {
  sent = 0;
  acked = 0;
  held: Buffer[] = [];
  heldBytes = 0;
  skipped = false;
  constructor(readonly window: number) {}
  private avail(): number {
    return Math.max(0, this.window - (this.sent - this.acked));
  }
  private take(): Buffer[] {
    const out: Buffer[] = [];
    while (this.heldBytes && this.avail() > 0) {
      const head = this.held[0];
      const n = Math.min(this.avail(), head.byteLength);
      if (n === head.byteLength) this.held.shift();
      else this.held[0] = head.subarray(n);
      out.push(head.subarray(0, n));
      this.sent += n;
      this.heldBytes -= n;
    }
    return out;
  }
  push(b: Buffer): Buffer[] {
    if (this.skipped) return [];
    this.held.push(b);
    this.heldBytes += b.byteLength;
    if (this.heldBytes > this.window) {
      this.superseded();
      this.skipped = true;
      return [];
    }
    return this.take();
  }
  ack(cumulative: number): Buffer[] | 'resync' {
    this.acked = Math.max(this.acked, Math.min(cumulative, this.sent));
    if (this.skipped) {
      if (this.sent - this.acked > this.window / 4) return [];
      this.skipped = false;
      return 'resync';
    }
    return this.take();
  }
  skipOnInput(): void {
    if (this.heldBytes) {
      this.superseded();
      this.skipped = true;
    }
  }
  superseded(): void {
    this.held = [];
    this.heldBytes = 0;
    this.skipped = false;
  }
}

type Probe = {
  sessionId: () => string;
  pending: () => number;
  queued: () => number;
  scrollback: () => number;
  renderer: () => 'webgl' | 'dom';
  text: () => string;
  onRender: (cb: () => void) => void;
  disposed: boolean;
};

/** The live terminal's viewport text (renderer-agnostic). */
function screen(page: Page): Promise<string> {
  return page.evaluate(() => {
    const live = (window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe?.filter((p) => !p.disposed) ?? [];
    return live.length ? live[live.length - 1].text() : '';
  });
}

function renderer(page: Page): Promise<string> {
  return page.evaluate(() => {
    const live = (window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe?.filter((p) => !p.disposed) ?? [];
    return live.length ? live[live.length - 1].renderer() : 'none';
  });
}

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
  await expect.poll(() => screen(page), { timeout: 20_000 }).toContain('READY$');
}

/** The daemon's side of `/ws/term` for one flood. `credit: false` plays an
 *  older daemon that ignores `credit` (the client stays in pause mode). */
function mockDaemon(page: Page, opts: { totalBytes: number; onCtrlC?: string; burst?: number; credit?: boolean }) {
  const burst = opts.burst ?? BURST;
  const stats = { clientFrames: [] as string[], sent: 0, snapshots: 0, creditResyncs: 0, interruptedAt: 0, floodFrom: -1, snapshotsBeforeCtrlC: 0 };
  let screenTail = 'READY$ ';
  let paused = false;
  let skipped = false;
  let interrupted = false;
  let flooding = false;
  let credit: MockCredit | null = null;
  const snapshot = (ws: WebSocketRoute): void => {
    stats.snapshots++;
    credit?.superseded();
    ws.send(JSON.stringify({ type: 'scrollback', data: Buffer.from(screenTail).toString('base64'), epoch: 1 }));
  };
  /** PTY output for this viewer: through the credit gate when granted. */
  const output = (ws: WebSocketRoute, b: Buffer): void => {
    if (credit) for (const out of credit.push(b)) ws.send(out);
    else if (!paused) ws.send(b);
    else skipped = true;
  };
  const flood = async (ws: WebSocketRoute): Promise<void> => {
    if (flooding) return;
    flooding = true;
    stats.floodFrom = stats.clientFrames.length;
    while (stats.sent < opts.totalBytes && !interrupted) {
      if (credit) {
        // The PTY runs at its own rate; the gate decides what this viewer gets.
        for (let i = 0; i < burst && stats.sent < opts.totalBytes; i++) {
          output(ws, FLOOD_FRAME);
          stats.sent += FLOOD_FRAME.byteLength;
        }
      } else if (paused) {
        // A paused viewer's output is held back (and dropped by the ring).
        skipped = true;
        stats.sent += FRAME;
      } else {
        for (let i = 0; i < burst && !paused && stats.sent < opts.totalBytes; i++) {
          ws.send(FLOOD_FRAME);
          stats.sent += FLOOD_FRAME.byteLength;
        }
      }
      screenTail = LINE.repeat(3);
      await new Promise((r) => setTimeout(r, 0));
    }
    if (!interrupted) {
      screenTail = `${LINE}FLOOD-END$ `;
      output(ws, Buffer.from('FLOOD-END$ '));
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
              // Start once the attach has settled: READY$ painted AND the
              // one post-attach resize compaction (Terminal.svelte: confirm
              // 150 ms + RESIZE_COMPACT_MS 900 ms → a 2nd `scrollback`) is
              // done, so the flood's own rebuild requests can be counted.
              setTimeout(() => void flood(ws), 1500);
              break;
            case 'credit':
              if (opts.credit === false) break; // an older daemon: unknown frame
              credit = new MockCredit(Math.min(Math.max(frame.window || 1024 * 1024, 64 * 1024), 8 * 1024 * 1024));
              ws.send(JSON.stringify({ type: 'credit', window: credit.window }));
              break;
            case 'ack': {
              if (!credit) break;
              const step = credit.ack(frame.bytes);
              if (step === 'resync') {
                stats.creditResyncs++;
                snapshot(ws);
              } else for (const out of step) ws.send(out);
              break;
            }
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
                stats.snapshotsBeforeCtrlC = stats.snapshots;
                screenTail = `${LINE}^C\r\n${opts.onCtrlC}`;
                credit?.skipOnInput();
                output(ws, Buffer.from(`^C\r\n${opts.onCtrlC}`));
              }
              break;
            }
          }
        });
      }),
  };
}

test.beforeEach(async ({}, info) => {
  test.skip(!isWebkitProject(info.project.name), 'WebKit perf gate: --project=desktop-webkit');
});

for (const burst of [2, 8]) {
  test(`a 20 MB flood at ${burst} frames/tick keeps the backlog ≤ 2.5 MB (credit) and ends on the last line`, async ({ page }) => {
    await installProbe(page);
    const daemon = mockDaemon(page, { totalBytes: 20 * MB, burst });
    await daemon.install();
    await fixturePage(page);
    await expect.poll(() => screen(page), { timeout: 60_000 }).toContain('FLOOD-END$');
    const f = daemon.stats.clientFrames;
    const peak = await page.evaluate(() => (window as unknown as { __ottoMaxPending: number }).__ottoMaxPending);
    const acks = f.filter((t) => t === 'ack').length;
    console.log(
      `[flood] credit burst=${burst} 20MB (${await renderer(page)}): peak backlog ${(peak / MB).toFixed(2)} MB, acks ${acks}, ` +
        `skip-resyncs ${daemon.stats.creditResyncs}, snapshots ${daemon.stats.snapshots}, ` +
        `frames ${JSON.stringify(f.filter((t) => t !== 'ack'))}`,
    );
    expect(f[0], 'credit is offered first thing on the socket').toBe('credit');
    expect(acks, 'the client acknowledges as xterm consumes').toBeGreaterThan(0);
    expect(f.filter((t) => t === 'pause'), 'credit mode never pauses').toHaveLength(0);
    expect(f.slice(daemon.stats.floodFrom).filter((t) => t === 'scrollback'), 'the flood requests no rebuild').toHaveLength(0);
    expect(f.filter((t) => t === 'scrollback').length, 'attach + at most the resize compaction').toBeLessThanOrEqual(2);
    expect(f.filter((t) => t === 'resync'), 'no input → no resync').toHaveLength(0);
    expect(peak, `peak backlog ${(peak / MB).toFixed(2)} MB`).toBeLessThanOrEqual(2.5 * MB);
    // Every snapshot is a requested one or one skip-resync, and a skip needs
    // two whole windows of output behind it.
    expect(daemon.stats.snapshots).toBeLessThanOrEqual(f.filter((t) => t === 'scrollback').length + daemon.stats.creditResyncs);
    expect(daemon.stats.creditResyncs).toBeLessThanOrEqual(10);
  });
}

test('an older daemon (no credit grant): the pause/resume fallback still pauses and ends on the last line', async ({ page }) => {
  await installProbe(page);
  const daemon = mockDaemon(page, { totalBytes: 20 * MB, credit: false });
  await daemon.install();
  await fixturePage(page);
  await expect.poll(() => screen(page), { timeout: 60_000 }).toContain('FLOOD-END$');
  const f = daemon.stats.clientFrames;
  const peak = await page.evaluate(() => (window as unknown as { __ottoMaxPending: number }).__ottoMaxPending);
  console.log(`[flood] legacy 20MB (${await renderer(page)}): peak backlog ${(peak / MB).toFixed(2)} MB, frames ${JSON.stringify(f)}, snapshots ${daemon.stats.snapshots}`);
  expect(f.filter((t) => t === 'ack'), 'no grant → no acks').toHaveLength(0);
  expect(f.filter((t) => t === 'pause').length, 'the client asked the daemon to pause').toBeGreaterThan(0);
  expect(f.slice(daemon.stats.floodFrom).filter((t) => t === 'scrollback'), 'the flood requests no rebuild').toHaveLength(0);
  const resumes = f.filter((t) => t === 'resume').length;
  expect(daemon.stats.snapshots).toBeLessThanOrEqual(f.filter((t) => t === 'scrollback').length + resumes);
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
  // Measure in the page: keydown → the first renderer pass (DOM or WebGL)
  // after which the marker is on screen.
  await page.evaluate(() => {
    const w = window as unknown as { __ctrlC?: { down: number; seen: number }; __ottoTermProbe: Probe[] };
    w.__ctrlC = { down: 0, seen: 0 };
    document.addEventListener('keydown', (e) => {
      if (e.ctrlKey && e.key === 'c' && !w.__ctrlC!.down) w.__ctrlC!.down = performance.now();
    }, true);
    const probe = w.__ottoTermProbe.filter((p) => !p.disposed).at(-1)!;
    probe.onRender(() => {
      if (w.__ctrlC!.down && !w.__ctrlC!.seen && probe.text().includes('INTERRUPTED$')) w.__ctrlC!.seen = performance.now();
    });
  });
  await page.keyboard.press('Control+c');
  await expect.poll(() => screen(page), { timeout: 10_000 }).toContain('INTERRUPTED$');
  await expect.poll(() => page.evaluate(() => (window as unknown as { __ctrlC: { seen: number } }).__ctrlC.seen)).toBeGreaterThan(0);
  const t = await page.evaluate(() => (window as unknown as { __ctrlC: { down: number; seen: number } }).__ctrlC);
  const peak = await page.evaluate(() => (window as unknown as { __ottoMaxPending: number }).__ottoMaxPending);
  const ms = t.seen - t.down;
  console.log(
    `[flood] ^C (${await renderer(page)}): ${ms.toFixed(0)} ms, peak backlog ${(peak / MB).toFixed(2)} MB, ` +
      `frames ${JSON.stringify(daemon.stats.clientFrames.filter((x) => x !== 'ack'))}, snapshots ${daemon.stats.snapshots}`,
  );
  expect(ms, `^C visible after ${ms.toFixed(0)} ms`).toBeLessThan(150);
  expect(daemon.stats.clientFrames.filter((x) => x === 'resync').length).toBeLessThanOrEqual(1);
  expect(daemon.stats.clientFrames[0], 'credit mode').toBe('credit');
  // The ^C itself may cost one resync + one skip-resync rebuild.
  expect(daemon.stats.snapshots - daemon.stats.snapshotsBeforeCtrlC, 'at most one resync/skip rebuild after ^C').toBeLessThanOrEqual(2);
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

    // 15 tiles are narrow: the header folds Zoom into its ⋯ menu (tier ≥ 5).
    const tile = page.locator('[data-tile-id]').first();
    const zoom = tile.locator('button[title="Zoom in on this session"]');
    if (await zoom.isVisible()) await zoom.click();
    else {
      await tile.locator('button[title="More…"]').click();
      await page.getByRole('menuitem', { name: 'Zoom in on this session' }).click();
    }
    await expect(page.locator('.tiled.single .pane')).toHaveCount(1);
    await expect.poll(async () => (await live()).includes(10_000), { timeout: 15_000 }).toBe(true);
  });
});
