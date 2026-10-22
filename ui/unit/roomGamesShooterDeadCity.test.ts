/**
 * Headless play-test for Dead City: a whole 16-bot battle royale on the host
 * simulation, a guest decoding every snapshot the host would send (20 Hz,
 * loot and structures only when they changed, never together). Catches what
 * the world tests cannot: bots stuck inside the ruins, a match that never
 * ends in the street grid, or a snapshot that stops decoding / outgrows the
 * relay cap on the new map.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { aliveCount, createMatch, STEP, tickMatch, type ShooterState } from '../src/modules/rooms/games/shooter/sim.ts';
import * as shooterNet from '../src/modules/rooms/games/shooter/net.ts';
import { mapFor, terrainHeight } from '../src/modules/rooms/games/shooter/map.ts';
import { shooterSeats } from '../src/modules/rooms/games/setup.ts';

const RELAY_CAP = 16 * 1024;

test('Dead City play-test: 16 bots drop into the ruins, loot, fight and one survives; guests decode every snapshot under the relay cap', () => {
  const map = mapFor('deadcity');
  const s: ShooterState = createMatch({ map: 'deadcity', difficulty: 'hard', seed: 31, seats: shooterSeats([], 16) });
  let guest: ShooterState | null = null;
  let lastLootRev = -1, lastBuildRev = -1, preferBuilds = false, sentAt = 0, n = 0, maxBytes = 0, last = 0;
  const seen = { pickup: 0, chest: 0, kill: 0, shot: 0, indoorsUp: 0 };
  // Seconds each fighter spends walled in (inside a tall box at body height); a bot jammed in the ruins would pile these up.
  const jammed = new Float64Array(16);
  while (s.phase !== 'done' && n++ < 60 * 600) {
    tickMatch(s, []);
    for (const e of s.events) {
      if (e.id <= last) continue;
      if (e.type === 'pickup') seen.pickup++;
      if (e.type === 'chest') seen.chest++;
      if (e.type === 'kill') seen.kill++;
      if (e.type === 'shot') seen.shot++;
    }
    last = s.eventSeq;
    if (n % 60 === 0) for (const p of s.players) {
      assert.ok(Number.isFinite(p.x + p.y + p.z + p.hp + p.shield), `fighter ${p.id} state is finite`);
      if (p.mode !== 'ground') continue;
      assert.ok(Math.abs(p.x) <= 240 && Math.abs(p.z) <= 240, 'inside the map');
      if (p.y > terrainHeight(map, p.x, p.z) + 3) seen.indoorsUp++;
      const inside = map.boxes.some(b => p.x > b.minX + 0.05 && p.x < b.maxX - 0.05 && p.z > b.minZ + 0.05 && p.z < b.maxZ - 0.05 && p.y + 0.9 > b.minY && p.y + 0.9 < b.maxY);
      jammed[p.id] = inside ? jammed[p.id] + 1 : 0;
      assert.ok(jammed[p.id] < 8, `fighter ${p.id} walled in at ${p.x.toFixed(1)},${p.y.toFixed(1)},${p.z.toFixed(1)}`);
    }
    if (n % 3 !== 0) continue;
    const now = n * STEP * 1000;
    const lootDue = s.lootRev !== lastLootRev || Math.floor(now / 2000) !== Math.floor(sentAt / 2000);
    const buildDue = s.buildRev !== lastBuildRev || Math.floor((now + 1000) / 2000) !== Math.floor((sentAt + 1000) / 2000);
    const builds: boolean = buildDue && (!lootDue || preferBuilds);
    const loot: boolean = lootDue && !builds;
    if (loot) { lastLootRev = s.lootRev; preferBuilds = true; }
    if (builds) { lastBuildRev = s.buildRev; preferBuilds = false; }
    sentAt = now;
    const json: string = JSON.stringify(shooterNet.encodeSnapshot(s, s.events, loot, builds));
    maxBytes = Math.max(maxBytes, json.length);
    assert.ok(json.length < RELAY_CAP, `snapshot at ${s.clock.toFixed(1)} s is ${json.length} bytes`);
    const back = shooterNet.decodeSnapshot(JSON.parse(json), () => 'Guest', guest?.loot ?? null, guest?.builds ?? null);
    assert.ok(back, `snapshot at ${s.clock.toFixed(1)} s decodes`);
    assert.equal(back.map, 'deadcity');
    assert.equal(back.players.filter(p => p.mode !== 'dead').length, aliveCount(s));
    guest = back;
  }
  assert.equal(s.phase, 'done', `match ended (clock ${s.clock.toFixed(0)} s)`);
  assert.ok(s.winner >= 0 && s.players[s.winner].mode !== 'dead' && aliveCount(s) === 1, 'one winner');
  assert.ok(guest && guest.winner === s.winner, 'the guest saw the same winner');
  assert.ok(seen.pickup > 30 && seen.chest > 0 && seen.kill >= 10 && seen.shot > 100, `pickups ${seen.pickup}, chests ${seen.chest}, kills ${seen.kill}, shots ${seen.shot}`);
  assert.ok(seen.indoorsUp > 0, 'someone fought from an upper floor');
  assert.ok(maxBytes < RELAY_CAP, `largest snapshot ${maxBytes} bytes`);
});
