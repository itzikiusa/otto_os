/// <reference lib="dom" />
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { RoomMediaClient, parseMediaEnvelope } from '../src/modules/rooms/room-media.ts';
import type { RoomMediaEnvironment } from '../src/modules/rooms/room-media.ts';
import type { RoomAction, RoomSnapshot } from '../src/lib/api/room-types.ts';

const snapshot: RoomSnapshot = { room_id: 'r', member_id: 'host', admission: 'admitted', host_member_id: 'host',
  members: [{ id: 'host', name: 'Host', role: 'host', admission: 'admitted', connected: true, generation: 1,
    control_requested: false, audio_joined: false, muted: true, room_muted: false, presenter_requested: false, presenter_allowed: true }], presentations: [] };

function fakeStream(kind: 'audio' | 'video') {
  const track = { kind, stopped: false, enabled: true, readyState: 'live', stop() { this.stopped = true; this.readyState = 'ended'; },
    getSettings: () => ({ width: 1280, height: 720 }), applyConstraints: async () => {}, addEventListener: () => {} };
  const stream = { getTracks: () => [track], getAudioTracks: () => kind === 'audio' ? [track] : [], getVideoTracks: () => kind === 'video' ? [track] : [] };
  return { track, stream: stream as unknown as MediaStream };
}

test('signal parser rejects oversized, malformed and unknown media envelopes', () => {
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'description', description: { type: 'offer', sdp: 'x'.repeat(65537) }, bindings: [] }), null);
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'ice', candidate: { candidate: 'x'.repeat(2049) } }), null);
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'bindings', bindings: [{ streamId: 's', sourceId: 'p', generation: -1 }] }), null);
  assert.equal(parseMediaEnvelope({ version: 1, kind: 'execute', command: 'anything' }), null);
  assert.ok(parseMediaEnvelope({ version: 1, kind: 'description', description: { type: 'offer', sdp: 'v=0\r\n' }, bindings: [] }));
});

test('ending room while screen permission is pending stops the late stream and publishes nothing', async () => {
  const captured = fakeStream('video'), actions: RoomAction[] = [];
  let resolve!: (stream: MediaStream) => void;
  const environment = { getDisplayMedia: () => new Promise<MediaStream>(r => { resolve = r; }) } as Partial<RoomMediaEnvironment>;
  const client = new RoomMediaClient({ memberId: 'host', send: a => actions.push(a), environment });
  client.update(snapshot);
  const pending = client.startPresentation();
  client.dispose();
  resolve(captured.stream);
  await pending;
  assert.equal(captured.track.stopped, true);
  assert.equal(actions.some(a => a.type === 'start_present'), false);
});

test('presenter permission is required before opening a screen picker', async () => {
  let pickerCalls = 0;
  const client = new RoomMediaClient({ memberId: 'guest', send: () => {}, environment: { getDisplayMedia: async () => { pickerCalls++; return fakeStream('video').stream; } } });
  client.update({ ...snapshot, member_id: 'guest', members: [{ ...snapshot.members![0], id: 'guest', role: 'viewer', presenter_allowed: false }] });
  await client.startPresentation();
  assert.equal(pickerCalls, 0);
  client.dispose();
});

test('late microphone capture cannot reactivate after leaving audio', async () => {
  const captured = fakeStream('audio'); let resolve!: (stream: MediaStream) => void;
  const destination = { disconnect() {}, connect() {} };
  const context = { state: 'running', destination, resume: async () => {}, close: async () => {},
    createMediaStreamSource: () => destination } as unknown as AudioContext;
  const client = new RoomMediaClient({ memberId: 'host', send: () => {}, environment: {
    createAudioContext: () => context, getUserMedia: () => new Promise(r => { resolve = r; }),
  } });
  client.update(snapshot);
  await client.joinAudio();
  const pending = client.unmute();
  client.leaveAudio(); resolve(captured.stream); await pending;
  assert.equal(captured.track.stopped, true);
  client.dispose();
});

test('a host generation change invalidates a pending guest picker even if disconnect was missed', async () => {
  const captured = fakeStream('video'); let resolve!: (stream: MediaStream) => void;
  const guest = { ...snapshot.members![0], id: 'guest', role: 'viewer' as const };
  const client = new RoomMediaClient({ memberId: 'guest', send: () => {}, environment: { getDisplayMedia: () => new Promise(r => { resolve = r; }) } });
  client.update({ ...snapshot, member_id: 'guest', members: [...snapshot.members!, guest] });
  const pending = client.startPresentation();
  client.update({ ...snapshot, member_id: 'guest', members: [{ ...snapshot.members![0], generation: 2 }, guest] });
  resolve(captured.stream); await pending;
  assert.equal(captured.track.stopped, true);
  client.dispose();
});

test('server coordinator shutdown stops the host microphone and audio context', async () => {
  const captured = fakeStream('audio'); let closed = false;
  const node = { disconnect() {}, connect() {} };
  const context = { state: 'running', destination: node, resume: async () => {}, close: async () => { closed = true; }, createMediaStreamSource: () => node } as unknown as AudioContext;
  const client = new RoomMediaClient({ memberId: 'host', send: () => {}, environment: { createAudioContext: () => context,
    getUserMedia: async () => captured.stream, createStream: () => captured.stream } });
  client.update(snapshot); await client.joinAudio(); await client.unmute();
  client.update({ ...snapshot, members: [{ ...snapshot.members![0], audio_joined: true, muted: false }] });
  client.update(snapshot);
  assert.equal(captured.track.stopped, true); assert.equal(closed, true);
  client.dispose();
});

test('moderation mute then lift cannot revive a picker opened before moderation', async t => {
  const captured = fakeStream('audio'); let resolve!: (stream: MediaStream) => void;
  const node = { disconnect() {}, connect() {} };
  const context = { state: 'running', destination: node, resume: async () => {}, close: async () => {}, createMediaStreamSource: () => node } as unknown as AudioContext;
  const client = new RoomMediaClient({ memberId: 'host', send: () => {}, environment: { createAudioContext: () => context,
    getUserMedia: () => new Promise(r => { resolve = r; }), createStream: () => captured.stream } });
  t.after(() => client.dispose());
  client.update(snapshot); await client.joinAudio(); const pending = client.unmute();
  client.update({ ...snapshot, members: [{ ...snapshot.members![0], room_muted: true }] });
  client.update(snapshot); resolve(captured.stream); await pending;
  assert.equal(captured.track.stopped, true); client.dispose();
});

test('recap accessor exposes only the host raw live microphone and withdraws muted audio', async t => {
  const captured = fakeStream('audio'), node = { disconnect() {}, connect() {} };
  const context = { state: 'running', destination: node, resume: async () => {}, close: async () => {}, createMediaStreamSource: () => node } as unknown as AudioContext;
  const client = new RoomMediaClient({ memberId: 'host', send: () => {}, environment: { createAudioContext: () => context,
    getUserMedia: async () => captured.stream, createStream: () => captured.stream } });
  t.after(() => client.dispose()); client.update(snapshot);
  assert.equal(client.getRecapAudioSources().size, 0);
  await client.joinAudio(); await client.unmute();
  assert.equal(client.getRecapAudioSources().size, 0, 'pending local join/unmute is not authoritative consented audio');
  client.update({ ...snapshot, members: [{ ...snapshot.members![0], audio_joined: true, muted: false }] });
  const source = client.getRecapAudioSources().get('host');
  assert.equal(source?.stream, captured.stream); assert.equal(source?.generation, 1);
  client.mute(); assert.equal(client.getRecapAudioSources().size, 0);
});
