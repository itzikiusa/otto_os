import { test } from 'node:test';
import assert from 'node:assert/strict';
import { gapBefore, musicVolume, playOrder } from '../src/modules/design-hall/create/scenePlay.ts';
import type { ProductionItem } from '../src/lib/api/types';

const line = (id: string, speaker: string, replies_to?: string, status = 'done'): ProductionItem =>
  ({ id, kind: 'voice_line', name: id, description: '', enabled: true, status, progress_pct: 100, details: { speaker, text: id, replies_to }, result: { clip_id: `c-${id}` } }) as ProductionItem;
const sound = (id: string, kind: 'sound' | 'music', loop = false): ProductionItem =>
  ({ id, kind, name: id, description: '', enabled: true, status: 'done', progress_pct: 100, details: { loop }, result: { clip_id: `c-${id}` } }) as ProductionItem;

test('lines play in plan order, each after the line it answers; unmade ones are left out', () => {
  const items = [line('answer', 'boy', 'ask'), line('ask', 'girl'), line('later', 'girl'), line('broken', 'boy', undefined, 'failed')];
  assert.deepEqual(playOrder(items).lines.map((l) => l.id), ['ask', 'answer', 'later']);
});

test('sounds loop under the lines, music always loops low and lower under speech', () => {
  const { beds } = playOrder([sound('fire', 'sound', true), sound('hoot', 'sound'), sound('tune', 'music')]);
  assert.deepEqual(beds.map((b) => [b.id, b.loop, b.music]), [['fire', true, false], ['hoot', false, false], ['tune', true, true]]);
  assert.ok(musicVolume(true) < musicVolume(false));
});

test('a short gap between speakers, a shorter one for the same speaker', () => {
  const [a, b, c] = [line('a', 'girl'), line('b', 'girl'), line('c', 'boy')].map((i) => playOrder([i]).lines[0]);
  assert.ok(gapBefore(a, b) < gapBefore(b, c));
});

test('a featured performance plays once while background music keeps looping', () => {
  const jazz = sound('jazz', 'music', true);
  jazz.details = { ...jazz.details, featured: true };
  const background = sound('underscore', 'music', true);
  const { beds } = playOrder([jazz, background]);
  assert.equal(beds[0].featured, true);
  assert.equal(beds[0].loop, false);
  assert.equal(beds[1].loop, true);
  assert.ok(musicVolume(false, true) > musicVolume(false));
  assert.equal(musicVolume(true, true), musicVolume(true));
});
