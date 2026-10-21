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
import { clamp, wrapAngle } from '../src/modules/rooms/games/common/math.ts';
import { advance, circ, FORK_ZONE, nearest, newNear, pointAt, progressOf, sampleAt, TRACK_IDS, trackFor } from '../src/modules/rooms/games/kart/tracks.ts';
import { createRace, idleInput, LAPS, tick, type KartInput, type KartState, type Racer } from '../src/modules/rooms/games/kart/sim.ts';
import * as kartNet from '../src/modules/rooms/games/kart/net.ts';
import { aliveCount, createMatch, STEP as SHOOTER_STEP, teamsAlive, tickMatch, type ShooterState, type TeamSize } from '../src/modules/rooms/games/shooter/sim.ts';
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
  // A route counts as driven once someone is well inside it (past the fork zone, short of the merge).
  const segs = new Set<number>();
  // Stuck detector: lap distance gained per 10 s window while racing and unfinished.
  const windowStart = s.racers.map(r => r.travel);
  let windowAt = 0, worstWindow = Infinity, decoded = 0, maxBytes = 0, rescues = 0, last = 0;
  let n = 0;
  while (s.phase !== 'done' && n++ < 60 * 600) {
    tick(s, []);
    for (const e of s.events) if (e.id > last && (e.type === 'respawn' || e.type === 'splash') && e.value !== 1) rescues++;
    last = s.eventSeq;
    for (const r of s.racers) if (r.seg === 0 || (r.segS > FORK_ZONE && r.segS < t.paths[r.seg].length - FORK_ZONE)) segs.add(r.seg);
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
        assert.equal(g.seg, h.seg); assert.equal(g.glide, h.glide); assert.equal(g.under, h.under); assert.equal(g.coins, h.coins);
        assert.ok(Math.abs(g.x - h.x) < 0.01 && Math.abs(g.z - h.z) < 0.01);
      });
      assert.deepEqual(back.coins.map(c => c.timer > 0), s.coins.map(c => c.timer > 0));
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
    // Coins changed hands.
    assert.ok(s.racers.some(r => r.coins > 0), `${track}: someone holds coins at the flag`);
  });
}

/** Autopilot for one human: steer for the road ahead along `route`; on a boost-only cut, fire the mushroom in the mud. */
function routePilot(s: KartState, r: Racer, route: number): KartInput {
  const t = trackFor(s.track);
  const spot = advance(t, r.seg, r.segS, 7 + Math.max(0, r.speed) * 0.3, route, { seg: 0, s: 0 });
  const p = pointAt(t.paths[spot.seg], spot.s, 0);
  const angle = wrapAngle(Math.atan2(p[0] - r.x, p[2] - r.z) - r.yaw);
  const mud = t.paths[r.seg].boost && r.surface === 'grass' && r.boost <= 0 && r.item !== null;
  return { ...idleInput(), throttle: 1, steer: clamp(-angle * 2.4, -1, 1), item: mud && !r.prevItem };
}

for (const track of TRACK_IDS) {
  test(`kart play-test: every route of ${track} is drivable start to finish, at any point of the drawbridge cycle, and roughly as quick`, () => {
    const t = trackFor(track);
    for (const branch of t.paths.slice(1)) {
      const times: number[] = [];
      for (const route of [0, 1 << (branch.index - 1)]) for (const clock of [0, 2.5, 5, 7.5]) {
        const s = createRace(track, [{ racer: 'oobi', human: true, slot: 0, name: 'P' }], 'normal', 1);
        while (s.phase !== 'race') tick(s, []);
        s.raceTime = clock;
        const r = s.racers[0];
        // From 90 m before the fork at racing speed (with a mushroom for a boost-only cut).
        const pt = pointAt(t, branch.from - 90, 0), near = nearest(t, pt[0], pt[2], sampleAt(t, branch.from - 90), newNear());
        r.x = pt[0]; r.z = pt[2]; r.y = t.ground(pt[0], pt[2], near, clock); r.yaw = pt[3];
        r.speed = 24; r.vx = Math.sin(r.yaw) * 24; r.vz = Math.cos(r.yaw) * 24;
        r.seg = 0; r.segS = near.s; r.safeSeg = 0; r.safeS = near.s; r.hint = -1; r.lastS = progressOf(t, 0, near.s); r.travel = r.lastS;
        if (route && branch.boost) { r.item = 'mushroom'; r.itemCount = 1; }
        const target = branch.to + 50, from = s.eventSeq, t0 = s.raceTime;
        let deep = false, n = 0;
        while (n++ < 60 * 60) {
          tick(s, [routePilot(s, r, route)]);
          if (r.seg === branch.index && r.segS > FORK_ZONE && r.segS < branch.length - FORK_ZONE) deep = true;
          const past = circ(r.lastS - target, t.length);
          if (r.seg === 0 && past > 0 && past < 60) break;
        }
        const label = `${track}/${branch.name} ${route ? 'branch' : 'main'} @${clock}s`;
        assert.ok(n < 60 * 60, `${label}: reached the merge (stuck at ${r.x.toFixed(0)}, ${r.z.toFixed(0)})`);
        assert.equal(deep, route !== 0, `${label}: drove the intended route`);
        const falls = s.events.filter(e => e.id > from && (e.type === 'respawn' || (e.type === 'splash' && e.value !== 1)));
        assert.equal(falls.length, 0, `${label}: no falls`);
        times.push(s.raceTime - t0);
      }
      // Neither side of a fork is a trap: the slower one (crash-free) is within 60 % of the quicker.
      const main = Math.min(...times.slice(0, 4)), alt = Math.min(...times.slice(4));
      assert.ok(Math.max(main, alt) < Math.min(main, alt) * 1.6, `${track}/${branch.name}: main ${main.toFixed(1)} s vs branch ${alt.toFixed(1)} s`);
    }
  });
}

