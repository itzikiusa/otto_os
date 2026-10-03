import { test, expect, type Page, type WebSocketRoute } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Garbled agent pane after a tab switch (review 01 G1).
//
// A full-screen TUI's screen only matches the daemon while the local xterm
// grid equals the PTY grid. Passing sizes — a layout settling on leave or
// return — used to resize xterm right away (reflowing / truncating the alt
// buffer) and, when the box settled back to the old size, nothing repainted:
// no SIGWINCH for an unchanged grid, no snapshot. Agent panes now MEASURE
// while the box settles and resize xterm only once the grid is confirmed,
// park at the PTY grid, compact after any local reflow, and offer a manual
// "Redraw terminal" (a fresh `scrollback` snapshot).
//
// The fixture mounts one agent Terminal (preferDom) against a mocked
// `/ws/term` that serves a cursor-addressed alternate-screen frame. The
// screen is read through `window.__ottoTermProbe` (renderer-agnostic).
// ─────────────────────────────────────────────────────────────────────────────

test.setTimeout(60_000);

type Probe = { text: () => string; disposed: boolean };

const ROWS = Array.from({ length: 10 }, (_, i) => `ROW-${String(i).padStart(2, '0')} ${'#'.repeat(40)}`);
/** A TUI frame: alt screen, every row placed by cursor addressing. */
function tuiFrame(tag: string): string {
  return '\x1b[?1049h\x1b[2J' + ROWS.map((r, i) => `\x1b[${i + 2};3H${tag}:${r}`).join('') + `\x1b[1;1H${tag}$ `;
}
/** Rows 2..11 as the probe reads them. */
function expectedRows(tag: string): string[] {
  return ROWS.map((r) => `  ${tag}:${r}`);
}

function screen(page: Page): Promise<string> {
  return page.evaluate(() => {
    const live = (window as unknown as { __ottoTermProbe?: Probe[] }).__ottoTermProbe?.filter((p) => !p.disposed) ?? [];
    return live.length ? live[live.length - 1].text() : '';
  });
}

async function tuiRows(page: Page): Promise<string[]> {
  return (await screen(page)).split('\n').slice(1, 1 + ROWS.length);
}

/** The daemon's side of `/ws/term`: every `scrollback` gets the current frame. */
function mockDaemon(page: Page) {
  const stats = { sockets: 0, snapshots: 0, resizes: [] as { cols: number; rows: number }[] };
  let tag = 'F1';
  let live: WebSocketRoute | null = null;
  return {
    stats,
    setFrame(next: string) {
      tag = next;
    },
    /** Raw PTY bytes to the live socket (e.g. to garble the screen). */
    send(bytes: string) {
      live?.send(Buffer.from(bytes));
    },
    install: () =>
      page.routeWebSocket(/\/ws\/term\//, (ws) => {
        stats.sockets++;
        live = ws;
        ws.onMessage((message) => {
          const frame = JSON.parse(String(message));
          if (frame.type === 'scrollback') {
            stats.snapshots++;
            ws.send(JSON.stringify({ type: 'scrollback', data: Buffer.from(tuiFrame(tag)).toString('base64'), epoch: 1 }));
          } else if (frame.type === 'resize') {
            stats.resizes.push({ cols: frame.cols, rows: frame.rows });
          }
        });
      }),
  };
}

async function open(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('otto_base', location.origin);
    localStorage.setItem('otto_token', 'fixture');
    (window as unknown as { __ottoTermProbe: unknown[] }).__ottoTermProbe = [];
  });
  await page.route('**/api/v1/**', (route) => route.fulfill({ json: [] }));
  await page.goto('/e2e/fixtures/terminal-links.html?keepalive');
  await expect.poll(() => tuiRows(page), { timeout: 20_000 }).toEqual(expectedRows('F1'));
}

/** Squash the pane's height for about one layout pass, then restore it —
 *  the "passing size" a leaving/returning layout measures. */
async function passingSize(page: Page, holdMs: number): Promise<void> {
  await page.evaluate(async (ms) => {
    const layout = document.querySelector<HTMLElement>('.layout')!;
    layout.style.height = '120px';
    await new Promise((r) => setTimeout(r, ms));
    layout.style.height = '';
  }, holdMs);
}

test('a passing pane size never reflows the TUI screen or reaches the PTY', async ({ page }) => {
  const daemon = mockDaemon(page);
  await daemon.install();
  await open(page);
  // Let the attach resize settle (settle 200 ms + confirm 150 ms).
  await page.waitForTimeout(800);
  const settled = daemon.stats.resizes.at(-1);
  expect(settled, 'the attach pushed its grid').toBeTruthy();
  // Longer than the RO debounce (90 ms), shorter than settle + confirm.
  await passingSize(page, 160);
  await page.waitForTimeout(1500);
  expect(daemon.stats.resizes.filter((r) => r.rows < settled!.rows), 'no passing grid sent').toEqual([]);
  expect(await tuiRows(page), 'every TUI row still where the daemon drew it').toEqual(expectedRows('F1'));
});

test('switching away and back with a passing size on return keeps the parked TUI intact', async ({ page }) => {
  const daemon = mockDaemon(page);
  await daemon.install();
  await open(page);
  await page.waitForTimeout(800);
  const settled = daemon.stats.resizes.at(-1)!;
  const toggle = page.getByRole('button', { name: 'Toggle terminal' });
  await toggle.click(); // park
  await expect.poll(() => screen(page)).toBe('');
  // Return while the layout is still squashed for a moment.
  await page.evaluate(() => {
    document.querySelector<HTMLElement>('.layout')!.style.height = '120px';
  });
  await toggle.click(); // adopt
  await page.evaluate(
    () =>
      new Promise<void>((r) =>
        requestAnimationFrame(() =>
          setTimeout(() => {
            document.querySelector<HTMLElement>('.layout')!.style.height = '';
            r();
          }, 60),
        ),
      ),
  );
  await page.waitForTimeout(1500);
  expect(daemon.stats.sockets, 'the parked engine was adopted, not reconnected').toBe(1);
  expect(daemon.stats.resizes.filter((r) => r.rows < settled.rows), 'no passing grid sent').toEqual([]);
  expect(await tuiRows(page)).toEqual(expectedRows('F1'));
});

test('Redraw terminal rebuilds a garbled screen from a fresh snapshot', async ({ page }) => {
  const daemon = mockDaemon(page);
  await daemon.install();
  await open(page);
  daemon.send('\x1b[4;3HGARBLED\x1b[8;1H\x1b[2K');
  await expect.poll(async () => (await tuiRows(page))[2]).toContain('GARBLED');
  const before = daemon.stats.snapshots;
  daemon.setFrame('F2');
  await page.getByRole('button', { name: 'Redraw terminal' }).click();
  await expect.poll(() => daemon.stats.snapshots).toBe(before + 1);
  await expect.poll(() => tuiRows(page)).toEqual(expectedRows('F2'));
  expect(daemon.stats.sockets, 'redraw keeps the socket').toBe(1);
});
