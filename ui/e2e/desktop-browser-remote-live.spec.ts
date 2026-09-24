import { test, expect, type APIRequestContext, type Page, type WebSocketRoute } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// Browser module — the REMOTE live view (a daemon-owned Chromium streamed over
// `WS /ws/browser/{tab_id}/live`, docs/contracts/ws.md §1b).
//
// No real Chromium is involved: the engine status / session routes are
// answered with `page.route` and the live socket with `page.routeWebSocket`,
// which plays the daemon's side of the protocol — it answers every `resize`
// with a real JPEG screencast frame (a fixture page rendered by Playwright at
// exactly the requested viewport) and records every input frame the UI sends,
// so the spec asserts on the wire (coordinates, keys, acks, control) rather
// than on pixels. Tabs and workspaces are REAL rows in the isolated test
// daemon; nothing here reaches 127.0.0.1:7700.

const TAB_URL = 'https://example.invalid/appointments';

const FIXTURE_HTML = `<!doctype html><html><head><style>
  body{margin:0;font:15px -apple-system,Helvetica,Arial;background:#fbfaf7;color:#1d2a26}
  header{display:flex;gap:28px;align-items:center;padding:18px 32px;border-bottom:1px solid #e5e1d8}
  header b{font-size:20px;color:#0f6b5c} header span{color:#58645f}
  main{padding:32px;max-width:720px}
  h1{font-size:26px;margin:0 0 6px} p{color:#58645f;margin:0 0 20px}
  .slots{display:grid;grid-template-columns:repeat(3,1fr);gap:10px;margin-bottom:24px}
  .slots div{border:1px solid #d6d1c5;border-radius:8px;padding:12px;text-align:center;background:#fff}
  .slots .on{border:2px solid #0f6b5c;background:#e6f2ef;font-weight:600}
  label{display:block;font-size:13px;color:#58645f;margin:14px 0 6px}
  .in{border:1px solid #d6d1c5;border-radius:6px;padding:10px 12px;background:#fff}
  button{margin-top:22px;background:#0f6b5c;color:#fff;border:0;border-radius:8px;padding:12px 22px;font-size:15px}
</style></head><body>
<header><b>Brightsmile</b><span>Appointments</span><span>Billing</span><span>Profile</span></header>
<main><h1>Choose a new time</h1><p>Cleaning with Dr. Ortega · 45 min · Main St. clinic</p>
<div class="slots"><div>Tue 9:00</div><div>Tue 14:30</div><div>Wed 8:15</div><div class="on">Thu 17:30</div><div>Fri 10:00</div><div>Fri 16:45</div></div>
<label>Patient</label><div class="in">Alex Rivera</div>
<label>Reason for change</label><div class="in">Work conflict on original date</div>
<button>Confirm reschedule</button></main></body></html>`;

interface LiveMock {
  /** Every JSON frame the UI sent, in order. */
  sent: any[];
  socket(): WebSocketRoute | null;
  /** Push a server frame (JSON). */
  push(msg: object): void;
  session: Record<string, any>;
  /** How many times the UI opened the live socket. */
  opens(): number;
}

function sessionFor(tabId: string, wsId: string, over: Record<string, any> = {}): Record<string, any> {
  return {
    tab_id: tabId,
    workspace_id: wsId,
    owner_id: 'root',
    engine: 'remote',
    build: 'chrome',
    version: '149.0.7827.55',
    profile: 'ephemeral',
    headed: false,
    state: 'ready',
    url: TAB_URL,
    title: 'Brightsmile — Reschedule',
    loading: false,
    can_go_back: false,
    can_go_forward: false,
    viewport: { width: 1280, height: 800, device_scale_factor: 1 },
    controller: 'none',
    controller_user_id: null,
    viewers: 1,
    created_at: new Date().toISOString(),
    last_activity_at: new Date().toISOString(),
    ...over,
  };
}

