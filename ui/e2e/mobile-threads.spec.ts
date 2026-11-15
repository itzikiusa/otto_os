import { test, expect, type APIRequestContext, type Page, type Route } from '@playwright/test';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Threads on a phone — the main surface once a phone is paired (Phone access,
// docs/features/phone-access.md). iPhone portrait only:
//   1. a "Continue on phone" deep link `#/threads/<id>` into ANOTHER workspace
//      switches to it and opens the thread (it used to open nothing);
//   2. the thread is usable at phone width: no horizontal scroll, sidebar /
//      inspector as drawers, the composer stays above the on-screen keyboard,
//      an inline approval's buttons are ≥44px tap targets;
//   3. the pairing page's `next` lands on the same thread, signed in.
// The /phone/* daemon routes are mocked here (page.route) so the spec runs
// against any daemon build that has structured runtime threads; the runtime
// thread itself is real (`otto-fake-codex`, see desktop-threads-runtime).
// ─────────────────────────────────────────────────────────────────────────────

// The PWA service worker would take the daemon fetches past page.route.
test.use({ serviceWorkers: 'block' });

let ctx: APIRequestContext;
let base = '';
let token = '';
let homeWs = '';
let otherWs = '';
let threadId = '';
let deskWs = '';
let deskThread = '';
const dirs: string[] = [];

async function workspace(name: string): Promise<string> {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-mthreads-'));
  dirs.push(dir);
  writeFileSync(join(dir, 'README.md'), `# ${name}\n`);
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name, root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  token = c.token;
  homeWs = await workspace('Phone Home WS');
  otherWs = await workspace('Phone Other WS');
  const r = await ctx.post(`${base}/api/v1/workspaces/${otherWs}/runtime-sessions`, {
    data: { provider: 'codex', title: 'Phone thread' },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  threadId = (await r.json()).id as string;
  // A terminal (PTY) thread the desktop started in a third workspace.
  deskWs = await workspace('Desk WS');
  const d = await ctx.post(`${base}/api/v1/workspaces/${deskWs}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Desk terminal thread', cwd: dirs[dirs.length - 1], meta: { origin: 'e2e', surface: 'threads' } },
  });
  expect(d.ok(), await d.text()).toBeTruthy();
  deskThread = (await d.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && threadId) await ctx.post(`${base}/api/v1/runtime-sessions/${threadId}/close`).catch(() => {});
  if (ctx && deskThread) await ctx.delete(`${base}/api/v1/sessions/${deskThread}`).catch(() => {});
  await ctx?.dispose();
  for (const d of dirs) rmSync(d, { recursive: true, force: true });
});

const CORS = {
  'access-control-allow-origin': '*',
  'access-control-allow-headers': 'authorization, content-type, x-otto-ui-conn, traceparent',
  'access-control-allow-methods': 'GET, POST, PATCH, DELETE, OPTIONS',
};

/** Fulfil a mocked daemon route cross-origin (the UI origin ≠ the daemon's). */
async function mock(page: Page, glob: string, body: (r: Route) => unknown | Promise<unknown>): Promise<void> {
  await page.route(glob, async (r) => {
    if (r.request().method() === 'OPTIONS') return r.fulfill({ status: 204, headers: CORS });
    return r.fulfill({ json: await body(r), headers: CORS });
  });
}

/** Mock the Phone-access routes: `paired` = this window is a paired phone. */
async function mockPhone(page: Page, paired: boolean): Promise<void> {
  const device = {
    id: 'dev1', name: 'iPhone · Safari', user_agent: 'iPhone', created_at: new Date().toISOString(),
    last_seen_at: new Date().toISOString(), last_ip: '100.111.7.125',
    idle_expires_at: new Date(Date.now() + 30 * 864e5).toISOString(),
    expires_at: new Date(Date.now() + 180 * 864e5).toISOString(), current: true,
  };
  await mock(page, '**/api/v1/phone/device/me', () => ({ device: paired ? device : null }));
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'iphone-portrait project only');
  // The window starts in the OTHER (home) workspace — the deep link must switch.
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('m-threads-init')) return;
    sessionStorage.setItem('m-threads-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, homeWs);
});

test('a thread deep link from another workspace opens the thread at phone width', async ({ page }, info) => {
  await mockPhone(page, true);
  await page.goto(`/#/threads/${threadId}`);
  const thread = page.getByTestId(`runtime-thread-${threadId}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('threads-breadcrumb')).toContainText('Phone thread');
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_workspace'))).toBe(otherWs);
  await expect(page).toHaveURL(new RegExp(`#/threads/${threadId}$`));
  await expectNoHorizontalOverflow(page);
  // A paired phone never offers "Continue on phone" (it is the phone): the
  // handoff lives in the toolbar ⋯ only, and not there either.
  await page.getByTestId('threads-thread-menu').click();
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: /Continue on phone/ })).toHaveCount(0);
  await page.keyboard.press('Escape');

  // Sidebar + inspector are drawers over the stage, fully on screen.
  await page.getByTestId('threads-toggle-sidebar').click();
  const sidebar = page.getByTestId('threads-sidebar');
  await expectFullyInViewport(page, sidebar, 'threads sidebar drawer');
  await page.getByRole('button', { name: 'Close Threads', exact: true }).click();
  await expect(sidebar).toBeHidden();
  await page.getByTestId('threads-toggle-inspector').click();
  await expectNoHorizontalOverflow(page);
  await page.getByRole('button', { name: 'Close Thread details', exact: true }).click();
  await expect(thread).toBeVisible();

  // The thread's header (toolbar) stays fully on screen: on open, with the
  // composer focused and blurred, and after a long conversation scrolled to
  // its start and back (the top used to end up cut off).
  const header = page.getByTestId('threads-page').getByTestId('page-header').first();
  await expectFullyInViewport(page, header, 'thread header on open');
  const composer = thread.getByTestId('runtime-composer');
  // ≥16px, or iOS zooms the page into the field on focus and stays zoomed.
  const composerPx = await composer.evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(composerPx, 'composer font ≥16px (no iOS zoom-on-focus)').toBeGreaterThanOrEqual(16);
  await composer.click();
  await expectFullyInViewport(page, header, 'thread header with the composer focused');

  // The composer stays in view with the on-screen keyboard up (a shorter
  // visual viewport); typing and sending works.
  await page.setViewportSize({ width: 430, height: 520 });
  await expectFullyInViewport(page, composer, 'composer with the keyboard up');
  await expectFullyInViewport(page, header, 'thread header with the keyboard up');
  // #app follows the visible viewport (lib/viewportFit.ts) on a touch device.
  expect(await page.evaluate(() => document.getElementById('app')!.getBoundingClientRect().height)).toBe(520);
  const sendBtn = thread.getByTestId('runtime-send');
  await composer.fill('hello from the phone');
  // On touch-first devices Enter inserts a newline; the send button sends.
  await composer.press('Enter');
  await expect(composer).toHaveValue('hello from the phone\n');
  await sendBtn.click();
  await expect(thread.getByText('echo: hello from the phone')).toBeVisible({ timeout: 20_000 });
  await page.setViewportSize({ width: 430, height: 932 });
  await composer.blur();
  await expectFullyInViewport(page, header, 'thread header after the composer blurs');

  // A long conversation: scroll every scroller to its start, then its end.
  for (let i = 0; i < 6; i++) {
    await composer.fill(`long ${i} ` + 'a long line of text '.repeat(30));
    await sendBtn.click();
    await expect(thread.getByText(`echo: long ${i} `).first()).toBeVisible({ timeout: 20_000 });
  }
  // A markdown table at phone width: a short column keeps its words whole
  // (it used to break letter by letter) and the table scrolls inside itself.
  await composer.fill('t\n\n| Agent | Status |\n|---|---|\n| tma-acp | ' + 'working on the Antigravity driver and the timeline '.repeat(4) + '|');
  await sendBtn.click();
  const cell = thread.locator('.md td', { hasText: /^tma-acp$/ }).last();
  await expect(cell).toBeVisible({ timeout: 20_000 });
  // Its narrowest line holds a whole word part ("tma-"), never one letter.
  const lineWidths = await cell.evaluate((el) => {
    const r = document.createRange();
    r.selectNodeContents(el);
    return [...r.getClientRects()].map((b) => b.width / parseFloat(getComputedStyle(el).fontSize));
  });
  expect(Math.min(...lineWidths), 'short table cell keeps its words whole').toBeGreaterThan(1.5);
  await expectNoHorizontalOverflow(page);
  for (const to of [0, 1e7]) {
    await page.evaluate((y) => {
      for (const el of document.querySelectorAll<HTMLElement>('[data-testid="threads-page"] *')) {
        if (el.scrollHeight > el.clientHeight + 4 && /auto|scroll/.test(getComputedStyle(el).overflowY)) el.scrollTop = y;
      }
    }, to);
    await expectFullyInViewport(page, header, `thread header after scrolling the conversation to ${to ? 'its end' : 'its start'}`);
  }
  expect(await page.evaluate(() => window.scrollY + document.scrollingElement!.scrollTop)).toBe(0);

  // An inline approval: its buttons are real tap targets.
  await composer.fill('ask');
  await sendBtn.click();
  const approval = thread.locator('[data-testid^="runtime-approval-"]');
  await expect(approval).toBeVisible({ timeout: 20_000 });
  for (const name of ['Approve', 'Decline']) {
    const btn = approval.getByRole('button', { name, exact: true });
    const box = await btn.boundingBox();
    expect(box, `${name} button`).not.toBeNull();
    expect(box!.height, `${name} tap target height`).toBeGreaterThanOrEqual(44);
  }
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('phone-thread-approval.png'), animations: 'disabled' });
  await approval.getByRole('button', { name: 'Approve', exact: true }).click();
  await expect(approval).toHaveCount(0, { timeout: 20_000 });
});

