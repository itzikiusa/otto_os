import {test, expect} from '@playwright/test';
import type {RoomSnapshot} from '../src/lib/api/room-types';
import type {RecapDetail} from '../src/lib/api/room-recap-types';
test.use({serviceWorkers: 'block'});
const detail: RecapDetail = {
  metadata: {id: 'archive', owner_id: 'owner', room_id: 'room', session_id: 'session', session_title: 'Release planning', created_at: '2026-09-27T12:00:00Z', updated_at: '2026-09-27T12:01:00Z', status: 'stopped', capture_epoch: 1, bytes_used: 2048, quota_bytes: 536870912, last_seq: 3, speech_available: true, speech_error: null, summary_status: 'idle', summary_error: null, summary_through_seq: null},
  events: [
    {seq: 1, created_at: '2026-09-27T12:00:00Z', capture_epoch: 1, payload: {type: 'speech', member_id: 'host', member_name: 'Maya', sequence: 1, offset_ms: 0, segments: [{start_ms: 0, end_ms: 1000, text: 'Ship only after the regression passes.'}]}},
    {seq: 2, created_at: '2026-09-27T12:00:01Z', capture_epoch: 1, payload: {type: 'gap', kind: 'screen', member_id: null, source_id: 'screen', reason: 'A hidden screen had no available frame.'}},
    {seq: 3, created_at: '2026-09-27T12:00:02Z', capture_epoch: 1, payload: {type: 'terminal', data_base64: Buffer.from('<script>untrusted terminal text</script>').toString('base64')}},
  ], next_cursor: null, draft: null,
};
test('local recap transcript preserves gaps and summary failure remains retryable', async ({page}, testInfo) => {
  await page.addInitScript(() => localStorage.setItem('otto_token', 'owner-test'));
  await page.route('**/api/v1/room-recaps**', async route => {
    expect(route.request().headers().authorization).toBe('Bearer owner-test');
    if (route.request().method() === 'POST') await route.fulfill({status: 503, json: {message: 'Codex is signed out. Sign in locally and try again.'}});
    else await route.fulfill({json: new URL(route.request().url()).pathname.endsWith('/room-recaps') ? [detail.metadata] : detail});
  });
  await page.goto('/#/room/recap-fixture');
  await page.evaluate(async () => { const path = '/e2e/fixtures/room-recap.svelte.ts'; const {mountRecapPanel} = await import(path); mountRecapPanel(); });
  await expect(page.getByRole('heading', {name: 'Room recaps', exact: true})).toBeVisible();
  await expect(page.getByText('Ship only after the regression passes.')).toBeVisible();
  await expect(page.getByText('A hidden screen had no available frame.', {exact: false})).toBeVisible();
  await expect(page.getByText('<script>untrusted terminal text</script>', {exact: true})).toBeVisible();
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.screenshot({path: `/tmp/otto-room-recap-${testInfo.project.name}.png`, fullPage: true});
  await page.getByRole('button', {name: 'Summary', exact: true}).click();
  await page.getByRole('button', {name: 'Generate summary…', exact: true}).click();
  await page.getByRole('dialog').getByRole('button', {name: 'Generate summary', exact: true}).click();
  await expect(page.getByText('Codex is signed out. Sign in locally and try again.', {exact: false})).toBeVisible();
  await expect(page.getByRole('button', {name: 'Generate summary…', exact: true})).toBeEnabled();
});
test('host consent gates real synthetic PCM and screen ingestion, then flushes a bounded WAV', async ({page}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-light', 'One synthetic AudioWorklet run; layout tests cover all projects.');
  const uploads: {url: string; body: Record<string, unknown>}[] = [];
  await page.addInitScript(() => localStorage.setItem('otto_token', 'owner-test'));
  await page.route('**/api/v1/room-recaps/**', async route => { uploads.push({url: route.request().url(), body: route.request().postDataJSON() as Record<string, unknown>}); await route.fulfill(uploads.filter(upload => upload.url.endsWith('/screen')).length === 1 && route.request().url().endsWith('/screen') ? {status: 503, json: {message: 'Temporary screen upload failure'}} : {json: {queued: true}}); });
  await page.goto('/#/room/capture-fixture');
  const room: RoomSnapshot = {room_id: 'room', member_id: 'host', host_member_id: 'host', admission: 'admitted', members: [{id: 'host', name: 'Host', role: 'host', admission: 'admitted', connected: true, generation: 1, control_requested: false, audio_joined: true, muted: false, room_muted: false, presenter_requested: false, presenter_allowed: true}], presentations: [{id: 'screen', member_id: 'host', title: 'Synthetic', width: 640, height: 360, generation: 1, clear_epoch: 1}], recap: {id: 'archive', state: 'paused', epoch: 1, pending_jobs: 0, consented_member_ids: [], reason: 'Waiting for consent', started_at: null}};
  await page.evaluate(async value => { const path = '/e2e/fixtures/room-recap.svelte.ts'; const {startSyntheticCapture} = await import(path); (window as unknown as {syntheticCapture: unknown}).syntheticCapture = await startSyntheticCapture(value); }, room);
  expect(uploads).toEqual([]);
  const active = {...room, recap: {...room.recap!, state: 'capturing' as const, epoch: 2, consented_member_ids: ['host'], started_at: new Date().toISOString()}};
  await page.evaluate(value => (window as unknown as {syntheticCapture: {update(value: RoomSnapshot): void}}).syntheticCapture.update(value), active);
  await expect.poll(() => uploads.filter(upload => upload.url.endsWith('/screen')).length, {timeout: 25000}).toBe(2);
  await page.evaluate(() => (window as unknown as {syntheticCapture: {mute(): void}}).syntheticCapture.mute());
  await expect.poll(() => uploads.some(upload => upload.url.endsWith('/audio'))).toBeTruthy();
  const audio = uploads.find(upload => upload.url.endsWith('/audio'))!.body;
  expect(audio.capture_epoch).toBe(2); expect(audio.member_id).toBe('host'); expect(audio.member_generation).toBe(1);
  const wav = Buffer.from(audio.wav_base64 as string, 'base64'); expect(wav.toString('ascii', 0, 4)).toBe('RIFF'); expect(wav.readUInt32LE(24)).toBe(16000); expect(wav.readUInt16LE(22)).toBe(1); expect(wav.length).toBeLessThanOrEqual(960044);
  // A later consent withdrawal must discard a new partial utterance immediately.
  await page.evaluate(() => (window as unknown as {syntheticCapture: {unmute(): void}}).syntheticCapture.unmute());
  await page.waitForTimeout(1000);
  const audioCount = uploads.filter(upload => upload.url.endsWith('/audio')).length;
  await page.evaluate(value => (window as unknown as {syntheticCapture: {update(value: RoomSnapshot): void}}).syntheticCapture.update(value), {...room, recap: {...room.recap!, epoch: 3}});
  await page.waitForTimeout(750);
  expect(uploads.filter(upload => upload.url.endsWith('/audio'))).toHaveLength(audioCount);
  const screen = uploads.find(upload => upload.url.endsWith('/screen'))!.body; expect(screen.capture_epoch).toBe(2); expect(screen.source_generation).toBe(1); expect(Buffer.from(screen.jpeg_base64 as string, 'base64').length).toBeLessThanOrEqual(256 * 1024);
  await page.evaluate(() => (window as unknown as {syntheticCapture: {stop(): void}}).syntheticCapture.stop());
});
for (const outcome of ['stopped', 'paused'] as const) test(`ending the room waits for recap ${outcome === 'stopped' ? 'completion' : 'consent withdrawal'}`,  async ({page}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-light', 'One protocol state test; layout is covered separately.');
  test.setTimeout(75000);
  const actions: Record<string, unknown>[] = [];
  let membership: import('@playwright/test').WebSocketRoute;
  const room = {room_id: 'finalize', member_id: 'host', host_member_id: 'host', admission: 'admitted', session_title: 'Finalize transcript', members: [], presentations: [], recap: {id: 'archive', state: 'capturing', epoch: 2, pending_jobs: 2, consented_member_ids: ['host'], reason: null, started_at: new Date().toISOString()}};
  await page.addInitScript(() => localStorage.setItem('otto_token', 'owner-test'));
  await page.route('**/api/v1/room-recaps/**', route => route.fulfill({json: {...detail, metadata: {...detail.metadata, speech_available: false}}}));
  await page.routeWebSocket('**/ws/rooms/finalize', ws => {
    membership = ws; ws.send(JSON.stringify({type: 'snapshot', room}));
    ws.onMessage(raw => {
      const action = JSON.parse(String(raw)); actions.push(action);
      if (action.type === 'heartbeat') ws.send(JSON.stringify({type: 'heartbeat'}));
      if (action.type === 'recap_stop') ws.send(JSON.stringify({type: 'snapshot', room: {...room, recap: {...room.recap, state: 'finalizing'}}}));
    });
  });
  await page.goto('/#/room/finalize-fixture');
  await page.evaluate(async () => { const path = '/e2e/fixtures/room-recap.svelte.ts'; const {mountHostRoomPage} = await import(path); await mountHostRoomPage(); });
  await page.getByRole('button', {name: 'End room…', exact: true}).click();
  await page.getByRole('dialog').getByRole('button', {name: 'End room', exact: true}).click();
  await expect.poll(() => actions.some(action => action.type === 'recap_stop')).toBeTruthy();
  await expect(page.getByText('Finishing 2 accepted media jobs. No new content is being captured.', {exact: true})).toBeVisible();
  await expect(page.getByRole('button', {name: 'Withdraw consent'})).toBeEnabled();
  expect(actions.some(action => action.type === 'end')).toBe(false);
  if (outcome === 'paused') {
    await page.getByRole('button', {name: 'Withdraw consent'}).click();
    await expect.poll(() => actions.some(action => action.type === 'recap_consent' && action.allow === false)).toBeTruthy();
  }
  membership!.send(JSON.stringify({type: 'snapshot', room: {...room, recap: {...room.recap, state: outcome, pending_jobs: 0}}}));
  await expect.poll(() => actions.some(action => action.type === 'end')).toBeTruthy();
});

