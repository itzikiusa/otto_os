import { test } from 'node:test';
import assert from 'node:assert/strict';
import { allocateVideoBudget, audioContributors, CaptureGeometry, CaptureSlot } from '../src/modules/rooms/room-media-policy.ts';
import type { RoomMember } from '../src/lib/api/room-types.ts';

function stream() {
  const track = { stopped: false, stop() { this.stopped = true; } };
  return { track, getTracks: () => [track] };
}

test('revocation stops capture that resolves after the permission picker', async () => {
  const slot = new CaptureSlot<ReturnType<typeof stream>>();
  const captured = stream();
  let resolve!: (value: typeof captured) => void;
  const pending = slot.acquire(() => new Promise(r => { resolve = r; }), () => true);
  slot.stop();
  resolve(captured);
  assert.equal(await pending, null);
  assert.equal(captured.track.stopped, true);
  assert.equal(slot.value, null);
});

test('failed source replacement preserves the existing capture', async () => {
  const slot = new CaptureSlot<ReturnType<typeof stream>>(), original = stream();
  assert.equal(await slot.acquire(async () => original, () => true), original);
  await assert.rejects(slot.acquire(async () => { throw new Error('Picker cancelled'); }, () => true));
  assert.equal(slot.value, original);
  assert.equal(original.track.stopped, false);
  slot.stop();
  assert.equal(original.track.stopped, true);
});

test('revoked permission is rechecked at capture commit and concurrent pickers cannot leak', async () => {
  const slot = new CaptureSlot<ReturnType<typeof stream>>();
  const old = stream(), fresh = stream();
  let resolve!: (value: typeof old) => void;
  const pending = slot.acquire(() => new Promise(r => { resolve = r; }), () => true);
  await slot.acquire(async () => fresh, () => true);
  resolve(old);
  assert.equal(await pending, null);
  assert.equal(old.track.stopped, true);
  assert.equal(slot.value, fresh);
  const denied = stream();
  assert.equal(await slot.acquire(async () => denied, () => false), null);
  assert.equal(denied.track.stopped, true);
  slot.stop();
});

test('four-presenter host star stays under aggregate video ceiling with one full pin per viewer', () => {
  const demands = Array.from({ length: 3 }, (_, viewer) =>
    Array.from({ length: 3 }, (_, source) => ({ key: `${viewer}:${source}`, viewerId: `${viewer}`, tier: 'full' as const, width: 1920, height: 1080 }))).flat();
  const budgets = allocateVideoBudget(demands);
  assert.equal(budgets.length, 9);
  assert.ok(budgets.reduce((sum, b) => sum + b.maxBitrate, 0) <= 8_000_000);
  for (let viewer = 0; viewer < 3; viewer++) {
    const outgoing = budgets.filter(b => b.viewerId === `${viewer}`);
    assert.equal(outgoing.filter(b => b.tier === 'full').length, 1);
    assert.ok(outgoing.filter(b => b.tier === 'preview').every(b => b.maxFramerate === 2 && b.maxBitrate <= 100_000));
  }
});

test('grid budgets are shared per viewer and hidden sources stop encoding', () => {
  const budgets = allocateVideoBudget(Array.from({ length: 4 }, (_, i) => ({ key: `${i}`, viewerId: 'viewer', tier: 'grid' as const, width: 1920, height: 1080 })));
  assert.ok(budgets.reduce((sum, b) => sum + b.maxBitrate, 0) <= 2_000_000);
  assert.ok(budgets.every(b => b.maxFramerate === 5 && b.scaleResolutionDownBy === 2));
  const hidden = allocateVideoBudget([{ key: 'hidden', viewerId: 'viewer', tier: 'hidden', width: 1920, height: 1080 }])[0];
  assert.equal(hidden.active, false);
  assert.equal(hidden.maxBitrate, 0);
});

test('dimensions preserve aspect ratio, never upscale and invalid metadata disables transmission', () => {
  const budgets = allocateVideoBudget([
    { key: 'small', viewerId: 'a', tier: 'full', width: 800, height: 600 },
    { key: 'portrait', viewerId: 'b', tier: 'preview', width: 1080, height: 1920 },
    { key: 'bad', viewerId: 'c', tier: 'full', width: NaN, height: 0 },
  ]);
  assert.equal(budgets[0].scaleResolutionDownBy, 1);
  assert.equal(budgets[1].scaleResolutionDownBy, 1920 / 180);
  assert.equal(budgets[2].active, false);
});

test('host mixes exclude the recipient, room-muted, departed and locally muted senders', () => {
  const member = (id: string, overrides: Partial<RoomMember> = {}): RoomMember => ({ id, name: id,
    role: 'viewer', admission: 'admitted', connected: true, generation: 1, control_requested: false,
    audio_joined: true, muted: false, room_muted: false, presenter_requested: false, presenter_allowed: false, ...overrides });
  const members = [member('host', { role: 'host' }), member('viewer'), member('rogue', { room_muted: true }), member('muted', { muted: true })];
  assert.deepEqual(audioContributors(members, 'viewer'), ['host']);
  assert.deepEqual(audioContributors(members, 'host'), ['viewer']);
  members[1].connected = false;
  assert.deepEqual(audioContributors(members, 'host'), []);
});

test('source geometry invalidates on resize but not intentional quality downscale', () => {
  const geometry = new CaptureGeometry(1920, 1080);
  geometry.qualityChanged(320, 180);
  assert.equal(geometry.observe(320, 180), null);
  assert.deepEqual(geometry.observe(320, 160), { width: 1920, height: 960 });
  assert.equal(geometry.observe(320, 160), null);
  geometry.qualityChanged(1920, 960);
  assert.equal(geometry.observe(1920, 960), null);
  assert.deepEqual(geometry.observe(1280, 720), { width: 1280, height: 720 });
  assert.equal(geometry.observe(NaN, 0), null);
});