test('the phone lists every workspace’s threads under their folders, light and dark', async ({ page }) => {
  await mockPhone(page, true);
  const folder = (i: number) => dirs[i].split('/').pop()!;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/#/threads');
    await page.getByTestId('threads-toggle-sidebar').click();
    const sidebar = page.getByTestId('threads-sidebar');
    // The window has the home workspace open; both threads live elsewhere.
    for (const [title, dir] of [['Phone thread', 1], ['Desk terminal thread', 2]] as const) {
      const group = sidebar.getByRole('region', { name: `Project ${folder(dir)}` });
      await expect(group, `${scheme}: ${title}`).toBeVisible({ timeout: 30_000 });
      await expect(group).toContainText(title);
    }
    await expectFullyInViewport(page, sidebar, `threads sidebar (${scheme})`);
    await expectNoHorizontalOverflow(page);
  }
  // A terminal thread of another workspace opens there.
  await page.getByTestId(`thread-row-${deskThread}`).getByRole('button', { name: /Desk terminal thread/ }).first().click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_workspace'))).toBe(deskWs);
});

test('Continue on phone (desktop credential) offers the thread QR when Phone access is on', async ({ page }) => {
  await mockPhone(page, false);
  await mock(page, '**/api/v1/phone/status', () => ({
        installed: true, cli_path: '/usr/local/bin/tailscale', backend_state: 'Running', logged_in: true,
        tailnet: 'me@example.com', magic_dns: true, self_name: 'mac.tail0b7b6b.ts.net', self_ips: ['100.120.155.91'],
        devices: [], serving: { url: 'http://mac.tail0b7b6b.ts.net', https: false, port: 80 }, other_serves: false,
        note: 'HTTPS unavailable', error: null, can_manage: true,
  }));
  await mock(page, '**/api/v1/phone/devices', () => [{ id: 'd', name: 'iPhone', user_agent: null, created_at: '', last_seen_at: '', last_ip: null, idle_expires_at: '', expires_at: '', current: false }]);
  await page.goto(`/#/threads/${threadId}`);
  await expect(page.getByTestId(`runtime-thread-${threadId}`)).toBeVisible({ timeout: 30_000 });
  // The handoff is a ⋯ item (the toolbar keeps sidebar, title, right panel, ⋯).
  await page.getByTestId('threads-thread-menu').click();
  await page.getByRole('menuitem', { name: /Continue on phone/ }).click();
  const sheet = page.getByRole('dialog', { name: 'Continue on phone' });
  await expectFullyInViewport(page, sheet, 'Continue on phone sheet');
  await expect(sheet).toContainText(`http://mac.tail0b7b6b.ts.net/#/threads/${threadId}`);
  await expect(sheet.locator('canvas')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('pairing lands on the requested thread, signed in, with no password', async ({ browser }) => {
  // A fresh phone: no stored login at all.
  const phone = await browser.newContext({ ...test.info().project.use, storageState: undefined, serviceWorkers: 'block' });
  const page = await phone.newPage();
  const { port } = JSON.parse(
    readFileSync(join(process.cwd(), 'e2e', `.auth-${process.env.OTTO_E2E_SLOT ?? '0'}`, 'daemon.json'), 'utf8'),
  ) as { port: string };
  await page.addInitScript(({ p, ws }) => {
    localStorage.setItem('otto_base', `http://127.0.0.1:${p}`);
    localStorage.setItem('otto_workspace', ws);
  }, { p: port, ws: homeWs });
  const code = 'Z'.repeat(43);
  let polls = 0;
  let claimed: unknown = null;
  await mock(page, '**/api/v1/phone/pair/claim', (r) => {
    claimed = r.request().postDataJSON();
    return { pairing_id: 'p1', poll_secret: 's'.repeat(43), confirm_code: '482913', expires_at: new Date(Date.now() + 120_000).toISOString() };
  });
  await mock(page, '**/api/v1/phone/pair/poll', () =>
    ++polls < 3 ? { status: 'pending' } : { status: 'approved', token, next: `threads/${threadId}` },
  );
  await mockPhone(page, true);
  await page.goto(`/#/pair/${code}/threads/${threadId}`);
  // The one-time code is stripped from the URL at once.
  await expect(page).toHaveURL(/#\/pair$/);
  await expect(page.getByText('482913')).toBeVisible();
  expect(claimed).toEqual({ code });
  await expectNoHorizontalOverflow(page);
  await expect(page).toHaveURL(new RegExp(`#/threads/${threadId}$`), { timeout: 20_000 });
  await expect(page.getByTestId(`runtime-thread-${threadId}`)).toBeVisible({ timeout: 30_000 });
  await phone.close();
});

test('a 60-turn conversation opens via tail_turns=20 on phone and loads earlier turns without scroll jump', async ({ page }) => {
  await mockPhone(page, true);
  const TOTAL = 60;
  const requestedUrls: string[] = [];

  const buildPage = (fromOrd: number, toOrd: number, hasOlder: boolean, olderTurns: number) => {
    const turns = [];
    const turn_items = [];
    let itemOrd = (fromOrd - 1) * 4 + 1;
    for (let ord = fromOrd; ord <= toOrd; ord++) {
      const tid = `turn-${ord}`;
      turns.push({
        id: tid,
        ordinal: ord,
        status: 'completed',
        started_at: '2026-11-15T00:00:00Z',
        completed_at: '2026-11-15T00:00:02Z',
        origin: 'user',
      });
      turn_items.push(
        {
          id: `u-${ord}`,
          turn_id: tid,
          ordinal: itemOrd++,
          status: 'completed',
          title: null,
          started_at: '2026-11-15T00:00:00Z',
          completed_at: '2026-11-15T00:00:00Z',
          updated_at: '2026-11-15T00:00:00Z',
          type: 'user_message',
          text: `Question number ${ord}`,
        },
        {
          id: `c1-${ord}`,
          turn_id: tid,
          ordinal: itemOrd++,
          status: 'completed',
          title: `rg turn_${ord}`,
          error: null,
          started_at: '2026-11-15T00:00:00Z',
          completed_at: '2026-11-15T00:00:01Z',
          updated_at: '2026-11-15T00:00:01Z',
          type: 'command_execution',
          command: `rg turn_${ord}`,
          cwd: '/tmp',
          exit_code: 0,
        },
        {
          id: `c2-${ord}`,
          turn_id: tid,
          ordinal: itemOrd++,
          status: 'completed',
          title: `cargo check ${ord}`,
          error: null,
          started_at: '2026-11-15T00:00:01Z',
          completed_at: '2026-11-15T00:00:02Z',
          updated_at: '2026-11-15T00:00:02Z',
          type: 'command_execution',
          command: `cargo check ${ord}`,
          cwd: '/tmp',
          exit_code: 0,
        },
        {
          id: `a-${ord}`,
          turn_id: tid,
          ordinal: itemOrd++,
          status: 'completed',
          title: null,
          started_at: '2026-11-15T00:00:01Z',
          completed_at: '2026-11-15T00:00:02Z',
          updated_at: '2026-11-15T00:00:02Z',
          type: 'assistant_message',
          message_id: `m-${ord}`,
          text: `Answer for turn ${ord}: ` + 'detailed summary line '.repeat(8),
          streaming: false,
        },
      );
    }
    return {
      provider_session: null,
      provider_thread: null,
      turns,
      turn_items,
      runtime_requests: [],
      plans: [],
      queued_prompts: [],
      has_older: hasOlder,
      oldest_turn_ordinal: fromOrd,
      older_turns: olderTurns,
    };
  };

  await mock(page, `**/api/v1/runtime-sessions/${threadId}/events*`, (r) => {
    const u = new URL(r.request().url());
    requestedUrls.push(u.pathname + u.search);
    const before = Number(u.searchParams.get('before_turn_ordinal') || '0');
    const after = Number(u.searchParams.get('after_sequence') || '0');
    if (before > 0) {
      const limit = Number(u.searchParams.get('limit_turns') || '20');
      const from = Math.max(1, before - limit);
      const to = before - 1;
      return {
        kind: 'snapshot',
        snapshot_sequence: 240,
        snapshot: buildPage(from, to, from > 1, from - 1),
      };
    }
    if (after >= 240) {
      return { kind: 'events', events: [], last_sequence: 240 };
    }
    return {
      kind: 'snapshot',
      snapshot_sequence: 240,
      snapshot: buildPage(41, TOTAL, true, 40),
    };
  });

  // Warm shell first, then time switching/opening the 60-turn thread.
  await page.goto('/#/threads');
  await expect(page.getByTestId('threads-page')).toBeVisible({ timeout: 30_000 });
  const t0 = Date.now();
  await page.goto(`/#/threads/${threadId}`);
  const thread = page.getByTestId(`runtime-thread-${threadId}`);
  await expect(thread.getByText('Answer for turn 60:')).toBeVisible({ timeout: 5_000 });
  const openMs = Date.now() - t0;
  expect(openMs, `60-turn phone conversation opened in ${openMs}ms`).toBeLessThan(800);
  expect(requestedUrls.some((u) => u.includes('tail_turns=20'))).toBe(true);

  // Closed work folds must mount 0 `.rf-body` containers until tapped open!
  await expect(thread.locator('.rf-body')).toHaveCount(0);
  await expect(thread.getByText('Answer for turn 25:')).toHaveCount(0);

  // Scroll to top of the 20-turn tail window, anchor turn 41's bubble Y, and tap "Load earlier (40 turns)".
  await thread.locator('.rt-scroll').evaluate((el) => {
    el.scrollTop = 0;
  });
  const loadOlderBtn = thread.getByTestId('runtime-load-older');
  await expect(loadOlderBtn).toContainText('Load earlier (40 turns)');
  await expectFullyInViewport(page, loadOlderBtn, 'Load earlier (40 turns) button');
  const beforeTop41 = await thread.getByText('Question number 41').evaluate((el) => el.getBoundingClientRect().top);
  await loadOlderBtn.click();
  await expect(thread.getByText('Answer for turn 21:')).toBeVisible({ timeout: 5_000 });
  await expect(loadOlderBtn).toContainText('Load earlier (20 turns)');
  const afterTop41 = await thread.getByText('Question number 41').evaluate((el) => el.getBoundingClientRect().top);
  expect(Math.abs(afterTop41 - beforeTop41), 'viewport anchor stayed stable when prepending 20 older turns').toBeLessThan(8);

  // Scroll to the top of the 40-turn window and load the remaining 20 turns without scroll jump.
  await thread.locator('.rt-scroll').evaluate((el) => {
    el.scrollTop = 0;
  });
  await expectFullyInViewport(page, loadOlderBtn, 'Load earlier (20 turns) button');
  const beforeTop21 = await thread.getByText('Question number 21').evaluate((el) => el.getBoundingClientRect().top);
  await loadOlderBtn.click();
  await expect(thread.getByText('Answer for turn 1:')).toBeVisible({ timeout: 5_000 });
  await expect(loadOlderBtn).toHaveCount(0);
  const afterTop21 = await thread.getByText('Question number 21').evaluate((el) => el.getBoundingClientRect().top);
  expect(Math.abs(afterTop21 - beforeTop21), 'viewport anchor stayed stable when prepending final 20 turns').toBeLessThan(8);
  await expectNoHorizontalOverflow(page);
});
