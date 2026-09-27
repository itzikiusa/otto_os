import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import type { RoomCredential, RoomSnapshot } from '../src/lib/api/room-types';

/** This file uses real daemon HTTP/WebSockets and a harmless shim agent from
 * global-setup. The synthetic rooms suite separately covers media topology. */
test('real daemon room admission, control, chat and end preserve the running session', async ({page, browser, baseURL}, info) => {
  test.skip(info.project.name !== 'rooms-live', 'Requires the isolated same-origin proxy in e2e/rooms-live.config.ts');
  const {ctx, base, token} = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`, {
    data: {kind: 'agent', provider: 'claude', title: 'Live room integration', cwd: '/tmp', meta: {origin: 'e2e'}},
  });
  expect(response.ok(), await response.text()).toBeTruthy();
  const session = await response.json() as {id: string};
  const created = await ctx.post(`${base}/api/v1/sessions/${session.id}/room`, {data: {name: 'Live host'}});
  expect(created.ok(), await created.text()).toBeTruthy();
  const host = await created.json() as RoomCredential;
  const guestContext = await browser.newContext({baseURL, storageState: {cookies: [], origins: []}});
  const guest = await guestContext.newPage();
  const guestRequests: string[] = [];
  const terminalFrames: {type: string; code?: string; epoch?: number}[] = [];
  const hostActions: unknown[] = [], browserErrors: string[] = [];
  page.on('pageerror', error => browserErrors.push(error.message));
  guest.on('pageerror', error => browserErrors.push(error.message));
  page.on('websocket', socket => socket.on('framesent', frame => {
    if (typeof frame.payload !== 'string' || socket.url().endsWith('/terminal')) return;
    try {hostActions.push(JSON.parse(frame.payload));} catch { /* non-JSON control */ }
  }));
  let guestRoom: RoomSnapshot | undefined;
  guest.on('request', request => { const path = new URL(request.url()).pathname; if (path.startsWith('/api/')) { guestRequests.push(path); expect(request.headers().authorization).toBeUndefined(); } });
  guest.on('websocket', socket => socket.on('framereceived', frame => {
    if (typeof frame.payload !== 'string') return;
    try {
      const event = JSON.parse(frame.payload);
      if (socket.url().endsWith('/terminal')) terminalFrames.push(event);
      else if (event.type === 'snapshot') guestRoom = event.room;
    } catch { /* binary terminal output */ }
  }));
  // Retain the real socket to assert the server rejects forged viewer/stale
  // input too. This neither mocks nor intercepts any network response.
  await guest.addInitScript(() => {
    const NativeSocket = window.WebSocket;
    window.WebSocket = class extends NativeSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols);
        if (String(url).endsWith('/terminal')) (window as unknown as {roomTerminal: WebSocket}).roomTerminal = this;
      }
    };
  });
  async function forgedInput(epoch: number) {
    await guest.evaluate(grant_epoch => (window as unknown as {roomTerminal: WebSocket}).roomTerminal.send(JSON.stringify({type: 'input', data: btoa('FORBIDDEN_ROOM_INPUT\n'), grant_epoch})), epoch);
  }
  async function typeTerminal(target: Page, value: string) {
    await target.locator('.xterm-helper-textarea').focus();
    await target.keyboard.type(value);
    await target.keyboard.press('Enter');
  }
  try {
    await page.goto('/#/room/live-host-bootstrap');
    await page.evaluate(async ({base, host}) => {
      const accessPath = '/src/modules/rooms/room-access.ts';
      const routerPath = '/src/lib/router.svelte.ts';
      (await import(accessPath)).rememberRoom(base, host);
      (await import(routerPath)).router.go(`room/${host.room_id}`);
    }, {base, host});
    await expect(page.getByRole('button', {name: 'Invite someone…'})).toBeVisible();
    await page.getByRole('button', {name: 'Invite someone…'}).click();
    await page.getByLabel('Maximum access').selectOption('editor');
    await page.getByRole('button', {name: 'Create invitation'}).click();
    const invitation = await page.getByRole('textbox', {name: 'Invitation link'}).inputValue();
    await page.getByRole('button', {name: 'Done', exact: true}).click();
    await guest.goto(`/${new URL(invitation).hash}`);
    await guest.getByLabel('Your display name').fill('Live guest');
    await guest.getByRole('button', {name: 'Request entry'}).click();
    await expect(guest.getByText('Waiting for the host')).toBeVisible();
    await page.getByRole('button', {name: 'Admit view only'}).click();
    await expect(guest.getByText('Live host controls the terminal', {exact: false})).toBeVisible();
    await expect(guest.locator('.xterm-rows')).toContainText('the real claude CLI is disabled in tests');
    await expect.poll(() => terminalFrames.some(frame => frame.type === 'scrollback')).toBeTruthy();
    const processEpoch = terminalFrames.find(frame => frame.type === 'scrollback')!.epoch;
    await forgedInput(guestRoom!.grant_epoch!);
    await expect.poll(() => terminalFrames.some(frame => frame.type === 'error' && frame.code === 'forbidden')).toBeTruthy();

    await guest.getByLabel('Message everyone').fill('REAL_ROOM_CHAT');
    await guest.getByRole('button', {name: 'Send', exact: true}).click();
    await expect(page.getByText('REAL_ROOM_CHAT', {exact: true})).toBeVisible();
    await expect(page.locator('.xterm-rows')).not.toContainText('REAL_ROOM_CHAT');
    await page.getByRole('combobox', {name: 'Access', exact: true}).selectOption('editor');
    await expect.poll(() => guestRoom?.members?.find(member => member.id === guestRoom?.member_id)?.role, {message: 'Host role selection reaches the guest snapshot'}).toBe('editor');
    await guest.getByRole('button', {name: 'Request control', exact: true}).click();
    await page.getByRole('button', {name: 'Give control…'}).click();
    await page.getByRole('button', {name: 'Give control', exact: true}).click();
    await expect(guest.getByRole('button', {name: 'Release control'})).toBeVisible();
    const grantedEpoch = guestRoom!.grant_epoch!;
    await typeTerminal(guest, 'REAL_ROOM_INPUT');
    await expect(page.locator('.xterm-rows')).toContainText('REAL_ROOM_INPUT');
    await page.getByRole('button', {name: 'Take back control'}).click();
    await expect(guest.getByRole('button', {name: 'Request control', exact: true})).toBeVisible();
    const errors = terminalFrames.filter(frame => frame.type === 'error').length;
    await forgedInput(grantedEpoch);
    await expect.poll(() => terminalFrames.filter(frame => frame.type === 'error').length).toBeGreaterThan(errors);
    await expect(page.locator('.xterm-rows')).not.toContainText('FORBIDDEN_ROOM_INPUT');
    await page.getByRole('button', {name: 'End room…'}).click();
    await page.getByRole('button', {name: 'End room', exact: true}).click();
    await expect(guest.getByRole('heading', {name: 'Room closed'})).toBeVisible();
    const after = await ctx.get(`${base}/api/v1/sessions/${session.id}`);
    expect((await after.json()).live).toBe(true);
    const epochAfter = await page.evaluate(async ({base, token, id}) => {
      return new Promise<number>((resolve, reject) => {
        const socket = new WebSocket(`${base.replace('http:', 'ws:')}/ws/term/${id}`, ['otto-bearer', token]);
        socket.binaryType = 'arraybuffer';
        let epoch: number | undefined, output = '';
        const timer = setTimeout(() => {socket.close(); reject(new Error('Owner terminal did not accept input after ending room'));}, 10000);
        socket.onopen = () => socket.send(JSON.stringify({type: 'scrollback', lines: 100}));
        socket.onmessage = event => {
          if (event.data instanceof ArrayBuffer) {
            output += new TextDecoder().decode(event.data);
            if (epoch !== undefined && output.includes('ROOM_AFTER_END')) {clearTimeout(timer); socket.close(); resolve(epoch);}
            return;
          }
          if (typeof event.data !== 'string') return;
          const frame = JSON.parse(event.data);
          if (frame.type === 'scrollback' && epoch === undefined) {
            epoch = frame.epoch;
            socket.send(JSON.stringify({type: 'input', data: btoa('ROOM_AFTER_END\n'), user: true}));
          }
        };
        socket.onerror = () => {clearTimeout(timer); reject(new Error('Owner terminal did not reconnect'));};
      });
    }, {base, token, id: session.id});
    expect(epochAfter).toBe(processEpoch);
    expect(guestRequests).toEqual(['/api/v1/room-join']);
    expect(browserErrors).toEqual([]);
  } finally {
    await info.attach('room-wire-evidence', {body: JSON.stringify({hostActions, browserErrors, guestRoom, terminalFrames}), contentType: 'application/json'});
    await guestContext.close();
    await ctx.delete(`${base}/api/v1/rooms/${host.room_id}`);
    await ctx.post(`${base}/api/v1/sessions/${session.id}/kill`);
    await ctx.dispose();
  }
});