test('open room recap replaces archive identity and discards the old pending page', async ({page}) => {
  await page.clock.install();
  await page.clock.pauseAt(new Date());
  let membership: import('@playwright/test').WebSocketRoute;
  let oldPageRequested = false;
  let releaseOld!: () => void;
  const heldOld = new Promise<void>(resolve => { releaseOld = resolve; });
  const room = {room_id: 'finalize', member_id: 'host', host_member_id: 'host', admission: 'admitted', session_title: 'Archive identity fixture', members: [], presentations: [], recap: {id: 'archive-A', state: 'stopped', epoch: 2, pending_jobs: 0, consented_member_ids: [], reason: null, started_at: null}};
  const event = (seq: number, text: string) => ({seq, created_at: '2026-10-05T00:00:00Z', capture_epoch: 2, payload: {type: 'speech', member_id: 'host', member_name: 'Host', sequence: seq, offset_ms: 0, segments: [{start_ms: 0, end_ms: 1, text}]}});
  await page.route('**/api/v1/room-recaps/**', async route => {
    const url = new URL(route.request().url());
    const id = url.pathname.split('/room-recaps/')[1].split('/')[0];
    const isOld = id === 'archive-A';
    const metadata = {...detail.metadata, id, room_id: 'finalize', speech_available: false, last_seq: isOld ? 101 : 1, session_title: isOld ? 'Original archive' : 'Replacement archive'};
    if (url.pathname.endsWith('/revision')) {
      await route.fulfill({json: {metadata, events_revision: `${id}-events`, draft_revision: null}});
      return;
    }
    const after = Number(url.searchParams.get('after') ?? 0);
    const limit = Number(url.searchParams.get('limit') ?? 100);
    if (isOld && after === 100) { oldPageRequested = true; await heldOld; }
    const all = isOld ? Array.from({length: 101}, (_, i) => event(i + 1, `Old archive event ${i + 1}`)) : [event(1, 'New archive starts at its first event')];
    const remaining = all.filter(e => e.seq > after), events = remaining.slice(0, limit);
    // Unmount fences the old response; releasing it must never replace B.
    await route.fulfill({json: {metadata, events, next_cursor: remaining.length > events.length ? events.at(-1)!.seq : null, draft: null}}).catch(() => {});
  });
  await page.routeWebSocket('**/ws/rooms/finalize', ws => {
    membership = ws; ws.send(JSON.stringify({type: 'snapshot', room}));
    ws.onMessage(raw => { if (JSON.parse(String(raw)).type === 'heartbeat') ws.send(JSON.stringify({type: 'heartbeat'})); });
  });
  try {
    await page.goto('/#/room/identity-fixture');
    await page.evaluate(async () => { const path = '/e2e/fixtures/room-recap.svelte.ts'; const {mountHostRoomPage} = await import(path); await mountHostRoomPage(); });
    await page.getByRole('button', {name: 'Open recap', exact: true}).click();
    const modal = page.getByRole('dialog', {name: 'Room recap', exact: true});
    await expect(modal.getByText('Old archive event 1', {exact: true})).toBeVisible();
    await modal.getByRole('button', {name: 'Next events', exact: true}).click();
    await expect.poll(() => oldPageRequested).toBe(true);
    membership!.send(JSON.stringify({type: 'snapshot', room: {...room, recap: {...room.recap, id: 'archive-B'}}}));
    // The clock is paused: switching identity must remount immediately, without
    // waiting for a four-second poll or the obsolete page response to finish.
    await expect(modal.getByText('New archive starts at its first event', {exact: true})).toBeVisible();
    await expect(modal.getByRole('button', {name: 'Earlier events', exact: true})).toBeDisabled();
    releaseOld();
    await page.clock.runFor(1);
    await expect(modal.getByText('Old archive event 101', {exact: true})).toHaveCount(0);
    await expect(modal.getByText('New archive starts at its first event', {exact: true})).toBeVisible();
  } finally { releaseOld(); }
});
