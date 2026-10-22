/**
 * Kart visuals: the pure parts of the renderer and HUD. Track dressing
 * (render/dressing.ts) is held to the route audit's rule — chevron boards
 * and the extra floodlights / grandstands never stand on any route's road
 * and never on a layout prop; banners hang only where a wall stands; tyre
 * marks stay off gaps and fade in fork mouths; the painted grid boxes sit
 * exactly where the racers are seated. The chase camera's spring, shake and
 * FOV kick behave; the HUD shows art for every item (no missing portrait)
 * and lightning reads as good news for its user, bad only for its victims.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { F_DRAW, F_GAP, F_PIPE, FORK_ZONE, TRACK_IDS, trackFor } from '../src/modules/rooms/games/kart/tracks.ts';
import { barriers, PROP_BOXES, PROP_PAD, propBlocksRoad, propLayout, terrainHeight } from '../src/modules/rooms/games/kart/layout.ts';
import {
  branchFade, CHEVRON_DEPTH, CHEVRON_HALF, CHEVRON_HEIGHT, chevronBoards, extraProps, fireworkSpots, footprintBlocksRoad, GRID_SLOTS, gridSlots, overlapsProps, skidAlpha, wallBanners,
} from '../src/modules/rooms/games/kart/render/dressing.ts';
import { chaseFov, Shake, spring, springTo } from '../src/modules/rooms/games/kart/render/camera.ts';
import { itemArt, noticeFor, placeTier, ROULETTE } from '../src/modules/rooms/games/kart/hud.ts';
import { ITEM_KINDS } from '../src/modules/rooms/games/kart/items.ts';
import { createRace, type KartEvent, type KartSeat } from '../src/modules/rooms/games/kart/sim.ts';

const seats = (n: number): KartSeat[] => Array.from({ length: n }, (_, i) => ({ racer: 'kai', human: i === 0, slot: i === 0 ? 0 : -1, name: `R${i}` }));

test('the painted grid boxes sit exactly where createRace seats every racer', () => {
  for (const id of TRACK_IDS) {
    const slots = gridSlots(trackFor(id));
    assert.equal(slots.length, GRID_SLOTS * 4);
    const race = createRace(id, seats(GRID_SLOTS), 'normal', 1);
    // Seating order differs (humans mid-grid) but the set of spots is the grid.
    for (const r of race.racers) {
      let best = Infinity;
      for (let k = 0; k < slots.length; k += 4) best = Math.min(best, Math.hypot(slots[k] - r.x, slots[k + 2] - r.z));
      assert.ok(best < 0.01, `${id}: racer ${r.id} is ${best.toFixed(2)} m off its grid box`);
    }
  }
});

test('chevron boards stand off every road, on dry land, clear of every layout prop, pointing the way the road turns', () => {
  let total = 0;
  for (const id of TRACK_IDS) {
    const track = trackFor(id);
    for (const b of chevronBoards(track)) {
      total++;
      assert.equal(footprintBlocksRoad(track, b.x, b.z, b.yaw, CHEVRON_HALF, CHEVRON_DEPTH, b.y - 0.5, b.y + CHEVRON_HEIGHT, PROP_PAD), false, `${id}: chevron board at ${b.x.toFixed(1)},${b.z.toFixed(1)} stands on a road`);
      assert.equal(overlapsProps(track, b.x, b.z, CHEVRON_HALF), false, `${id}: chevron board overlaps a layout prop`);
      if (track.liquid !== null) assert.ok(terrainHeight(track, b.x, b.z) > track.liquid, `${id}: chevron board in the liquid`);
      assert.ok(b.dir === 1 || b.dir === -1);
    }
  }
  assert.ok(total >= 20, `expected boards on the sharp bends, got ${total}`);
});

test('extra floodlights and grandstands pass layout\'s road test and keep off every layout prop and each other', () => {
  let total = 0;
  for (const id of TRACK_IDS) {
    const track = trackFor(id);
    const extra = extraProps(track);
    extra.forEach((p, i) => {
      total++;
      assert.ok(PROP_BOXES[p.key], `${id}: ${p.key} has no measured footprint`);
      assert.equal(propBlocksRoad(track, p, PROP_PAD), false, `${id}: ${p.key} at ${p.x.toFixed(1)},${p.z.toFixed(1)} stands on a road`);
      const r = Math.hypot(PROP_BOXES[p.key][0], PROP_BOXES[p.key][1]) * p.scale;
      assert.equal(overlapsProps(track, p.x, p.z, r), false, `${id}: ${p.key} overlaps a layout prop`);
      for (const q of extra.slice(i + 1)) {
        const rq = Math.hypot(PROP_BOXES[q.key][0], PROP_BOXES[q.key][1]) * q.scale;
        assert.ok(Math.hypot(p.x - q.x, p.z - q.z) >= r + rq, `${id}: ${p.key} and ${q.key} overlap`);
      }
    });
    // Layout's own props are untouched by the dressing.
    assert.ok(propLayout(track).length > 0);
  }
  assert.ok(total > 20, `expected floodlights along the straights, got ${total}`);
});

test('banners hang only where a barrier wall stands', () => {
  let any = 0;
  for (const id of TRACK_IDS) {
    const track = trackFor(id);
    for (const path of track.paths) {
      const walls = barriers(track, path).filter(b => b.kind === 'wall');
      for (const b of wallBanners(track, path)) {
        const wall = walls.find(w => w.side === b.side);
        assert.ok(wall, `${id}/${path.name}: banner without a wall`);
        for (let i = 0; i < path.count; i++) if (b.mask[i]) { any++; assert.equal(wall!.mask[i], 1, `${id}/${path.name}: banner over an opening at sample ${i}`); }
      }
    }
  }
  assert.ok(any > 0);
});

test('tyre marks: 0..1, none over gaps, drawbridge leaves or half-pipes, real marks on bends, faded in fork mouths', () => {
  for (const id of TRACK_IDS) {
    const track = trackFor(id);
    for (const path of track.paths) {
      const a = skidAlpha(path);
      for (let i = 0; i < path.count; i++) {
        assert.ok(a[i] >= 0 && a[i] <= 1);
        if (path.flags[i] & (F_GAP | F_DRAW | F_PIPE)) assert.equal(a[i], 0, `${id}/${path.name}: tyre marks over a gap / leaf / pipe`);
        if (path.index > 0 && path.dist[i] < FORK_ZONE * 0.25) { assert.equal(a[i], 0); assert.equal(branchFade(path, i), 0); }
      }
    }
    assert.ok(track.paths.some(p => skidAlpha(p).some(v => v > 0.3)), `${id}: no tyre marks on any bend`);
  }
});

test('fireworks burst high over the finish line', () => {
  for (const id of TRACK_IDS) {
    const track = trackFor(id), spots = fireworkSpots(track);
    assert.ok(spots.length >= 9);
    for (let k = 0; k < spots.length; k += 3) {
      assert.ok(spots[k + 1] > track.py[0] + 25);
      assert.ok(Math.hypot(spots[k] - track.px[0], spots[k + 2] - track.pz[0]) < 80);
    }
  }
});

test('chase camera spring: no overshoot from rest, frame-rate independent, settles', () => {
  const s = spring(0);
  let last = 0;
  for (let i = 0; i < 120; i++) {
    const x = springTo(s, 10, 9, 1 / 60);
    assert.ok(x >= last - 1e-9 && x <= 10 + 1e-9, 'moves monotonically toward the target without overshoot');
    last = x;
  }
  assert.ok(Math.abs(last - 10) < 0.01);
  const a = spring(0), b = spring(0);
  springTo(a, 5, 7, 1 / 30);
  springTo(b, 5, 7, 1 / 60); springTo(b, 5, 7, 1 / 60);
  assert.ok(Math.abs(a.x - b.x) < 1e-9 && Math.abs(a.v - b.v) < 1e-9, 'two half steps equal one full step');
});

test('screen shake decays to nothing; the FOV kicks harder for stronger boosts and widens with speed', () => {
  const shake = new Shake();
  shake.add(0.6);
  assert.ok(Math.abs(shake.offset(0.3, 0, 1)) > 0 || Math.abs(shake.offset(0.31, 1, 1)) > 0);
  for (let i = 0; i < 120; i++) shake.update(1 / 60);
  assert.equal(shake.trauma, 0);
  assert.equal(shake.offset(1.234, 0, 1), 0);
  assert.ok(chaseFov(30, 1, 1.3, false) > chaseFov(30, 0, 0, false));
  assert.ok(chaseFov(30, 1, 1.55, false) > chaseFov(30, 1, 1.3, false));
  assert.ok(chaseFov(38, 0, 0, false) > chaseFov(20, 0, 0, false));
  assert.ok(chaseFov(40, 1, 1.6, true) < 100);
});

test('every item has HUD art, and every portrait it names ships with the game', () => {
  assert.deepEqual([...ROULETTE].sort(), [...ITEM_KINDS].sort());
  for (const kind of ITEM_KINDS) {
    const art = itemArt(kind);
    if (art.kind === 'portrait') assert.ok(fs.existsSync(new URL(`../public${art.src}`, import.meta.url)), `${kind}: ${art.src} is missing`);
  }
  assert.equal(itemArt('green').kind, 'shell');
  assert.equal(itemArt('bolt').kind, 'bolt');
  const golden = itemArt('golden');
  assert.ok(golden.kind === 'portrait' && golden.gold);
  const triple = itemArt('triple');
  assert.ok(triple.kind === 'portrait' && triple.count === 3);
});

test('lightning call-outs: good news for its user, neutral for onlookers, bad only for the struck', () => {
  const racers = [{ name: 'Me', place: 4 }, { name: 'Rosa', place: 1 }, { name: 'Dex', place: 2 }];
  const ev = (type: KartEvent['type'], racer: number, value?: number): KartEvent => ({ id: 1, type, racer, x: 0, y: 0, z: 0, value });
  assert.equal(noticeFor(ev('bolt', 0, 2), 0, racers)?.kind, 'good');
  assert.match(noticeFor(ev('bolt', 0, 2), 0, racers)!.text, /2 struck/);
  assert.notEqual(noticeFor(ev('bolt', 1, 2), 0, racers)?.kind, 'bad');
  assert.equal(noticeFor(ev('zap', 0), 0, racers)?.kind, 'bad');
  assert.equal(noticeFor(ev('zap', 2), 0, racers), null, 'someone else being struck is not the viewer\'s news');
  assert.equal(noticeFor(ev('draft', 0), 0, racers)?.kind, 'small');
  assert.equal(noticeFor(ev('burnout', 0), 0, racers)?.kind, 'bad');
  assert.equal(noticeFor(ev('finish', 0), 0, racers)?.text, 'Finished 4th!');
  assert.deepEqual([1, 2, 3, 4, 8].map(placeTier), ['gold', 'silver', 'bronze', 'pack', 'pack']);
});
