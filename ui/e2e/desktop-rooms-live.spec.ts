import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import type { RoomCredential, RoomSnapshot } from '../src/lib/api/room-types';

/** Real daemon HTTP/WebSockets and a harmless shim agent from global-setup.
 * Media uses explicitly fake Chromium microphones and generated display pixels. */
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
  // The terminal may render with WebGL. Read xterm's parsed screen through
  // its existing opt-in probe rather than assuming DOM renderer internals.
  for (const target of [page, guest]) await target.addInitScript(() => {
    (window as unknown as {__ottoTermProbe: unknown[]}).__ottoTermProbe = [];
  });
  const terminalText = (target: Page) => target.evaluate(() =>
    (window as unknown as {__ottoTermProbe: {disposed: boolean; text(): string}[]})
      .__ottoTermProbe.filter(probe => !probe.disposed).map(probe => probe.text()).join('\n'));

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
    await expect(guest.locator('.room-status').getByText('Live host controls the terminal', {exact: false})).toBeVisible();
    await expect.poll(() => terminalText(guest)).toContain('the real claude CLI is disabled in tests');
    await expect.poll(() => terminalFrames.some(frame => frame.type === 'scrollback')).toBeTruthy();
    const processEpoch = terminalFrames.find(frame => frame.type === 'scrollback')!.epoch;
    await forgedInput(guestRoom!.grant_epoch!);
    await expect.poll(() => terminalFrames.some(frame => frame.type === 'error' && frame.code === 'forbidden')).toBeTruthy();

    await guest.getByLabel('Message everyone').fill('REAL_ROOM_CHAT');
    await guest.getByRole('button', {name: 'Send', exact: true}).click();
    await expect(page.getByText('REAL_ROOM_CHAT', {exact: true})).toBeVisible();
    expect(await terminalText(page)).not.toContain('REAL_ROOM_CHAT');
    await page.getByRole('combobox', {name: 'Access', exact: true}).selectOption('editor');
    // Raising a member's access is confirmed by the host.
    await page.getByRole('button', {name: 'Allow', exact: true}).click();
    await expect.poll(() => guestRoom?.members?.find(member => member.id === guestRoom?.member_id)?.role, {message: 'Host role selection reaches the guest snapshot'}).toBe('editor');
    await guest.getByRole('button', {name: 'Request control', exact: true}).click();
    await page.getByRole('button', {name: 'Give control…'}).click();
    await page.getByRole('button', {name: 'Give control', exact: true}).click();
    await expect(guest.getByRole('button', {name: 'Release control'})).toBeVisible();
    const grantedEpoch = guestRoom!.grant_epoch!;
    await typeTerminal(guest, 'REAL_ROOM_INPUT');
    await expect.poll(() => terminalText(page)).toContain('REAL_ROOM_INPUT');
    await page.getByRole('button', {name: 'Take back control'}).click();
    await expect(guest.getByRole('button', {name: 'Request control', exact: true})).toBeVisible();
    const errors = terminalFrames.filter(frame => frame.type === 'error').length;
    await forgedInput(grantedEpoch);
    await expect.poll(() => terminalFrames.filter(frame => frame.type === 'error').length).toBeGreaterThan(errors);
    expect(await terminalText(page)).not.toContain('FORBIDDEN_ROOM_INPUT');
    // End room sits in the host's ⋯ menu (beside the terminal controls).
    await page.getByRole('button', {name: 'More room actions'}).click();
    await page.getByRole('menuitem', {name: 'End room…'}).click();
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

interface MediaObservation {
  peers: {state: RTCPeerConnectionState; signaling: RTCSignalingState; gathering: RTCIceGatheringState;
    mLines: number; transceivers: number; audioBytes: number; videoBytes: number; frames: number;
    sendingVideo: number; localCandidates: number; remoteCandidates: number}[];
  tracks: MediaStreamTrackState[];
  events: {event: string; peer: number; state: string; at: number}[];
}
declare global {interface Window {liveRoomMedia: {observe(): Promise<MediaObservation>}}}

