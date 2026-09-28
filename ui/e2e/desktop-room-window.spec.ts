import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.beforeEach(async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '1');
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspace);
});

test('sidebar navigation preserves the shell and does not repeat authentication boot', async ({ page }) => {
  page.on('pageerror', error => console.error('pageerror:', error.message));
  let metaCalls = 0;
  page.on('request', (request) => { if (new URL(request.url()).pathname === '/api/v1/meta') metaCalls++; });
  await page.goto('/#/agents');
  await expect(page.locator('.navigator')).toBeVisible({ timeout: 30_000 });
  const initialCalls = metaCalls;
  await page.locator('.shell-main').evaluate((element) => {
    (window as Window & { originalShell?: Element }).originalShell = element;
  });
  for (const module of ['connections', 'rooms', 'agents']) {
    await page.locator(`.navigator [data-nav-id="${module}"]`).first().click();
    await expect(page).toHaveURL(new RegExp(`#/${module}`));
    await expect(page.locator('.shell-main')).toBeVisible();
    expect(await page.evaluate(() => (window as Window & { originalShell?: Element }).originalShell === document.querySelector('.shell-main'))).toBe(true);
    expect(metaCalls).toBe(initialCalls);
  }
});

const credential = { room_id: 'fixture-room', member_id: 'host-member', token: 'host-token-private-1234567890' };

type Fixture = { calls: {command: string; args: Record<string, unknown>}[]; failOpen: boolean; failRead: boolean };
async function nativeFixture(page: Page, host = false) {
  await page.addInitScript(({ host, credential }) => {
    const callbacks = new Map<number, (value: unknown) => void>();
    let id = 0;
    const fixture: Fixture = { calls: [], failOpen: false, failRead: false };
    Object.assign(window, {
      __roomNative: fixture,
      __OTTO_WIN__: host ? 'host-room-1' : 'main',
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: {label: host ? 'host-room-1' : 'main'}, currentWebview: {label: host ? 'host-room-1' : 'main'} },
        transformCallback(callback: (value: unknown) => void) { callbacks.set(++id, callback); return id; },
        unregisterCallback(key: number) { callbacks.delete(key); },
        convertFileSrc(path: string) { return path; },
        async invoke(command: string, args: Record<string, unknown> = {}) {
          fixture.calls.push({command, args});
          if (command === 'plugin:event|listen') return ++id;
          if (command === 'windows_registry') return ['main'];
          if (command === 'plugin:window|is_focused') return true;
          if (command === 'open_host_room_window') {
            if (fixture.failOpen) throw new Error('Could not open room window');
            return 'host-room-1';
          }
          if (command === 'get_host_room_context') {
            if (fixture.failRead) throw new Error('Room context unavailable');
            return {origin: location.origin, credential};
          }
          return null;
        },
      },
    });
  }, {host, credential});
}
async function roomCalls(page: Page, command: string) {
  return page.evaluate(cmd => (window as unknown as {__roomNative: Fixture}).__roomNative.calls.filter(c => c.command === cmd), command);
}
async function startSheet(page: Page) {
  await page.evaluate(async () => {
    const path = '/e2e/fixtures/start-room.ts';
    (await import(path)).showStartRoom();
  });
  await page.getByLabel('Your display name').fill('Maya');
}

test('starting and reopening a hosted room keeps the original workspace mounted', async ({ page }) => {
  await nativeFixture(page);
  let creates = 0;
  await page.route('**/api/v1/sessions/fixture-session/room', route => { creates++; return route.fulfill({json: credential}); });
  await page.route('**/api/v1/rooms', route => route.fulfill({json: [{room_id: credential.room_id, session_title: 'Build together', members: []}]}));
  await page.goto('/#/agents');
  await expect(page.locator('.shell-main')).toBeVisible({timeout: 30_000});
  await page.locator('.shell-main').evaluate(el => (window as Window & {originalShell?: Element}).originalShell = el);
  await startSheet(page);
  await page.getByRole('button', {name: 'Start room', exact: true}).click();
  await expect.poll(() => roomCalls(page, 'open_host_room_window')).toHaveLength(1);
  await expect(page).toHaveURL(/#\/agents$/);
  expect(await page.evaluate(() => (window as Window & {originalShell?: Element}).originalShell === document.querySelector('.shell-main'))).toBe(true);
  expect((await roomCalls(page, 'open_host_room_window'))[0].args.credential).toEqual(credential);
  await page.locator('.navigator [data-nav-id="rooms"]').click();
  await page.getByRole('button', {name: 'Open room', exact: true}).click();
  await expect.poll(() => roomCalls(page, 'open_host_room_window')).toHaveLength(2);
  await expect(page).toHaveURL(/#\/rooms$/);
  expect(creates).toBe(1);
  expect(await page.evaluate(() => JSON.stringify({...localStorage}))).not.toContain(credential.token);
});

test('retrying a failed room window does not create another room or navigate away', async ({ page }) => {
  await nativeFixture(page);
  let creates = 0;
  await page.route('**/api/v1/sessions/fixture-session/room', route => { creates++; return route.fulfill({json: credential}); });
  await page.goto('/#/agents');
  await expect(page.locator('.shell-main')).toBeVisible({timeout: 30_000});
  await startSheet(page);
  await page.evaluate(() => { (window as unknown as {__roomNative: Fixture}).__roomNative.failOpen = true; });
  await page.getByRole('button', {name: 'Start room', exact: true}).click();
  await expect(page.getByRole('alert')).toContainText('Could not open room window');
  await expect(page).toHaveURL(/#\/agents$/);
  await page.evaluate(() => { (window as unknown as {__roomNative: Fixture}).__roomNative.failOpen = false; });
  await page.getByRole('button', {name: 'Open room', exact: true}).click();
  await expect.poll(() => roomCalls(page, 'open_host_room_window')).toHaveLength(2);
  expect(creates).toBe(1);
});

for (const scheme of ['light', 'dark'] as const) {
  test(`host window loads only the room, with isolated credentials and retry (${scheme})`, async ({ page }, info) => {
    await nativeFixture(page, true);
    await page.addInitScript(scheme => {
      localStorage.setItem('otto_scheme', scheme);
      (window as unknown as {__roomNative: Fixture}).__roomNative.failRead = true;
    }, scheme);
    const chunks: string[] = [];
    page.on('request', req => { if (new URL(req.url()).pathname === '/src/shell/App.svelte') chunks.push(req.url()); });
    await page.routeWebSocket('**/ws/rooms/fixture-room', socket => {
      socket.send(JSON.stringify({type: 'snapshot', room: {room_id: 'fixture-room', member_id: 'host-member', host_member_id: 'host-member', admission: 'admitted', session_title: 'Build together', members: [], messages: []}}));
    });
    await page.goto('/#/room-host/fixture-room');
    await expect(page.getByRole('alert')).toContainText('Room context unavailable');
    await page.evaluate(() => { (window as unknown as {__roomNative: Fixture}).__roomNative.failRead = false; });
    await page.getByRole('button', {name: 'Retry', exact: true}).click();
    await expect(page.getByRole('button', {name: 'Invite someone…', exact: true})).toBeVisible();
    await expect(page.locator('.shell-main')).toHaveCount(0);
    expect(chunks).toEqual([]);
    expect(page.url()).not.toContain(credential.token);
    const storage = await page.evaluate(() => JSON.stringify({...localStorage}));
    expect(storage).not.toContain(credential.token);
    expect(storage).not.toContain('room-host');
    await page.screenshot({path: info.outputPath(`host-room-${scheme}.png`)});
  });
}