function statusJson(installed: boolean, install: object | null = null): object {
  const build = (b: string, bytes: number, label: string) => ({
    build: b,
    version: '149.0.7827.55',
    platform: 'mac-arm64',
    installed: installed && b === 'chrome',
    download_bytes: bytes,
    label,
    sha256_pinned: true,
    path: null,
    source: null,
  });
  return {
    platform_supported: true,
    builds: [
      build('chrome', 180 * 1024 * 1024, 'Chrome for Testing — full browser (~180 MB)'),
      build('chrome-headless-shell', 98 * 1024 * 1024, 'chrome-headless-shell (~98 MB)'),
    ],
    settings: { build: 'chrome', headed: false, max_sessions: 6, idle_timeout_secs: 900, downloads: 'quarantine' },
    install,
    processes: installed ? 1 : 0,
    sessions: 0,
  };
}

/** Render the fixture page at `w×h` into a v1 binary screencast frame. */
async function frameBytes(page: Page, w: number, h: number, seq: number): Promise<Buffer> {
  const shooter = await page.context().newPage();
  try {
    await shooter.setViewportSize({ width: w, height: h });
    await shooter.setContent(FIXTURE_HTML);
    const jpeg = await shooter.screenshot({ type: 'jpeg', quality: 80 });
    const header = Buffer.from(
      JSON.stringify({
        seq, mime: 'image/jpeg', width: w, height: h, device_width: w, device_height: h,
        page_scale_factor: 1, offset_top: 0, scroll_x: 0, scroll_y: 0, timestamp: Date.now() / 1000,
      }),
    );
    const out = Buffer.alloc(5 + header.length + jpeg.length);
    out.writeUInt8(1, 0);
    out.writeUInt32BE(header.length, 1);
    header.copy(out, 5);
    jpeg.copy(out, 5 + header.length);
    return out;
  } finally {
    await shooter.close();
  }
}

async function mockLive(page: Page, tabId: string, wsId: string, over: Record<string, any> = {}): Promise<LiveMock> {
  const sent: any[] = [];
  let sock: WebSocketRoute | null = null;
  let opens = 0;
  let seq = 0;
  const session = sessionFor(tabId, wsId, over);
  await page.route('**/api/v1/browser/live/status', (r) => r.fulfill({ json: statusJson(true) }));
  await page.route(`**/api/v1/browser/tabs/${tabId}/live`, (r) =>
    r.request().method() === 'DELETE' ? r.fulfill({ status: 204, body: '' }) : r.fulfill({ json: session }),
  );
  await page.routeWebSocket(/\/ws\/browser\/[^/]+\/live/, (ws) => {
    sock = ws;
    opens += 1;
    ws.onMessage((m) => {
      if (typeof m !== 'string') return;
      const msg = JSON.parse(m);
      sent.push(msg);
      if (msg.type === 'resize') {
        void frameBytes(page, msg.width, msg.height, ++seq).then((b) => ws.send(b)).catch(() => {});
      }
    });
    ws.send(JSON.stringify({ type: 'state', session }));
  });
  return {
    sent,
    session,
    socket: () => sock,
    push: (msg) => sock?.send(JSON.stringify(msg)),
    opens: () => opens,
  };
}

let ctx: APIRequestContext;
let base: string;
let wsId = '';
let tabId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  const tab = await (await ctx.post(`${base}/api/v1/workspaces/${wsId}/browser/tabs`, { data: { url: TAB_URL } })).json();
  tabId = tab.id;
  const r = await ctx.patch(`${base}/api/v1/browser/tabs/${tabId}`, { data: { mode: 'live', title: 'Brightsmile' } });
  expect(r.ok()).toBeTruthy();
  // The live tab must never fall back to a real reader fetch in this spec.
  await page.route('**/browser/page?url=**', (route) => route.fulfill({ status: 500, body: '{}' }));
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

async function openLive(page: Page): Promise<void> {
  await page.goto('/#/browser');
  await expect(page.getByTestId('remote-live')).toHaveAttribute('data-status', 'live', { timeout: 20_000 });
  await expect(page.getByTestId('live-meter')).toBeVisible({ timeout: 15_000 });
}

