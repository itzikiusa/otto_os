import { test, expect, type Page, type WebSocketRoute } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Several agent panes in one window (perf 01 N4 — guards F1 + F9).
//
// F1: an agent pane (preferDom) whose confirmed grid got WIDER asks the daemon
// for a full snapshot (`scrollback`, up to 4000 rows) to close the void a
// bottom-anchored TUI leaves after a widen. With N panes a window resize used
// to fire N of those at once; the window-wide CompactQueue (termCompactQueue.ts)
// now allows ONE in flight, and a pane that is off-screen (IntersectionObserver)
// or in a hidden window defers its compact until it comes back. A NARROWER grid
// never compacts (the TUI's own SIGWINCH redraw repaints it — resizeDecision).
// F9: in a hidden window only the pane the user last focused keeps acking its
// credit stream; the others keep parsing but withhold acks (the daemon stops
// after one window) and send ONE cumulative ack when the window shows again.
//
// The fixture (e2e/fixtures/terminal-panes.html) mounts p1–p3 side by side at
// the viewport width plus p4 below the fold inside a scroll box. A mocked
// `/ws/term/<sid>` (one per socket) grants credit, answers the attach snapshot
// at once and every later (compact) snapshot after COMPACT_REPLY_MS, so an
// in-flight compact is observable and the window-wide concurrency is counted.
// Widths are chosen so each pane fits ≥ 80 cols at the 13 px font (below that
// the auto-fit shrinks the FONT, and a widen may not add columns).
// ─────────────────────────────────────────────────────────────────────────────

test.setTimeout(90_000);

const IDS = ['p1', 'p2', 'p3', 'p4'] as const;
type Sid = (typeof IDS)[number];
const NARROW = { width: 2100, height: 900 };
const WIDE = { width: 2700, height: 900 };
/** Compact replies are held this long: the slot stays taken meanwhile. */
const COMPACT_REPLY_MS = 250;
/** Settle 200 + confirm 150 + RESIZE_COMPACT_MS 900, plus slack: no compact
 *  can still be on its way after this much quiet. */
const QUIET_MS = 2500;
const FRAME = 64 * 1024;
const LINE = 'z'.repeat(78) + '\r\n';
const OUT_FRAME = Buffer.from(LINE.repeat(Math.ceil(FRAME / LINE.length))).subarray(0, FRAME);
/** Hidden-window output per pane: 9 × 64 KB, under the 1 MB credit window,
 *  so the mock never holds anything back and every byte reaches the pane. */
const OUT_FRAMES = 9;

type Frame = { sid: Sid; type: string; t: number; bytes?: number; attach?: boolean };

function mockDaemon(page: Page) {
  const frames: Frame[] = [];
  const sockets = new Map<Sid, WebSocketRoute>();
  const attached = new Set<Sid>();
  /** Window-wide compact snapshots requested but not yet answered. */
  let inflight = 0;
  let maxInflight = 0;
  return {
    frames,
    get maxInflight() {
      return maxInflight;
    },
    resetMaxInflight() {
      maxInflight = inflight;
    },
    attachedCount: () => attached.size,
    /** Compact `scrollback` requests (every one after a socket's attach). */
    compacts: (sid?: Sid, from = 0) =>
      frames.slice(from).filter((f) => f.type === 'scrollback' && !f.attach && (!sid || f.sid === sid)).length,
    acks: (sid: Sid, from = 0) => frames.slice(from).filter((f) => f.type === 'ack' && f.sid === sid),
    /** PTY output to one pane (binary frames, inside its credit window). */
    output(sid: Sid, n: number) {
      const ws = sockets.get(sid);
      for (let i = 0; i < n; i++) ws?.send(OUT_FRAME);
    },
    install: () =>
      page.routeWebSocket(/\/ws\/term\//, (ws) => {
        const sid = (new URL(ws.url()).pathname.split('/').pop() ?? '') as Sid;
        sockets.set(sid, ws);
        let attachSeen = false;
        ws.onMessage((message) => {
          const frame = JSON.parse(String(message));
          const rec: Frame = { sid, type: frame.type, t: Date.now() };
          if (frame.type === 'ack') rec.bytes = frame.bytes;
          frames.push(rec);
          switch (frame.type) {
            case 'credit':
              ws.send(JSON.stringify({ type: 'credit', window: 1024 * 1024 }));
              break;
            case 'scrollback': {
              const snap = JSON.stringify({ type: 'scrollback', data: Buffer.from(`READY-${sid}$ `).toString('base64'), epoch: 1 });
              if (!attachSeen) {
                // The attach snapshot (one per socket, not queued — perf F1).
                attachSeen = true;
                rec.attach = true;
                attached.add(sid);
                ws.send(snap);
                break;
              }
              inflight++;
              maxInflight = Math.max(maxInflight, inflight);
              setTimeout(() => {
                inflight--;
                ws.send(snap);
              }, COMPACT_REPLY_MS);
              break;
            }
          }
        });
      }),
  };
}

