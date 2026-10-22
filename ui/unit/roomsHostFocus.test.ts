// accessibility-responsive-4-05 / ux-design-4-10: room controls whose own
// server echo removes them must hand focus on (never drop it to <body>), and
// a pending guest's "Cancel request" confirm must describe cancelling.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (p: string) => readFileSync(new URL(`../src/modules/rooms/${p}`, import.meta.url), 'utf8');

test('host actions that unmount their own button go through the refocus path', () => {
  const src = read('RoomParticipants.svelte');
  for (const type of ['grant_control', 'grant_present', 'remove', 'admit', 'reject']) {
    assert.doesNotMatch(src, new RegExp(`\\bsend\\(\\{type: '${type}'`), `${type} must use decide()`);
    assert.match(src, new RegExp(`decide\\(member, \\{type: '${type}'`), `${type} routed through decide()`);
  }
});

test('Release and Request control share one always-mounted button', () => {
  const src = read('RoomPage.svelte');
  assert.equal((src.match(/'Release control'/g) ?? []).length, 1);
  assert.doesNotMatch(src, /\{:else if !host && canType\}/);
  assert.match(src, /canType \? 'Release control' : self\?\.control_requested \? 'Control requested' : 'Request control'/);
});

test('a pending guest is asked to cancel the request, not to leave the room', () => {
  const src = read('RoomPage.svelte');
  assert.match(src, /title: 'Cancel your request to join\?', confirmLabel: 'Cancel request'/);
  assert.match(src, /The host won’t see your request any more\./);
});