test('engine missing: asks first, names the size, lighter option, then downloads with progress', async ({ page }) => {
  let installBody: any = null;
  let phase: 'missing' | 'installing' | 'ready' = 'missing';
  await page.route('**/api/v1/browser/live/status', (r) => {
    if (phase === 'missing') return r.fulfill({ json: statusJson(false) });
    if (phase === 'installing') {
      return r.fulfill({
        json: statusJson(false, {
          build: 'chrome-headless-shell', version: '149.0.7827.55', state: 'downloading',
          received_bytes: 40 * 1024 * 1024, total_bytes: 98 * 1024 * 1024, error: null,
          started_at: new Date().toISOString(), finished_at: null,
        }),
      });
    }
    return r.fulfill({ json: statusJson(true) });
  });
  await page.route('**/api/v1/browser/live/install', async (r) => {
    installBody = r.request().postDataJSON();
    phase = 'installing';
    await r.fulfill({
      status: 202,
      json: {
        build: 'chrome-headless-shell', version: '149.0.7827.55', state: 'downloading', received_bytes: 0,
        total_bytes: 98 * 1024 * 1024, error: null, started_at: new Date().toISOString(), finished_at: null,
      },
    });
  });

  await page.goto('/#/browser');
  const setup = page.getByTestId('live-engine-setup');
  await expect(setup.getByRole('heading', { name: 'Enable live browsing' })).toBeVisible({ timeout: 15_000 });
  // Nothing was downloaded just by opening the tab.
  expect(installBody).toBeNull();
  await expect(setup.getByRole('button', { name: 'Download Chrome (180 MB)' })).toBeVisible();
  await setup.getByLabel(/Use the lighter engine/).check();
  await setup.getByRole('button', { name: 'Download lighter engine (98 MB)' }).click();
  expect(installBody).toEqual({ build: 'chrome-headless-shell' });
  await expect(setup.getByRole('progressbar', { name: 'Download progress' })).toHaveAttribute('aria-valuenow', '41', { timeout: 10_000 });
  await expect(setup).toContainText('40 MB of 98 MB');
});

test('frames draw, clicks map to page CSS px, frames are acked, the viewport follows the pane', async ({ page }) => {
  const live = await mockLive(page, tabId, wsId);
  await openLive(page);

  // resize on open matches the pane, and every drawn frame is acked
  const resize = live.sent.find((m) => m.type === 'resize');
  const canvas = page.locator('[data-testid="remote-live"] canvas');
  const box = (await canvas.boundingBox())!;
  expect(Math.abs(resize.width - box.width)).toBeLessThanOrEqual(2);
  expect(Math.abs(resize.height - box.height)).toBeLessThanOrEqual(2);
  await expect.poll(() => live.sent.some((m) => m.type === 'ack' && m.seq >= 1)).toBe(true);

  // a click at a known spot lands at the same CSS px on the remote page
  await canvas.click({ position: { x: 200, y: 120 } });
  await expect.poll(() => live.sent.filter((m) => m.type === 'mouse' && m.action === 'up').length).toBe(1);
  const down = live.sent.find((m) => m.type === 'mouse' && m.action === 'down');
  expect(down).toMatchObject({ button: 'left', click_count: 1, modifiers: 0 });
  expect(Math.abs(down.x - 200)).toBeLessThanOrEqual(1);
  expect(Math.abs(down.y - 120)).toBeLessThanOrEqual(1);

  // wheel scrolls the remote page, not Otto
  await canvas.hover({ position: { x: 300, y: 300 } });
  await page.mouse.wheel(0, 240);
  await expect.poll(() => live.sent.some((m) => m.type === 'mouse' && m.action === 'wheel' && m.delta_y > 0)).toBe(true);
});

