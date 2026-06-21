import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { expectNoHorizontalOverflow, expectContentHasHeight } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// FULL mobile sweep for SESSIONS (Agents) — portrait AND landscape.
//
// Runs against every viewport project the config defines:
//   iphone-portrait (430×932)   iphone-landscape (932×430)
//   ipad-portrait   (834×1194)  ipad-landscape  (1194×834 → desktop chrome)
//   iphone-se       (375×667)
//
// Phone-mode is the LIVE viewport width (≤640px), NOT the device name: an iPhone
// in landscape is 932px wide → the app treats it as a "tablet" (desktop chrome).
// So phone-only affordances (key bar, MobileActionBar, floating term controls,
// DOM renderer) are gated on the real width via liveIsPhone(), mirroring
// terminal-mobile.spec.ts.
//
// Covers the user journey: open a plain shell, open a claude/codex agent, write a
// command + see output, switch sessions/tabs, tiled view, the New Session sheet,
// and the phone drawers — asserting at each step: no horizontal overflow, the
// terminal/content panes actually have height (no "black void"), and the controls
// are on-screen and tappable.
// ─────────────────────────────────────────────────────────────────────────────

let workspaceId = '';
let shellA = '';
let shellB = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  shellA = await seedShellSession(ctx, base, workspaceId);
  shellB = await seedShellSession(ctx, base, workspaceId);
  // Real output so the terminal has something to render + scroll on every device.
  await ctx.post(`${base}/api/v1/sessions/${shellA}/input`, {
    data: { text: 'for i in $(seq 1 60); do echo "OTTO-DATA-$i"; done', submit: true },
  });
  await new Promise((r) => setTimeout(r, 1200));
  await ctx.dispose();
});

// Activate the seeded workspace + collapse the left rail so it doesn't cover the
// terminal in portrait. Tabbed view (default) so a single pane is on screen.
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspaceId);
});

async function liveIsPhone(page: Page): Promise<boolean> {
  return page.evaluate(() => window.matchMedia('(max-width: 640px)').matches);
}

/** The New Session sheet's primary button. Scoped to the sheet + exact name so it
 *  never collides with the pane header's "Restart session" control (which a loose
 *  /start session/i regex would also match). */
function startSessionBtn(page: Page) {
  return page.locator('.sheet[role="dialog"]').getByRole('button', { name: 'Start Session', exact: true });
}

/** Poll a locator's bounding box until it STOPS MOVING (two consecutive samples
 *  identical) — used after opening a drawer/sheet so on-screen assertions measure
 *  the settled position, not a frame mid slide-in animation. Stability (not "x≥0")
 *  is required because a right-edge drawer animates from x>0 inward, so an x-only
 *  check would pass on the very first, still-animating frame. */
async function settledBox(locator: ReturnType<Page['locator']>) {
  let prev = '';
  await expect
    .poll(
      async () => {
        const b = await locator.boundingBox();
        const key = b ? `${Math.round(b.x)},${Math.round(b.y)},${Math.round(b.width)},${Math.round(b.height)}` : '';
        const stable = key !== '' && key === prev;
        prev = key;
        return stable;
      },
      { intervals: [80, 80, 120, 160, 200, 300], timeout: 4000 },
    )
    .toBe(true);
  return locator.boundingBox();
}

/** Read a session's terminal scrollback over a WebSocket in the PAGE context
 *  (same origin + stored token as the app). This is the RENDERER-AGNOSTIC way to
 *  verify "the data is in this session + viewable": on tablet/desktop xterm paints
 *  to a WebGL canvas with no DOM text, so `.xterm-rows` can't be inspected — but
 *  the daemon ring buffer is the single source of truth behind the canvas. Returns
 *  the decoded scrollback text (binary frames + the base64 `scrollback` frame). */
