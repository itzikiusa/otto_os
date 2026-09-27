import { test, expect, type WebSocketRoute } from '@playwright/test';
import type { RoomSnapshot } from '../src/lib/api/room-types';
const admitted: RoomSnapshot = {
  room_id: 'demo', member_id: 'guest', admission: 'admitted', session_id: 'session', session_title: 'Build together', provider: 'shell', host_member_id: 'host', driver_member_id: 'host', grant_epoch: 1,
  members: [
    {id: 'host', name: 'Maya', role: 'host', admission: 'admitted', connected: true, generation: 1, control_requested: false, audio_joined: false, muted: true, room_muted: false, presenter_requested: false, presenter_allowed: true},
    {id: 'guest', name: 'Alex', role: 'editor', admission: 'admitted', connected: true, generation: 1, control_requested: false, audio_joined: false, muted: true, room_muted: false, presenter_requested: false, presenter_allowed: false},
  ], messages: [], presentations: [], annotation_grants: [], annotations: [], annotations_enabled: true,
};
test('guest admission, driver handoff and chat stay isolated from owner APIs', async ({page}, testInfo) => {
  const requests: string[] = [], terminalInput: unknown[] = [], actions: Record<string, unknown>[] = [];
  let membership: WebSocketRoute | undefined;
  await page.addInitScript(() => { localStorage.setItem('otto_token', 'owner-secret-must-not-travel'); });
  await page.route('**/api/v1/**', async route => {
    requests.push(route.request().url());
    expect(route.request().headers().authorization).toBeUndefined();
    if (route.request().url().endsWith('/room-join')) await route.fulfill({json: {room_id: 'demo', member_id: 'guest', token: 'guest-token'}});
    else await route.fulfill({status: 403, json: {message: 'Unexpected owner request'}});
  });
  await page.routeWebSocket('**/ws/rooms/demo', ws => {
    membership = ws;
    ws.send(JSON.stringify({type: 'snapshot', room: {room_id: 'demo', member_id: 'guest', admission: 'pending'}}));
    ws.onMessage(raw => {
      const action = JSON.parse(String(raw)) as Record<string, unknown>; actions.push(action);
      if (action.type === 'heartbeat') ws.send(JSON.stringify({type: 'heartbeat'}));
      if (action.type === 'chat') ws.send(JSON.stringify({type: 'snapshot', room: {...admitted, messages: [{seq: 1, member_id: 'guest', name: 'Alex', text: action.text, nonce: action.nonce, created_at: new Date().toISOString()}]}}));
    });
  });
  await page.routeWebSocket('**/ws/rooms/demo/terminal', ws => {
    ws.send(JSON.stringify({type: 'scrollback', data: btoa('Welcome to the shared session\r\n'), cols: 80, rows: 24, epoch: 1}));
    ws.onMessage(raw => { const action = JSON.parse(String(raw)); if (action.type === 'input') terminalInput.push(action); });
  });
  await page.goto('/#/room/demo/invite_1234567890');
  await expect(page.getByRole('heading', {name: 'Join this session'})).toBeVisible();
  await expect(page).toHaveURL(/#\/room\/demo$/);
  expect(requests).toEqual([]);
  await page.getByLabel('Your display name').fill('Alex');
  await page.getByRole('button', {name: 'Request entry'}).click();
  await expect(page.getByText('Waiting for the host')).toBeVisible();
  await expect(page.getByText('Maya', {exact: true})).toHaveCount(0);
  expect(terminalInput).toEqual([]);
  membership!.send(JSON.stringify({type: 'snapshot', room: admitted}));
  await expect(page.getByText(/Maya controls the terminal/)).toBeVisible();
  await page.getByRole('button', {name: 'Request control', exact: true}).click();
  await expect.poll(() => actions.some(a => a.type === 'request_control')).toBeTruthy();
  if (testInfo.project.name === 'phone') await page.getByRole('button', {name: 'People & chat'}).click();
  await page.getByLabel('Message everyone').fill('<script>hello</script>');
  await page.getByRole('button', {name: 'Send', exact: true}).click();
  await expect(page.getByText('<script>hello</script>', {exact: true})).toBeVisible();
  await expect(page.getByLabel('Message everyone')).toHaveValue('');
  expect(terminalInput).toEqual([]);
  if (testInfo.project.name === 'phone') await page.getByRole('button', {name: 'Session', exact: true}).click();
  membership!.send(JSON.stringify({type: 'snapshot', room: {...admitted, driver_member_id: 'guest', grant_epoch: 4}}));
  await expect(page.getByRole('button', {name: 'Release control'})).toBeVisible();
  await page.locator('.xterm-helper-textarea').focus();
  await page.keyboard.type('hello');
  await expect.poll(() => terminalInput.length).toBeGreaterThan(0);
  expect(terminalInput.every(input => (input as {grant_epoch: number}).grant_epoch === 4)).toBeTruthy();
  await page.locator('.xterm-helper-textarea').evaluate(node => {
    const clipboardData = new DataTransfer(); clipboardData.items.add(new File(['fake'], 'test.png', {type: 'image/png'}));
    node.dispatchEvent(new ClipboardEvent('paste', {clipboardData, bubbles: true, cancelable: true}));
  });
  expect(requests).toHaveLength(1);
  const written = terminalInput.length;
  membership!.send(JSON.stringify({type: 'snapshot', room: {...admitted, grant_epoch: 5}}));
  await expect(page.getByRole('button', {name: 'Request control', exact: true})).toBeVisible();
  await page.keyboard.type('blocked');
  expect(terminalInput).toHaveLength(written);

  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  membership!.send(JSON.stringify({type: 'snapshot', room: {...admitted, recap: {id: 'recap', state: 'awaiting_consent', epoch: 7, pending_jobs: 0, consented_member_ids: [], reason: null, started_at: null}}}));
  await page.getByRole('button', {name: 'I consent to capture'}).click();
  await expect.poll(() => actions.some(a => a.type === 'recap_consent' && a.epoch === 7 && a.allow === true)).toBeTruthy();
  membership!.send(JSON.stringify({type: 'snapshot', room: {...admitted, recap: {id: 'recap', state: 'capturing', epoch: 7, pending_jobs: 0, consented_member_ids: ['host', 'guest'], reason: null, started_at: new Date().toISOString()}}}));
  await page.getByRole('button', {name: 'Withdraw consent'}).click();
  await expect.poll(() => actions.some(a => a.type === 'recap_consent' && a.epoch === 7 && a.allow === false)).toBeTruthy();
  expect(requests).toHaveLength(1);
  await page.screenshot({path: `/tmp/otto-room-${testInfo.project.name}.png`, fullPage: true});
  membership!.send(JSON.stringify({type: 'ended', reason: 'The host ended the room.'}));
  await expect(page.getByText('The host ended the room.')).toBeVisible();
});

test('screen pins are personal and annotation permission controls fit long names', async ({page, context}, testInfo) => {
  const fixture: RoomSnapshot = {...admitted, member_id: 'host', members: admitted.members!.map(m => ({...m, name: m.id === 'host' ? 'Maya — infrastructure and developer experience' : 'Alexandria — platform engineering and operations'})), presentations: [
    {id: 'screen-one', member_id: 'host', title: 'Architecture walkthrough', generation: 1, width: 640, height: 360, clear_epoch: 1},
    {id: 'screen-two', member_id: 'guest', title: 'Live investigation', generation: 1, width: 640, height: 360, clear_epoch: 1},
  ], annotations: [{id: 'demo-mark', source_id: 'screen-one', source_generation: 1, clear_epoch: 1, grant_epoch: 1, member_id: 'host', tool: 'pen', points: [{x: .2, y: .3}, {x: .45, y: .3}, {x: .45, y: .5}], expires_at: '2099-01-01T00:00:00Z'}], annotation_grants: [
    {source_id: 'screen-one', member_id: 'host', allowed: true, blocked: false, requested: false, epoch: 1},
    {source_id: 'screen-one', member_id: 'guest', allowed: false, blocked: false, requested: true, epoch: 1},
    {source_id: 'screen-two', member_id: 'host', allowed: true, blocked: false, requested: false, epoch: 1},
    {source_id: 'screen-two', member_id: 'guest', allowed: true, blocked: false, requested: false, epoch: 1},
  ]};
  const other = await context.newPage();
  for (const tab of [page, other]) {
    await tab.goto('/#/room/synthetic-layout');
    await tab.evaluate(async snapshot => {
      const modulePath = '/e2e/fixtures/room-screens.svelte.ts';
      const {mountScreens} = await import(modulePath);
      (window as unknown as {roomFixture: unknown}).roomFixture = mountScreens(snapshot);
    }, fixture);
    await expect(tab.getByRole('heading', {name: 'Shared screens'})).toBeVisible();
  }
  await expect(page.getByRole('button', {name: /Allow Alexandria/})).toBeVisible();
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  const allow = await page.getByRole('button', {name: /Allow Alexandria/}).boundingBox();
  const tools = await page.getByRole('group', {name: 'Annotation tool'}).first().boundingBox();
  expect(allow!.y).toBeGreaterThanOrEqual(tools!.y + tools!.height);
  await page.getByRole('button', {name: /Allow Alexandria/}).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as {roomFixture: {actions: {type: string}[]}}).roomFixture.actions.some(a => a.type === 'grant_annotation'))).toBeTruthy();
  await page.getByRole('button', {name: 'Draw', exact: true}).first().click();
  const canvas = page.getByRole('img', {name: 'Annotations on Architecture walkthrough'});
  const bounds = await canvas.boundingBox();
  await page.mouse.move(bounds!.x + bounds!.width * .3, bounds!.y + bounds!.height * .3);
  await page.mouse.down();
  await page.mouse.move(bounds!.x + bounds!.width * .6, bounds!.y + bounds!.height * .5, {steps: 3});
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => (window as unknown as {roomFixture: {actions: {type: string; source_id?: string; grant_epoch?: number; clear_epoch?: number}[]}}).roomFixture.actions.some(a => a.type === 'annotation' && a.source_id === 'screen-one' && a.grant_epoch === 1 && a.clear_epoch === 1))).toBeTruthy();
  await page.screenshot({path: `/tmp/otto-room-screens-${testInfo.project.name}.png`, fullPage: true});
  await page.getByRole('button', {name: /^Pin Maya/}).click();
  await expect(page.getByRole('button', {name: 'Show grid'})).toBeVisible();
  await expect(other.getByRole('button', {name: 'Show grid'})).toHaveCount(0);
  await page.evaluate(snapshot => (window as unknown as {roomFixture: {update(value: RoomSnapshot): void}}).roomFixture.update(snapshot), {...fixture, presentations: fixture.presentations!.slice(1)});
  await expect(page.getByRole('button', {name: 'Show grid'})).toHaveCount(0);
  await expect(page.getByRole('button', {name: /^Pin Alexandria/})).toHaveAttribute('aria-pressed', 'false');
  await other.close();
});