test('keyboard: typing is forwarded, ⌘K stays with Otto, Esc releases, paste inserts text', async ({ page }) => {
  const live = await mockLive(page, tabId, wsId);
  await openLive(page);
  const canvas = page.locator('[data-testid="remote-live"] canvas');
  await canvas.click({ position: { x: 40, y: 40 } });
  const sink = page.getByRole('textbox', { name: /^Live page/ });
  await expect(sink).toBeFocused();
  await expect(page.getByText('Typing goes to the page')).toBeVisible();

  await page.keyboard.type('Hi');
  await page.keyboard.press('Enter');
  await expect
    .poll(() => live.sent.filter((m) => m.type === 'key' && m.action === 'down').map((m) => m.text ?? m.key))
    .toEqual(['H', 'i', '\r']);
  const h = live.sent.find((m) => m.type === 'key' && m.key === 'H');
  expect(h).toMatchObject({ code: 'KeyH', key_code: 72, modifiers: 8 });

  // ⌘K / Ctrl+K opens Otto's palette and is NOT sent to the page
  await page.keyboard.press('ControlOrMeta+k');
  await expect(page.locator('.palette')).toBeVisible();
  expect(live.sent.some((m) => m.type === 'key' && m.key.toLowerCase() === 'k')).toBe(false);
  await page.keyboard.press('Escape');

  // back in the page: Esc hands the keyboard back to Otto
  await canvas.click({ position: { x: 40, y: 40 } });
  await expect(sink).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(sink).not.toBeFocused();
  expect(live.sent.some((m) => m.type === 'key' && m.key === 'Escape')).toBe(false);

  // paste → a paste frame (never keystrokes)
  await canvas.click({ position: { x: 40, y: 40 } });
  await sink.evaluate((el) => {
    const dt = new DataTransfer();
    dt.setData('text/plain', 'pasted text');
    el.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
  });
  await expect.poll(() => live.sent.find((m) => m.type === 'paste')?.text).toBe('pasted text');
});

test('navigation: page nav updates the address bar; toolbar and address bar drive the remote', async ({ page }) => {
  const live = await mockLive(page, tabId, wsId);
  await openLive(page);
  live.push({
    type: 'state',
    session: { ...live.session, url: 'https://example.invalid/confirm', title: 'Confirmed', can_go_back: true },
  });
  await expect(page.getByRole('textbox', { name: 'Address' })).toHaveValue('https://example.invalid/confirm');
  await page.getByRole('button', { name: 'Back', exact: true }).click();
  await expect.poll(() => live.sent.some((m) => m.type === 'nav' && m.action === 'back')).toBe(true);
  await page.getByRole('button', { name: 'Reload page' }).click();
  await expect.poll(() => live.sent.some((m) => m.type === 'nav' && m.action === 'reload')).toBe(true);
  await expect(page.getByRole('button', { name: 'Forward' })).toBeDisabled();

  await page.getByRole('textbox', { name: 'Address' }).fill('example.invalid/next');
  await page.getByRole('textbox', { name: 'Address' }).press('Enter');
  await expect
    .poll(() => live.sent.find((m) => m.type === 'nav' && m.action === 'goto')?.url)
    .toBe('https://example.invalid/next');
});

test('agent driving: input is held back, Take over / Hand back use the control frame', async ({ page }) => {
  const live = await mockLive(page, tabId, wsId, { controller: 'agent' });
  await openLive(page);
  const bar = page.getByTestId('live-drive-bar');
  await expect(bar).toContainText('An agent is driving this page');
  const canvas = page.locator('[data-testid="remote-live"] canvas');
  await canvas.click({ position: { x: 50, y: 50 } });
  expect(live.sent.some((m) => m.type === 'mouse' && m.action === 'down')).toBe(false);

  await bar.getByRole('button', { name: 'Take over' }).click();
  await expect.poll(() => live.sent.some((m) => m.type === 'control' && m.action === 'take_over')).toBe(true);
  const me = await page.evaluate(async () => {
    const base = localStorage.getItem('otto_base') ?? '';
    const r = await fetch(`${base}/api/v1/auth/me`, { headers: { Authorization: `Bearer ${localStorage.getItem('otto_token')}` } });
    return (await r.json()).user.id as string;
  });
  live.push({ type: 'state', session: { ...live.session, controller: 'human', controller_user_id: me } });
  await expect(bar).toContainText('You have control');
  await canvas.click({ position: { x: 50, y: 50 } });
  await expect.poll(() => live.sent.some((m) => m.type === 'mouse' && m.action === 'down')).toBe(true);
  await bar.getByRole('button', { name: 'Hand back' }).click();
  await expect.poll(() => live.sent.some((m) => m.type === 'control' && m.action === 'hand_back')).toBe(true);
});