async function instrumentLiveMedia(page: Page) {
  await page.addInitScript(() => {
    const peers: RTCPeerConnection[] = [], tracks: MediaStreamTrack[] = [];
    const events: {event: string; peer: number; state: string; at: number}[] = [];
    const NativePeer = window.RTCPeerConnection;
    window.RTCPeerConnection = class extends NativePeer {
      constructor(configuration?: RTCConfiguration) {
        super(configuration); const index = peers.push(this) - 1;
        for (const event of ['connectionstatechange', 'signalingstatechange', 'icegatheringstatechange', 'icecandidate']) {
          this.addEventListener(event, () => {
            events.push({event, peer: index, state: `${this.connectionState}/${this.signalingState}/${this.iceGatheringState}`, at: performance.now()});
            if (events.length > 200) events.shift();
          });
        }
      }
    };
    // getUserMedia remains a real browser call; the dedicated config selects
    // Chromium's fake device so no physical microphone can be opened.
    const capture = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia = async constraints => {
      const stream = await capture(constraints); tracks.push(...stream.getTracks()); return stream;
    };
    navigator.mediaDevices.getDisplayMedia = async () => {
      const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 360;
      const context = canvas.getContext('2d')!; let tick = 0;
      const paint = () => {context.fillStyle = `hsl(${tick++ % 360} 50% 40%)`; context.fillRect(0, 0, 640, 360); context.fillStyle = 'white'; context.font = '28px sans-serif'; context.fillText(`Synthetic room ${tick}`, 24, 60);};
      paint(); const stream = canvas.captureStream(10), timer = setInterval(paint, 100);
      const track = stream.getVideoTracks()[0], stop = track.stop.bind(track);
      track.stop = () => {clearInterval(timer); stop();}; tracks.push(track); return stream;
    };
    window.liveRoomMedia = {async observe() {
      const observations = [];
      for (const peer of peers) {
        let audioBytes = 0, videoBytes = 0, frames = 0, localCandidates = 0, remoteCandidates = 0;
        for (const row of (await peer.getStats()).values()) {
          if (row.type === 'inbound-rtp') {
            if (row.kind === 'audio') audioBytes += row.bytesReceived ?? 0;
            if (row.kind === 'video') {videoBytes += row.bytesReceived ?? 0; frames += row.framesDecoded ?? 0;}
          }
          if (row.type === 'local-candidate') localCandidates++;
          if (row.type === 'remote-candidate') remoteCandidates++;
        }
        observations.push({state: peer.connectionState, signaling: peer.signalingState, gathering: peer.iceGatheringState,
          mLines: peer.localDescription?.sdp.match(/^m=/gm)?.length ?? 0, transceivers: peer.getTransceivers().length,
          sendingVideo: peer.getSenders().filter(sender => sender.track?.kind === 'video').length,
          audioBytes, videoBytes, frames, localCandidates, remoteCandidates});
      }
      return {peers: observations, tracks: tracks.map(track => track.readyState), events};
    }};
  });
}