async function readScrollback(page: Page, id: string): Promise<string> {
  return page.evaluate(
    ({ sid }) =>
      new Promise<string>((resolve) => {
        const base = localStorage.getItem('otto_base') ?? location.origin;
        const token = localStorage.getItem('otto_token') ?? '';
        const u = new URL(base);
        const proto = u.protocol === 'https:' ? 'wss:' : 'ws:';
        const sock = new WebSocket(`${proto}//${u.host}/ws/term/${sid}?token=${encodeURIComponent(token)}`);
        sock.binaryType = 'arraybuffer';
        const dec = new TextDecoder();
        let acc = '';
        const finish = () => {
          try { sock.close(); } catch { /* already closed */ }
          resolve(acc);
        };
        const timer = setTimeout(finish, 4500);
        sock.onopen = () => sock.send(JSON.stringify({ type: 'scrollback', lines: 5000 }));
        sock.onmessage = (ev: MessageEvent) => {
          if (ev.data instanceof ArrayBuffer) {
            acc += dec.decode(new Uint8Array(ev.data));
          } else if (typeof ev.data === 'string') {
            try {
              const m = JSON.parse(ev.data);
              if (m.type === 'scrollback' && m.data) {
                const bin = atob(m.data);
                const bytes = new Uint8Array(bin.length);
                for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
                acc += dec.decode(bytes);
                // Scrollback snapshot received — that's all we need.
                clearTimeout(timer);
                finish();
              }
            } catch { /* ignore control frames */ }
          }
        };
        sock.onerror = finish;
      }),
    { sid: id },
  );
}

/** Assert the session's scrollback contains `token` (polls — the ring buffer may
 *  lag a freshly-typed echo by a beat). Renderer-agnostic data verification. */
async function expectScrollbackContains(page: Page, id: string, token: string): Promise<void> {
  await expect
    .poll(() => readScrollback(page, id), { timeout: 15_000, message: `scrollback should contain "${token}"` })
    .toContain(token);
}

/** Open a session route and wait for its terminal host to mount AND its WS to
 *  connect. The connect wait matters before typing: xterm drops input while the
 *  socket isn't OPEN, so a too-early keystroke would silently never reach the PTY. */
