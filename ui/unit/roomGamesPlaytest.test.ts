/**
 * Headless play-tests for the Rooms games: whole races and whole matches run
 * on the host simulation while a "guest" decodes every snapshot the host would
 * send (JSON round-trip, the same cadence as the Svelte components). They catch
 * what single-feature tests miss: racers that get stuck somewhere on a lap,
 * branches nobody ever takes, a snapshot that stops decoding mid-match, or one
 * that outgrows the relay's 16 KiB cap once structures pile up.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { TRACK_IDS, trackFor } from '../src/modules/rooms/games/kart/tracks.ts';
import { createRace, LAPS, tick, type KartState } from '../src/modules/rooms/games/kart/sim.ts';
import * as kartNet from '../src/modules/rooms/games/kart/net.ts';
import { aliveCount, createMatch, STEP as SHOOTER_STEP, tickMatch, type ShooterState } from '../src/modules/rooms/games/shooter/sim.ts';
import * as shooterNet from '../src/modules/rooms/games/shooter/net.ts';
import type { ShooterMapId } from '../src/modules/rooms/games/shooter/catalog.ts';
import { kartSeats, shooterSeats } from '../src/modules/rooms/games/setup.ts';

const wire = (v: unknown): string => JSON.stringify(v);
/** The relay's per-snapshot cap. */
const RELAY_CAP = 16 * 1024;

// ── Kart: a full Grand Prix race on every track ───────────────────────────────

/** Whole race with 8 CPUs; a guest decodes a snapshot every third tick (20 Hz). */
function kartRace(track: (typeof TRACK_IDS)[number], seed: number) {
  const s: KartState = createRace(track, kartSeats([], true), 'hard', seed);
  const t = trackFor(track);
  const segs = new Set<number>();
  // Stuck detector: lap distance gained per 10 s window while racing and unfinished.
  const windowStart = s.racers.map(r => r.travel);
  let windowAt = 0, worstWindow = Infinity, decoded = 0, maxBytes = 0, rescues = 0, last = 0;
  let n = 0;
  while (s.phase !== 'done' && n++ < 60 * 600) {
    tick(s, []);
    for (const e of s.events) if (e.id > last && (e.type === 'respawn' || e.type === 'splash') && e.value !== 1) rescues++;
    last = s.eventSeq;
    for (const r of s.racers) segs.add(r.seg);
    if (s.phase === 'race' && s.raceTime - windowAt >= 10) {
      s.racers.forEach((r, i) => {
        if (r.finish === null && r.lap < LAPS) worstWindow = Math.min(worstWindow, r.travel - windowStart[i]);
        windowStart[i] = r.travel;
      });
      windowAt = s.raceTime;
    }
    if (n % 3 === 0) {
      const json = wire(kartNet.encodeSnapshot(s, s.events));
      maxBytes = Math.max(maxBytes, json.length);
      const back = kartNet.decodeSnapshot(JSON.parse(json), () => 'Guest');
      assert.ok(back, `${track}: snapshot at ${s.raceTime.toFixed(2)} s decodes`);
      back.racers.forEach((g, i) => {
        const h = s.racers[i];
        assert.equal(g.seg, h.seg); assert.equal(g.glide, h.glide); assert.equal(g.under, h.under);
        assert.ok(Math.abs(g.x - h.x) < 0.01 && Math.abs(g.z - h.z) < 0.01);
      });
      decoded++;
    }
  }
  return { s, t, segs, worstWindow, decoded, maxBytes, rescues };
}

for (const track of TRACK_IDS) {
  test(`kart play-test: 8 CPUs race 3 laps of ${track}, nobody stuck, every route taken, guests decode every snapshot`, () => {
    const { s, t, segs, worstWindow, decoded, maxBytes, rescues } = kartRace(track, 7);
    assert.equal(s.phase, 'done', `${track}: race ended (race time ${s.raceTime.toFixed(0)} s)`);
    for (const r of s.racers) {
      assert.ok(r.finish !== null, `${track}: racer ${r.id} finished`);
      assert.equal(r.lap, LAPS, `${track}: racer ${r.id} drove every lap`);
      assert.ok(r.travel >= LAPS * t.length, `${track}: racer ${r.id} covered the distance`);
    }
    // Nobody sat still: every unfinished racer gained at least 60 m in every 10 s of racing.
    assert.ok(Number.isFinite(worstWindow) && worstWindow > 60, `${track}: slowest 10 s window ${worstWindow.toFixed(1)} m`);
    // Every route (main loop + each branch) was driven by someone.
    for (const p of t.paths) assert.ok(segs.has(p.index), `${track}: someone drove ${p.name ?? `path ${p.index}`}`);
    // Falls happen, but rarely: under one rescue per racer per lap on average.
    assert.ok(rescues < s.racers.length * LAPS, `${track}: ${rescues} rescues`);
    assert.ok(decoded > 1000 && maxBytes < RELAY_CAP, `${track}: ${decoded} snapshots, largest ${maxBytes} bytes`);
  });
}