test('approval: an agent’s held request shows where / what / who and records the decision', async ({ page }) => {
  let decided: any = null;
  await page.route('**/api/v1/mcp/approvals?status=pending', (r) =>
    r.fulfill({
      json: [{
        id: 'ap-1', workspace_id: wsId, kind: 'browser_action', server_id: null, server_name: null, tool: null,
        title: 'Submit a form on book.example?', detail: 'Confirm the Thursday 17:30 slot',
        args_redacted_json: JSON.stringify({ origin: 'https://book.example', method: 'POST', target_host: 'book.example', screenshot_path: '/tmp/x.png' }),
        risk_label: null, status: 'pending', requested_by: 'sess-1', requested_by_kind: 'agent', decided_by: null,
        decision_note: null, created_at: new Date().toISOString(), decided_at: null, consumed_at: null, expires_at: null,
      }],
    }),
  );
  await page.route('**/api/v1/mcp/approvals/ap-1/decide', async (r) => {
    decided = r.request().postDataJSON();
    await r.fulfill({ json: { id: 'ap-1', status: 'approved' } });
  });
  const live = await mockLive(page, tabId, wsId, { controller: 'agent' });
  await openLive(page);
  live.push({ type: 'approval', approval_id: 'ap-1', status: 'pending', title: 'Submit a form on book.example?' });
  const card = page.getByTestId('live-approval');
  await expect(card.getByRole('heading', { name: 'Submit a form on book.example?' })).toBeVisible();
  await expect(card).toContainText('POST request to book.example');
  await expect(card).toContainText('Whoever runs book.example');
  await expect(card).toContainText('Confirm the Thursday 17:30 slot');
  await expect(card).toBeFocused();
  await card.getByRole('button', { name: 'Approve' }).click();
  await expect.poll(() => decided).toEqual({ approved: true });
  await expect(card).toBeHidden();
});

test('reconnects after a dropped socket; a closed session offers Reconnect', async ({ page }) => {
  const live = await mockLive(page, tabId, wsId);
  await openLive(page);
  await live.socket()!.close({ code: 1011, reason: 'boom' });
  await expect(page.getByTestId('remote-live')).toHaveAttribute('data-status', /reconnecting|connecting|live/);
  await expect.poll(() => live.opens(), { timeout: 10_000 }).toBeGreaterThanOrEqual(2);
  await expect(page.getByTestId('remote-live')).toHaveAttribute('data-status', 'live', { timeout: 10_000 });

  live.push({ type: 'closed', reason: 'idle' });
  const ended = page.getByTestId('live-ended');
  await expect(ended).toContainText('no one watching');
  await ended.getByRole('button', { name: 'Reconnect' }).click();
  await expect(page.getByTestId('remote-live')).toHaveAttribute('data-status', 'live', { timeout: 10_000 });
});

test('page dialogs, SSRF blocks and popups are shown in the pane', async ({ page }) => {
  const live = await mockLive(page, tabId, wsId);
  await openLive(page);
  live.push({ type: 'dialog', dialog_type: 'confirm', message: 'Leave the booking?', default_prompt: '', url: TAB_URL });
  const dlg = page.getByTestId('live-page-dialog');
  await expect(dlg).toContainText('example.invalid says');
  await dlg.getByRole('button', { name: 'Confirm' }).click();
  await expect.poll(() => live.sent.find((m) => m.type === 'dialog')).toEqual({ type: 'dialog', accept: true });

  live.push({ type: 'blocked', host: '10.0.0.1', reason: 'ssrf' });
  await expect(page.getByTestId('live-banner')).toContainText('Otto blocked a request to 10.0.0.1');
  live.push({ type: 'popup', url: 'https://example.invalid/popup' });
  await expect(page.getByTestId('live-banner').getByRole('button', { name: 'Open in new tab' })).toBeVisible();
});

test('phone width: the live pane fits, tap is a click, no horizontal scroll', async ({ browser }) => {
  const context = await browser.newContext({
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
    storageState: (test.info().project.use as { storageState?: string }).storageState,
  });
  const page = await context.newPage();
  await page.route('**/browser/page?url=**', (route) => route.fulfill({ status: 500, body: '{}' }));
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  const live = await mockLive(page, tabId, wsId);
  await openLive(page);
  await expectNoHorizontalOverflow(page);
  const canvas = page.locator('[data-testid="remote-live"] canvas');
  const box = (await canvas.boundingBox())!;
  await page.touchscreen.tap(box.x + 60, box.y + 60);
  await expect.poll(() => live.sent.filter((m) => m.type === 'mouse' && m.action === 'down').length).toBe(1);
  await context.close();
});