test('four real browser media clients negotiate synthetic audio and screen forwarding', async ({page}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-light', 'One synthetic WebRTC topology run is enough; layout tests cover all projects.');
  test.setTimeout(60000);
  await page.goto('/#/room/synthetic-media');
  await page.evaluate(async () => {
    const modulePath = '/e2e/fixtures/room-media-smoke.ts';
    const {startMediaSmoke} = await import(modulePath);
    (window as unknown as {mediaSmoke: unknown}).mediaSmoke = await startMediaSmoke();
  });
  try {
    await expect.poll(() => page.evaluate(() => {
      const summary = (window as unknown as {mediaSmoke: {summary(): {received: Record<string, string[]>}}}).mediaSmoke.summary();
      return ['host', 'guest1', 'guest2', 'guest3'].map(id => summary.received[id]?.length ?? 0);
    }), {timeout: 35000}).toEqual([4, 4, 4, 4]);
    await page.evaluate(() => (window as unknown as {mediaSmoke: {resize(id: string): void}}).mediaSmoke.resize('guest3'));
    await expect.poll(() => page.evaluate(() => (window as unknown as {mediaSmoke: {summary(): {sourceInfo: {id: string; generation: number}[]}}}).mediaSmoke.summary().sourceInfo.find(s => s.id === 'screen-guest3')?.generation), {timeout: 10000}).toBe(2);
    await expect.poll(() => page.evaluate(() => Object.values((window as unknown as {mediaSmoke: {summary(): {received: Record<string, string[]>}}}).mediaSmoke.summary().received).map(sources => sources.length)), {timeout: 10000}).toEqual([4, 4, 4, 4]);
    await page.evaluate(() => (window as unknown as {mediaSmoke: {stopPresenting(id: string): void}}).mediaSmoke.stopPresenting('guest3'));
    await expect.poll(() => page.evaluate(() => Object.values((window as unknown as {mediaSmoke: {summary(): {received: Record<string, string[]>}}}).mediaSmoke.summary().received).map(sources => sources.length)), {timeout: 10000}).toEqual([3, 3, 3, 3]);
    await page.evaluate(() => (window as unknown as {mediaSmoke: {remove(id: string): void}}).mediaSmoke.remove('guest2'));
    await expect.poll(() => page.evaluate(() => {
      const received = (window as unknown as {mediaSmoke: {summary(): {received: Record<string, string[]>}}}).mediaSmoke.summary().received;
      return ['host', 'guest1', 'guest2', 'guest3'].map(id => received[id]?.length ?? 0);
    }), {timeout: 10000}).toEqual([2, 2, 0, 2]);

  } finally {
    await testInfo.attach('media-topology', {body: await page.evaluate(() => JSON.stringify((window as unknown as {mediaSmoke: {summary(): unknown}}).mediaSmoke.summary())), contentType: 'application/json'});
    await page.evaluate(() => (window as unknown as {mediaSmoke: {stop(): void}}).mediaSmoke.stop());
  }
});