async function openSession(page: Page, id: string): Promise<void> {
  await page.goto(`/#/agents/${id}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  // Wait until the "connecting…/reconnecting…" overlay is gone (WS attached).
  await expect(page.locator('.term-overlay .badge').filter({ hasText: /connecting|reconnecting/ }))
    .toHaveCount(0, { timeout: 20_000 });
}

/** A box is on-screen if it is non-null and lies (with a small tolerance)
 *  within the viewport — neither clipped past the right/bottom edge nor pushed
 *  off the left/top. This catches "control rendered off-screen / unreachable". */
async function expectOnScreen(page: Page, box: { x: number; y: number; width: number; height: number } | null, label: string): Promise<void> {
  const vp = page.viewportSize();
  expect(box, `${label}: present`).not.toBeNull();
  expect(vp, 'viewport size').not.toBeNull();
  expect(box!.width, `${label}: has width`).toBeGreaterThan(0);
  expect(box!.height, `${label}: has height`).toBeGreaterThan(0);
  expect(box!.x, `${label}: not off left`).toBeGreaterThanOrEqual(-2);
  expect(box!.y, `${label}: not off top`).toBeGreaterThanOrEqual(-2);
  expect(box!.x + box!.width, `${label}: not clipped right`).toBeLessThanOrEqual(vp!.width + 2);
  expect(box!.y + box!.height, `${label}: not clipped bottom`).toBeLessThanOrEqual(vp!.height + 2);
}

// ── 1. The session view lays out correctly (no overflow, real heights) ───────
test('session view: no horizontal overflow + panes have real height', async ({ page }) => {
  await openSession(page, shellA);

  await expectNoHorizontalOverflow(page);
  await expectContentHasHeight(page);

  const host = await page.locator('.term-host').first().boundingBox();
  expect(host, 'terminal host present').not.toBeNull();
  expect(host!.height, 'terminal height (no black-void collapse)').toBeGreaterThan(120);
  expect(host!.width, 'terminal width').toBeGreaterThan(0);
});

// ── 2. The pane header never overflows / pushes the page wide ────────────────
// The header packs many chips (title, provider, cwd, idle hint, controls). On a
// narrow handset these must truncate, not force horizontal scroll.
test('pane header fits within the pane (chips truncate, no overflow)', async ({ page }) => {
  await openSession(page, shellA);
  const head = page.locator('.pane-head').first();
  await expect(head).toBeVisible();
  const overflow = await head.evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(overflow, 'pane header horizontal overflow (px)').toBeLessThanOrEqual(2);
  // And it must not have widened the document either.
  await expectNoHorizontalOverflow(page);
});

// ── 3. Viewing data: seeded output is actually rendered/visible ──────────────
// Renderer-aware: on phone xterm uses its DOM renderer, so the rendered text
// lives in `.xterm-rows` and we assert it directly. On tablet/desktop xterm
// paints to a WebGL canvas (no DOM text), so we verify the data is present by
// reading the session scrollback over a WebSocket (renderer-agnostic) and that a
// render surface (canvas or rows) is actually painting.
test('viewing data: seeded terminal output is rendered', async ({ page }) => {
  await openSession(page, shellA);
  await page.waitForTimeout(1500); // let the WS attach + scrollback paint

  if (await liveIsPhone(page)) {
    // Phone uses xterm's DOM renderer → rendered text is in the DOM rows.
    await expect
      .poll(
        () => page.evaluate(() => document.querySelector('.xterm-rows')?.textContent ?? ''),
        { timeout: 15_000, message: 'seeded output should appear in the terminal rows' },
      )
      .toContain('OTTO-DATA-');
  } else {
    // Tablet/desktop use the WebGL canvas (no DOM text) — verify via scrollback.
    await expectScrollbackContains(page, shellA, 'OTTO-DATA-');
    // …and confirm a render surface is actually painting (canvas or rows).
    const painting = await page.evaluate(() => {
      const c = document.querySelector('.xterm canvas') as HTMLCanvasElement | null;
      const rows = document.querySelector('.xterm-rows');
      return (!!c && c.width > 0 && c.height > 0) || !!(rows && (rows.textContent ?? '').trim().length > 0);
    });
    expect(painting, 'a terminal render surface (canvas or DOM rows) is present').toBe(true);
  }
});

// ── 4. Writing commands: type into the terminal and confirm it reached the PTY
// Real keyboard typing requires a focusable xterm textarea. WebKit touch
// EMULATION doesn't synthesize the iOS soft-keyboard focus path, so this runs on
// the pointer-driven (non-phone) viewports; phone typing via the key accessory
// bar is covered by terminal-mobile.spec.ts. The echoed token is confirmed via
// the session scrollback so it's renderer-agnostic (WebGL canvas has no DOM text).
test('writing commands: typed command echoes into the terminal', async ({ page }) => {
  await openSession(page, shellB);
  test.skip(await liveIsPhone(page), 'soft-keyboard focus not reproducible under touch emulation');

  const host = page.locator('.term-host').first();
  await host.click();
  await expect
    .poll(() => page.evaluate(() => document.activeElement?.classList.contains('xterm-helper-textarea')))
    .toBe(true);

  const token = `OTTOTYPED${Date.now().toString().slice(-6)}`;
  await page.keyboard.type(`echo ${token}`);
  await page.keyboard.press('Enter');

  // The echoed token must land in the session scrollback — proves the keystrokes
  // reached the PTY. Renderer-agnostic (WebGL canvas has no DOM text to read).
  await expectScrollbackContains(page, shellB, token);
});

// ── 5. New Session sheet: opens, fits the viewport, is fully usable ──────────
test('New Session sheet fits the viewport and Start is reachable', async ({ page }) => {
  await openSession(page, shellA);

  // The "+" new-tab button is in the TabBar on every viewport (Agents view).
  await page.locator('.tabbar .new-tab').click();

  const sheet = page.locator('.sheet[role="dialog"]');
  await expect(sheet).toBeVisible();
  await expectOnScreen(page, await sheet.boundingBox(), 'New Session sheet');
  await expectNoHorizontalOverflow(page);

  // Provider cards render (at least the built-in shell).
  const providers = page.locator('.provider-card');
  expect(await providers.count(), 'provider cards').toBeGreaterThan(0);

  // The footer Start button must be on-screen + clickable (not clipped below the
  // fold on a short landscape phone).
  const start = startSessionBtn(page);
  await expect(start).toBeVisible();
  await expectOnScreen(page, await start.boundingBox(), 'Start Session button');

  // Tap targets in the provider grid are comfortable.
  const firstCard = await providers.first().boundingBox();
  expect(firstCard!.height, 'provider card tap height').toBeGreaterThanOrEqual(40);

  await page.keyboard.press('Escape');
  await expect(sheet).toBeHidden();
});

// ── 6. New Session: provider selection adapts (claude/codex extras) ──────────
test('New Session: selecting claude/codex reveals agent-only options', async ({ page }) => {
  await openSession(page, shellA);
  await page.locator('.tabbar .new-tab').click();
  await expect(page.locator('.sheet[role="dialog"]')).toBeVisible();

  const shellCard = page.locator('.provider-card', { hasText: 'shell' });
  const claudeCard = page.locator('.provider-card', { hasText: 'claude' });

  // shell selected → no Browser-tools toggle.
  if (await shellCard.count()) {
    await shellCard.first().click();
    await expect(page.locator('.toggle-row')).toBeHidden();
  }
  // claude selected → Browser-tools toggle + context preview appear.
  if (await claudeCard.count()) {
    await claudeCard.first().click();
    await expect(page.locator('.toggle-row')).toBeVisible();
    await expect(page.locator('.preview-toggle')).toBeVisible();
    // Still fits — the extra fields must not blow out the layout.
    await expectNoHorizontalOverflow(page);
    await expectOnScreen(page, await page.locator('.sheet[role="dialog"]').boundingBox(), 'sheet w/ claude options');
  }
  await page.keyboard.press('Escape');
});

// ── 7. Opening a plain shell through the UI actually launches one ─────────────
test('opening a plain shell from the sheet launches a live terminal', async ({ page }) => {
  await openSession(page, shellA);
  const before = await page.locator('.tabbar .tab').count();

  await page.locator('.tabbar .new-tab').click();
  await expect(page.locator('.sheet[role="dialog"]')).toBeVisible();
  const shellCard = page.locator('.provider-card', { hasText: 'shell' });
  test.skip(!(await shellCard.count()), 'shell provider not advertised by this daemon');
  await shellCard.first().click();
  await startSessionBtn(page).click();

  // Sheet closes, a new tab is added, and its terminal mounts with real height.
  await expect(page.locator('.sheet[role="dialog"]')).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => page.locator('.tabbar .tab').count()).toBeGreaterThan(before);
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  const host = await page.locator('.term-host').first().boundingBox();
  expect(host!.height, 'new shell terminal height').toBeGreaterThan(120);
  await expectNoHorizontalOverflow(page);
});

// ── 8. Opening a claude agent through the UI (best-effort) ────────────────────
// If the daemon advertises claude, create one and assert the PANE LAYOUT is
// correct (header chip + terminal host height). We do NOT assert on CLI content
// — the throwaway daemon may not fully boot the agent — only that the session UI
// renders properly on the device.
test('opening a claude agent renders a correct pane (best-effort)', async ({ page }) => {
  await openSession(page, shellA);
  await page.locator('.tabbar .new-tab').click();
  await expect(page.locator('.sheet[role="dialog"]')).toBeVisible();
  const claudeCard = page.locator('.provider-card', { hasText: 'claude' });
  test.skip(!(await claudeCard.count()), 'claude provider not advertised by this daemon');
  await claudeCard.first().click();
  await startSessionBtn(page).click();

  await expect(page.locator('.sheet[role="dialog"]')).toBeHidden({ timeout: 10_000 });
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  const host = await page.locator('.term-host').first().boundingBox();
  expect(host!.height, 'claude terminal height').toBeGreaterThan(120);
  // Provider chip in the pane header reflects the agent kind.
  await expect(page.locator('.pane-head .provider-chip').first()).toContainText(/claude/i);
  await expectNoHorizontalOverflow(page);
});

// ── 9. Tabs: multiple sessions are reachable + scrollable, no overflow ───────
test('tab bar: tabs scroll horizontally and switch sessions', async ({ page }) => {
  // Open BOTH seeded sessions so two tabs exist (a tab is only added when a
  // session is actually opened/routed to — seeding alone doesn't open a tab).
  await openSession(page, shellB);
  await openSession(page, shellA);
  const tabs = page.locator('.tabbar .tab');
  await expect.poll(() => tabs.count(), { message: 'two open tabs' }).toBeGreaterThanOrEqual(2);

  // The tab strip itself must not overflow the document.
  await expectNoHorizontalOverflow(page);

  // View-mode toggle (tabs/tiled/mission) is reachable.
  await expectOnScreen(page, await page.locator('.view-toggle').boundingBox(), 'view-mode toggle');

  // Switching tabs swaps the active pane without breaking layout.
  await tabs.nth(1).click();
  await expect(page.locator('.term-host').first()).toBeVisible();
  await expectContentHasHeight(page);
  await expectNoHorizontalOverflow(page);
});

// ── 10. Tiled view: all sessions visible at once, no overflow ────────────────
test('tiled view renders tiles with height and no overflow', async ({ page }) => {
  await openSession(page, shellA);
  // Switch to tiled via the toolbar (grid) button.
  await page.locator('.view-toggle button[aria-label="Tiled view"]').click();

  // Tiled grid mounts; at least one terminal host with real height is visible.
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  const host = await page.locator('.term-host').first().boundingBox();
  expect(host!.height, 'tile terminal height').toBeGreaterThan(80);
  await expectNoHorizontalOverflow(page);
  await expectContentHasHeight(page);

  // Restore tabbed view so we don't leak the mode to other tests' state.
  await page.locator('.view-toggle button[aria-label="Tabbed view"]').click();
});

// ── 11. Phone-only: floating term controls + action bar are on-screen ────────
test('phone: terminal floating controls + action bar are reachable', async ({ page }) => {
  await openSession(page, shellA);
  test.skip(!(await liveIsPhone(page)), 'phone-mode only (≤640px live viewport)');

  // The floating ⌨/zoom strip sits bottom-right; it must be fully on-screen.
  const kbToggle = page.locator('.phone-btn[aria-label="Toggle keyboard"]');
  await expect(kbToggle).toBeVisible();
  await expectOnScreen(page, await kbToggle.boundingBox(), 'phone keyboard toggle');
  const kbBox = await kbToggle.boundingBox();
  expect(kbBox!.height, 'keyboard toggle tap height').toBeGreaterThanOrEqual(44);

  // The bottom MobileActionBar exposes New/Close/Find/Palette as real tap targets.
  const actionBar = page.locator('.action-bar[role="toolbar"]');
  await expect(actionBar).toBeVisible();
  await expectOnScreen(page, await actionBar.boundingBox(), 'mobile action bar');
  const newBtn = page.locator('.action-bar .ab-btn', { hasText: 'New' });
  await expect(newBtn).toBeVisible();
  expect((await newBtn.boundingBox())!.height, 'action bar tap height').toBeGreaterThanOrEqual(40);

  // Bottom nav is present and on-screen.
  await expectOnScreen(page, await page.locator('.bottomnav').boundingBox(), 'bottom nav');
});

// ── 12. Phone-only: left navigator drawer + right activity drawer fit ────────
test('phone: navigator + activity drawers open within the viewport', async ({ page }) => {
  await openSession(page, shellA);
  test.skip(!(await liveIsPhone(page)), 'phone-mode only (≤640px live viewport)');

  // Open the left navigator drawer from the top bar.
  await page.locator('.mtop-btn[aria-label="Open navigator"]').click();
  const leftDrawer = page.locator('.drawer[aria-label="Navigator"]').first();
  await expect(leftDrawer).toBeVisible();
  await expectOnScreen(page, await settledBox(leftDrawer), 'navigator drawer');
  await expectNoHorizontalOverflow(page);
  // Close it (Escape).
  await page.keyboard.press('Escape');
  await expect(leftDrawer).toBeHidden();

  // Open the right activity drawer (agent session focused → the panel toggle shows).
  const rightToggle = page.locator('.mtop-btn[aria-label="Toggle right panel"]');
  if (await rightToggle.count()) {
    await rightToggle.click();
    const rightDrawer = page.locator('.drawer[aria-label="Activity"]').first();
    await expect(rightDrawer).toBeVisible();
    await expectOnScreen(page, await settledBox(rightDrawer), 'activity drawer');
    await expectNoHorizontalOverflow(page);
  }
});