// Separate pages and browser contexts deliberately use the production daemon
// signal route, avoiding the synchronous in-memory signaling fixture entirely.
test('real daemon media connects separate pages and survives concurrent screen replacements', async ({page, browser, baseURL}, info) => {
  test.skip(info.project.name !== 'rooms-live', 'Requires the isolated room daemon and explicit fake-device launch configuration');
  test.setTimeout(180000);
  const {ctx, base} = await apiCtx(), workspace = await seedWorkspace(ctx, base);
  const started = await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`, {data: {kind: 'agent', provider: 'claude', title: 'Live room media integration', cwd: '/tmp', meta: {origin: 'e2e'}}});
  expect(started.ok(), await started.text()).toBeTruthy();
  const session = await started.json() as {id: string};
  const created = await ctx.post(`${base}/api/v1/sessions/${session.id}/room`, {data: {name: 'Media host'}});
  expect(created.ok(), await created.text()).toBeTruthy();
  const host = await created.json() as RoomCredential;
  const guestContext = await browser.newContext({baseURL, permissions: ['microphone'], storageState: {cookies: [], origins: []}});
  const guest = await guestContext.newPage(), pages = [page, guest];
  const wire: {side: string; type: string; kind?: string; message?: string}[] = [], errors: string[] = [];
  let guestRoom: RoomSnapshot | undefined;
  for (const [index, target] of pages.entries()) {
    target.on('pageerror', error => errors.push(error.message));
    target.on('websocket', socket => socket.on('framereceived', frame => {
      if (typeof frame.payload !== 'string' || socket.url().endsWith('/terminal')) return;
      try {
        const event = JSON.parse(frame.payload);
        if (index === 1 && event.type === 'snapshot') guestRoom = event.room;
        wire.push({side: index ? 'guest' : 'host', type: event.type, kind: event.media?.kind, message: event.type === 'error' ? event.message : undefined});
      } catch { /* binary frame */ }
    }));
    await instrumentLiveMedia(target);
  }
  const observations = () => Promise.all(pages.map(target => target.evaluate(() => window.liveRoomMedia.observe())));
  const active = (value: MediaObservation) => value.peers.filter(peer => peer.state !== 'closed');
  async function connected() {
    await expect.poll(async () => (await observations()).every(value => active(value).length === 1 && active(value).every(peer => peer.state === 'connected' && peer.signaling === 'stable')), {timeout: 20000, message: 'Both real daemon-signaled media endpoints connect'}).toBeTruthy();
  }
  const samples: MediaObservation[][] = [];
  try {
    await page.goto('/#/room/live-media-bootstrap');
    await page.evaluate(async ({base, host}) => {
      const accessPath = '/src/modules/rooms/room-access.ts', routerPath = '/src/lib/router.svelte.ts';
      (await import(accessPath)).rememberRoom(base, host); (await import(routerPath)).router.go(`room/${host.room_id}`);
    }, {base, host});
    await page.getByRole('button', {name: 'Invite someone…'}).click();
    await page.getByLabel('Maximum access').selectOption('editor');
    await page.getByRole('button', {name: 'Create invitation'}).click();
    const invitation = await page.getByRole('textbox', {name: 'Invitation link'}).inputValue();
    await page.getByRole('button', {name: 'Done', exact: true}).click();
    await guest.goto(`/${new URL(invitation).hash}`);
    await guest.getByLabel('Your display name').fill('Media guest');
    await guest.getByRole('button', {name: 'Request entry'}).click();
    await page.getByRole('button', {name: 'Admit view only'}).click();
    await guest.getByRole('button', {name: 'Request to present', exact: true}).click();
    await page.getByRole('button', {name: 'Allow presentation', exact: true}).click();
    await expect(guest.getByRole('button', {name: 'Share screen…'})).toBeVisible();
    await page.getByRole('button', {name: 'Join audio', exact: true}).click();
    await expect.poll(() => guestRoom?.members?.find(member => member.id === guestRoom?.host_member_id)?.audio_joined).toBe(true);
    await guest.getByRole('button', {name: 'Join audio', exact: true}).click();
    await Promise.all(pages.map(target => target.getByRole('button', {name: 'Turn microphone on', exact: true}).click()));
    await connected();
    await expect.poll(async () => (await observations()).every(value => active(value)[0]?.audioBytes > 0), {timeout: 10000, message: 'Fake microphone audio arrives through the real peer connections'}).toBeTruthy();
    let negotiatedLines: number[] | undefined;
    for (let cycle = 0; cycle < 5; cycle++) {
      await Promise.all(pages.map(target => target.getByRole('button', {name: 'Share screen…', exact: true}).click()));
      await Promise.all(pages.map(target => expect(target.getByRole('region', {name: 'Shared screens'}).locator('video')).toHaveCount(2)));
      await connected();
      const before = await observations();
      await expect.poll(async () => (await observations()).every((value, index) => active(value)[0].frames > active(before[index])[0].frames && active(value)[0].videoBytes > active(before[index])[0].videoBytes), {timeout: 10000, message: `Both remote screen streams advance on cycle ${cycle + 1}`}).toBeTruthy();
      const sample = await observations(); samples.push(sample);
      const lines = sample.map(value => active(value)[0].mLines);
      negotiatedLines ??= lines;
      expect(lines).toEqual(negotiatedLines);
      expect(lines.every(count => count > 0 && count <= 4)).toBeTruthy();
      await Promise.all(pages.map(target => target.getByRole('button', {name: 'Stop sharing screen', exact: true}).click()));
      await Promise.all(pages.map(target => expect(target.getByRole('region', {name: 'Shared screens'})).toHaveCount(0)));
      await expect.poll(async () => (await observations()).every(value => active(value)[0].sendingVideo === 0)).toBeTruthy();
    }
    const beforeAudio = await observations();
    await expect.poll(async () => (await observations()).every((value, index) => active(value)[0].audioBytes > active(beforeAudio[index])[0].audioBytes)).toBeTruthy();
    expect(errors).toEqual([]);
    expect(wire.filter(event => event.type === 'error')).toEqual([]);
    await page.getByRole('button', {name: 'End room…'}).click();
    await page.getByRole('button', {name: 'End room', exact: true}).click();
    await expect(guest.getByRole('heading', {name: 'Room closed'})).toBeVisible();
    await expect.poll(async () => (await observations()).every(value => value.peers.every(peer => peer.state === 'closed') && value.tracks.every(state => state === 'ended'))).toBeTruthy();
  } finally {
    await info.attach('live-media-evidence', {body: JSON.stringify({wire, errors, samples, final: await observations().catch(() => null)}), contentType: 'application/json'});
    await guestContext.close(); await ctx.delete(`${base}/api/v1/rooms/${host.room_id}`);
    await ctx.post(`${base}/api/v1/sessions/${session.id}/kill`); await ctx.dispose();
  }
});

/** The whole browser flow against the REAL daemon, step by step, with a
 * screenshot per step: Start room from the session's More menu (the room opens
 * in its own tab — the postMessage hand-off of a reactive credential), invite,
 * admit, hand over control, type, paste an image, message the agent from the
 * Chat view, and find in the terminal. Watchable: `--headed` + slowMo. */
test.describe('room parity, live', () => {
  test('start in its own tab, join, control, paste an image, prompt from Chat, find', async ({page, browser, baseURL}, info) => {
    test.skip(info.project.name !== 'rooms-live', 'Requires the isolated same-origin proxy in e2e/rooms-live.config.ts');
    const {ctx, base} = await apiCtx();
    const workspace = await seedWorkspace(ctx, base);
    const created = await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`, {
      data: {kind: 'agent', provider: 'claude', title: 'Room parity live', cwd: '/tmp', meta: {origin: 'e2e'}},
    });
    expect(created.ok(), await created.text()).toBeTruthy();
    const session = await created.json() as {id: string};
    const shots = info.outputPath('steps');
    let step = 0;
    const shot = async (target: Page, name: string) => {
      const path = `${shots}/${String(++step).padStart(2, '0')}-${name}.png`;
      await target.screenshot({path});
      await info.attach(`${String(step).padStart(2, '0')} ${name}`, {path, contentType: 'image/png'});
    };
    const probe = () => {(window as unknown as {__ottoTermProbe: unknown[]}).__ottoTermProbe = [];};
    const terminalText = (target: Page) => target.evaluate(() =>
      (window as unknown as {__ottoTermProbe: {disposed: boolean; text(): string}[]})
        .__ottoTermProbe.filter(p => !p.disposed).map(p => p.text()).join('\n'));
    const errors: string[] = [];
    await page.context().addInitScript(probe);
    await page.addInitScript((id) => {
      localStorage.setItem('otto_workspace', id); localStorage.setItem('otto_rail_expanded', '1'); localStorage.setItem('otto_firstrun_dismissed', '1');
    }, workspace);
    page.context().on('page', p => p.on('pageerror', e => errors.push(`host-tab: ${e.message}`)));
    page.on('pageerror', e => errors.push(`host: ${e.message}`));
    const guestContext = await browser.newContext({baseURL, storageState: {cookies: [], origins: []}});
    await guestContext.addInitScript(probe);
    const guest = await guestContext.newPage();
    guest.on('pageerror', e => errors.push(`guest: ${e.message}`));
    let roomId = '';
    try {
      // 1. Start the room from the session's own More menu.
      await page.goto(`/#/agents/${session.id}`);
      await page.getByRole('button', {name: /^More… —/}).first().click();
      await page.getByRole('menuitem', {name: 'Start room…'}).click();
      await page.getByLabel('Your display name').fill('Live host');
      await shot(page, 'start-room-dialog');
      const tabPromise = page.waitForEvent('popup');
      await page.getByRole('button', {name: 'Start room', exact: true}).click();
      const host = await tabPromise;
      await expect(host).toHaveURL(/#\/room-host\//);
      roomId = decodeURIComponent(host.url().split('#/room-host/')[1]);
      await expect(host.getByRole('button', {name: 'Invite someone…'})).toBeVisible({timeout: 20_000});
      await expect(host.getByText('didn’t answer', {exact: false})).toHaveCount(0);
      await shot(host, 'room-opened-in-its-own-tab');
      // The original tab stays free: navigate it elsewhere, the room keeps going.
      await page.goto('/#/rooms');
      await expect(host.getByRole('button', {name: 'Invite someone…'})).toBeVisible();

      // 2. Invite + admit a guest from a separate browser.
      await host.getByRole('button', {name: 'Invite someone…'}).click();
      await host.getByLabel('Maximum access').selectOption('editor');
      await host.getByRole('button', {name: 'Create invitation'}).click();
      const invitation = await host.getByRole('textbox', {name: 'Invitation link'}).inputValue();
      await shot(host, 'invitation');
      await host.getByRole('button', {name: 'Done', exact: true}).click();
      await guest.goto(`/${new URL(invitation).hash}`);
      await guest.getByLabel('Your display name').fill('Live guest');
      await guest.getByRole('button', {name: 'Request entry'}).click();
      await expect(guest.getByText('Waiting for the host')).toBeVisible();
      await shot(guest, 'guest-waiting');
      await host.getByRole('button', {name: 'Admit view only'}).click();
      await expect(guest.locator('.room-status').getByText('Live host controls the terminal', {exact: false})).toBeVisible();
      // The host (driver) types in the room terminal; the guest sees it live.
      await host.locator('.xterm-helper-textarea').focus();
      await host.keyboard.type('echo HOST_TYPED_IN_ROOM');
      await host.keyboard.press('Enter');
      await expect.poll(() => terminalText(guest), {timeout: 20_000}).toContain('HOST_TYPED_IN_ROOM');
      await host.getByRole('combobox', {name: 'Access', exact: true}).selectOption('editor');
      await host.getByRole('button', {name: 'Allow', exact: true}).click();
      await shot(guest, 'guest-admitted');

      // 3. Control hand-off.
      await guest.getByRole('button', {name: 'Request control', exact: true}).click();
      await host.getByRole('button', {name: 'Give control…'}).click();
      await host.getByRole('button', {name: 'Give control', exact: true}).click();
      await expect(guest.getByRole('button', {name: 'Release control'})).toBeVisible();

      // 4. Typing reaches the host's terminal.
      await guest.locator('.xterm-helper-textarea').focus();
      await guest.keyboard.type('echo LIVE_ROOM_TYPED');
      await guest.keyboard.press('Enter');
      await expect.poll(() => terminalText(host)).toContain('LIVE_ROOM_TYPED');
      await shot(host, 'host-sees-guest-typing');

      // 5. A pasted image is stored on the HOST and its path typed in.
      const png = await guest.evaluate(async () => {
        const c = document.createElement('canvas'); c.width = 32; c.height = 32;
        const g = c.getContext('2d')!; g.fillStyle = '#3a7'; g.fillRect(0, 0, 32, 32);
        const blob: Blob = await new Promise(r => c.toBlob(b => r(b!), 'image/png'));
        const file = new File([blob], 'shot.png', {type: 'image/png'});
        const data = new DataTransfer(); data.items.add(file);
        document.querySelector('.xterm-helper-textarea')!.dispatchEvent(new ClipboardEvent('paste', {clipboardData: data, bubbles: true, cancelable: true}));
        return blob.size;
      });
      expect(png).toBeGreaterThan(0);
      await expect.poll(() => terminalText(host), {timeout: 15_000}).toMatch(/snips\/[^\s]+\.png/);
      await shot(host, 'host-sees-pasted-image-path');

      // 6. Chat view: the conversation (or its empty state) and the composer.
      await guest.getByRole('button', {name: 'Chat', exact: true}).click();
      await expect(guest.getByLabel('Agent conversation')).toBeVisible();
      await guest.getByLabel('Message the agent').fill('LIVE_ROOM_PROMPT from chat');
      await guest.getByLabel('Agent conversation').getByRole('button', {name: 'Send', exact: true}).click();
      await expect.poll(() => terminalText(host), {timeout: 15_000}).toContain('LIVE_ROOM_PROMPT from chat');
      await shot(guest, 'guest-chat-view');

      // 7. Find in the terminal from the room's ⋯ menu.
      await guest.getByRole('button', {name: 'Terminal', exact: true}).click();
      // Back from Chat at full size: hiding it must not have shrunk its font.
      // Room terminals render on the DOM: real rows at a readable size.
      const fontPx = await guest.locator('.xterm-rows').first().evaluate(e => parseFloat(getComputedStyle(e).fontSize));
      expect(fontPx).toBeGreaterThanOrEqual(11);
      await guest.getByRole('button', {name: 'Terminal options'}).click();
      await guest.getByRole('menuitem', {name: 'Find in terminal'}).click();
      await guest.keyboard.type('LIVE_ROOM_TYPED');
      // Local + scrollback search answer; the counter settles on the matches.
      await expect(guest.getByText(/^\d+\/\d+$/).first()).toBeVisible({timeout: 10_000});
      await shot(guest, 'guest-find');
      await shot(host, 'host-final');
      expect(errors).toEqual([]);
    } finally {
      await guestContext.close();
      if (roomId) await ctx.delete(`${base}/api/v1/rooms/${roomId}`);
      await ctx.post(`${base}/api/v1/sessions/${session.id}/kill`);
      await ctx.dispose();
    }
  });
});