async function open(page: Page, size: { width: number; height: number }) {
  const daemon = mockDaemon(page);
  await daemon.install();
  await page.setViewportSize(size);
  await page.addInitScript(() => {
    localStorage.setItem('otto_base', location.origin);
    localStorage.setItem('otto_token', 'fixture');
  });
  await page.route('**/api/v1/**', (route) => route.fulfill({ json: [] }));
  await page.goto('/e2e/fixtures/terminal-panes.html');
  await expect.poll(() => daemon.attachedCount(), { timeout: 20_000 }).toBe(IDS.length);
  // Attach-time forced resizes, verify-fit passes and font auto-fit settle.
  await page.waitForTimeout(QUIET_MS);
  return daemon;
}

/** Fake the window's visibility (another Space / minimized) in the page. */
async function setVisibility(page: Page, state: 'hidden' | 'visible'): Promise<void> {
  await page.evaluate((s) => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => s });
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => s === 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
  }, state);
}

test('widening the window compacts one pane at a time, and an off-screen pane only once it scrolls into view', async ({ page }) => {
  const daemon = await open(page, NARROW);
  const from = daemon.frames.length;
  daemon.resetMaxInflight();
  await page.setViewportSize(WIDE);

  const visible = ['p1', 'p2', 'p3'] as const;
  await expect
    .poll(() => visible.reduce((n, sid) => n + daemon.compacts(sid, from), 0), { timeout: 20_000 })
    .toBeGreaterThanOrEqual(1);
  // Let every queued compact drain (each waits for the previous reply).
  await page.waitForTimeout(QUIET_MS + visible.length * COMPACT_REPLY_MS);
  const perPane = Object.fromEntries(IDS.map((sid) => [sid, daemon.compacts(sid, from)]));
  console.log(`[compact-queue] widen: compacts ${JSON.stringify(perPane)}, max in flight ${daemon.maxInflight}`);
  expect(daemon.maxInflight, 'one compact in flight per window').toBeLessThanOrEqual(1);
  for (const sid of visible) expect(perPane[sid], `${sid}: at most one compact per widen`).toBeLessThanOrEqual(1);
  expect(perPane.p4, 'the off-screen pane defers its compact').toBe(0);

  // p4 comes into view: it runs the compact it skipped, exactly once.
  const before = daemon.frames.length;
  await page.locator('[data-pane="p4"]').scrollIntoViewIfNeeded();
  await expect.poll(() => daemon.compacts('p4', before), { timeout: 10_000 }).toBe(1);
  await page.waitForTimeout(QUIET_MS);
  expect(daemon.compacts('p4', before), 'p4 compacts once on wake').toBe(1);
  expect(daemon.compacts(undefined, before), 'nobody else compacts on the scroll').toBe(1);
  expect(daemon.maxInflight).toBeLessThanOrEqual(1);
});

test('narrowing the window never compacts', async ({ page }) => {
  const daemon = await open(page, WIDE);
  const from = daemon.frames.length;
  await page.setViewportSize(NARROW);
  // The narrower grid still reaches the PTY…
  await expect
    .poll(() => daemon.frames.slice(from).filter((f) => f.type === 'resize').length, { timeout: 10_000 })
    .toBeGreaterThanOrEqual(1);
  await page.waitForTimeout(QUIET_MS);
  // …but the TUI's SIGWINCH redraw repaints it: no snapshot.
  expect(daemon.compacts(undefined, from), 'a narrower grid sends no compact').toBe(0);
});

test('a hidden window: only the focused pane acks; the others send one cumulative ack on return', async ({ page }) => {
  const daemon = await open(page, NARROW);
  // p1 is the pane the user works in.
  await page.locator('[data-pane="p1"] .xterm-helper-textarea').focus();
  await page.waitForTimeout(500);
  await setVisibility(page, 'hidden');
  const from = daemon.frames.length;
  const total = OUT_FRAMES * FRAME;
  for (const sid of ['p1', 'p2', 'p3'] as const) daemon.output(sid, OUT_FRAMES);

  // The focused pane keeps streaming: it acks as xterm consumes, to the end.
  await expect
    .poll(() => Math.max(0, ...daemon.acks('p1', from).map((a) => a.bytes ?? 0)), { timeout: 20_000 })
    .toBe(total);
  // Same bytes, parsed at the same speed: give p2/p3 time to finish too.
  await page.waitForTimeout(1500);
  expect(daemon.acks('p2', from), 'p2 withholds acks while the window is hidden').toHaveLength(0);
  expect(daemon.acks('p3', from), 'p3 withholds acks while the window is hidden').toHaveLength(0);

  const back = daemon.frames.length;
  await setVisibility(page, 'visible');
  for (const sid of ['p2', 'p3'] as const) {
    await expect.poll(() => daemon.acks(sid, back).length, { timeout: 10_000 }).toBeGreaterThanOrEqual(1);
  }
  await page.waitForTimeout(1000);
  for (const sid of ['p2', 'p3'] as const) {
    const acks = daemon.acks(sid, back);
    console.log(`[compact-queue] ${sid} acks on return: ${JSON.stringify(acks.map((a) => a.bytes))}`);
    expect(acks, `${sid}: ONE cumulative ack on return`).toHaveLength(1);
    expect(acks[0].bytes, `${sid}: the ack covers everything consumed while hidden`).toBe(total);
    expect(
      daemon.frames.slice(from).filter((f) => f.sid === sid && f.type === 'scrollback'),
      `${sid}: nothing overflowed, so no snapshot is requested`,
    ).toHaveLength(0);
  }
});