// ── Storm Royale: a full 16-bot match on both islands ─────────────────────────

/**
 * Host loop with the ShooterGame send cadence (20 Hz; loot and structures ride
 * when they changed or every two seconds, never together) and a guest that keeps
 * the previous loot / structures when a snapshot leaves them out.
 */
function shooterMatch(map: ShooterMapId, seed: number) {
  const s: ShooterState = createMatch({ map, difficulty: 'hard', seed, seats: shooterSeats([], 16) });
  let guest: ShooterState | null = null;
  let lastLootRev = -1, lastBuildRev = -1, preferBuilds = false, sentAt = 0, n = 0, maxBytes = 0, maxBuilds = 0, withBuilds = 0;
  while (s.phase !== 'done' && n++ < 60 * 600) {
    tickMatch(s, []);
    for (const p of s.players) assert.ok(Number.isFinite(p.x + p.y + p.z + p.hp + p.shield), `${map}: fighter ${p.id} state is finite`);
    maxBuilds = Math.max(maxBuilds, s.builds.length);
    if (n % 3 !== 0) continue;
    const now = n * SHOOTER_STEP * 1000;
    const lootDue = s.lootRev !== lastLootRev || Math.floor(now / 2000) !== Math.floor(sentAt / 2000);
    const buildDue = s.buildRev !== lastBuildRev || Math.floor((now + 1000) / 2000) !== Math.floor((sentAt + 1000) / 2000);
    const builds = buildDue && (!lootDue || preferBuilds);
    const loot = lootDue && !builds;
    if (loot) { lastLootRev = s.lootRev; preferBuilds = true; }
    if (builds) { lastBuildRev = s.buildRev; preferBuilds = false; withBuilds++; }
    sentAt = now;
    const json = wire(shooterNet.encodeSnapshot(s, s.events, loot, builds));
    maxBytes = Math.max(maxBytes, json.length);
    assert.ok(json.length < RELAY_CAP, `${map}: snapshot at ${s.clock.toFixed(1)} s is ${json.length} bytes`);
    const back = shooterNet.decodeSnapshot(JSON.parse(json), () => 'Guest', guest?.loot ?? null, guest?.builds ?? null);
    assert.ok(back, `${map}: snapshot at ${s.clock.toFixed(1)} s decodes`);
    if (loot) assert.equal(back.loot.length, s.loot.length);
    if (builds) assert.equal(back.builds.length, s.builds.length);
    assert.equal(back.players.filter(p => p.mode !== 'dead').length, aliveCount(s));
    guest = back;
  }
  return { s, guest, maxBytes, maxBuilds, withBuilds, ticks: n };
}

for (const map of ['isle', 'mesa'] as ShooterMapId[]) {
  test(`shooter play-test: a 16-bot match on ${map} ends with one winner, guests decode every snapshot under the relay cap`, () => {
    const { s, guest, maxBytes, maxBuilds, withBuilds, ticks } = shooterMatch(map, 23);
    assert.equal(s.phase, 'done', `${map}: match ended (clock ${s.clock.toFixed(0)} s)`);
    assert.ok(ticks < 60 * 600, `${map}: ended within 10 minutes of play`);
    assert.ok(s.winner >= 0 && s.players[s.winner].mode !== 'dead' && aliveCount(s) === 1);
    assert.ok(guest && guest.winner === s.winner, `${map}: the guest saw the same winner`);
    assert.ok(maxBuilds > 0 && withBuilds > 0, `${map}: bots built (${maxBuilds} pieces at most) and structures were sent`);
    assert.ok(maxBytes < RELAY_CAP, `${map}: largest snapshot ${maxBytes} bytes`);
  });
}