// ── Storm Royale: a full 16-bot match on both islands ─────────────────────────

/**
 * Host loop with the ShooterGame send cadence (20 Hz; loot and structures ride
 * when they changed or every two seconds, never together) and a guest that keeps
 * the previous loot / structures when a snapshot leaves them out.
 */
function shooterMatch(map: ShooterMapId, seed: number, teamSize: TeamSize = 1) {
  const s: ShooterState = createMatch({ map, difficulty: 'hard', seed, seats: shooterSeats([], 16), teamSize });
  let guest: ShooterState | null = null;
  let lastLootRev = -1, lastBuildRev = -1, preferBuilds = false, sentAt = 0, n = 0, maxBytes = 0, maxBuilds = 0, withBuilds = 0, last = 0;
  // Team play bookkeeping: knocks, revives, supply drops opened and their loot picked up.
  const seen = { knock: 0, revive: 0, supplyOpened: 0, supplyLooted: 0, felled: 0, launch: 0 };
  const opened: { x: number; z: number }[] = [];
  while (s.phase !== 'done' && n++ < 60 * 600) {
    tickMatch(s, []);
    for (const e of s.events) {
      if (e.id <= last) continue;
      if (e.type === 'knock') seen.knock++;
      if (e.type === 'revive') seen.revive++;
      if (e.type === 'fell') seen.felled++;
      if (e.type === 'launch') seen.launch++;
      if (e.type === 'supply' && e.flag === 2) { seen.supplyOpened++; opened.push({ x: e.x, z: e.z }); }
      if (e.type === 'pickup' && opened.some(o => Math.hypot(o.x - e.x, o.z - e.z) < 3)) seen.supplyLooted++;
    }
    last = s.eventSeq;
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
    assert.equal(back.players.filter(p => p.mode === 'down').length, s.players.filter(p => p.mode === 'down').length);
    assert.deepEqual(back.felled, s.felled);
    guest = back;
  }
  return { s, guest, maxBytes, maxBuilds, withBuilds, ticks: n, seen };
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

for (const [map, size, seed] of [['isle', 2, 23], ['mesa', 4, 7]] as [ShooterMapId, TeamSize, number][]) {
  test(`shooter play-test: ${size === 2 ? 'duos' : 'squads'} on ${map}: knocks and revives happen, a supply drop is looted, one team wins together`, () => {
    const { s, guest, maxBytes, ticks, seen } = shooterMatch(map, seed, size);
    assert.equal(s.phase, 'done', `${map}: match ended (clock ${s.clock.toFixed(0)} s)`);
    assert.ok(ticks < 60 * 600, `${map}: ended within 10 minutes of play`);
    assert.equal(teamsAlive(s), 1, 'one team left');
    assert.ok(s.winTeam >= 0 && s.players[s.winner].team === s.winTeam && s.players[s.winner].mode !== 'dead');
    for (const p of s.players) assert.equal(p.place === 1, p.team === s.winTeam, `fighter ${p.id} (team ${p.team}) placed ${p.place}`);
    // Teams share their place; every place from 1 to the team count is used once per team.
    const places = new Map<number, number>();
    for (const p of s.players) { assert.ok(!places.has(p.team) || places.get(p.team) === p.place, `team ${p.team} shares one place`); places.set(p.team, p.place); }
    assert.deepEqual([...places.values()].sort((a, b) => a - b), Array.from({ length: 16 / size }, (_, i) => i + 1));
    assert.ok(seen.knock > 0 && seen.revive > 0, `${seen.knock} knocks, ${seen.revive} revives`);
    assert.ok(seen.supplyOpened > 0 && seen.supplyLooted > 0, `${seen.supplyOpened} supply drops opened, ${seen.supplyLooted} pickups from them`);
    assert.ok(seen.felled > 0, `${seen.felled} trees felled`);
    assert.ok(guest && guest.winTeam === s.winTeam && guest.squad === size, `${map}: the guest saw the same winning team`);
    assert.ok(maxBytes < RELAY_CAP, `${map}: largest snapshot ${maxBytes} bytes`);
  });
}
