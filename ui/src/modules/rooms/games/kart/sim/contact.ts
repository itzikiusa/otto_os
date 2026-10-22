// Kart contacts each tick: slipstream drafting, kart-to-kart bumps,
// rolling / crushing obstacles, item boxes and coins.
import { wrapAngle } from '../../common/math.ts';
import { CRUSHER_HALF, obstacleAt, ROLLER_RADIUS, type Track } from '../tracks.ts';
import { STEP, DRAFT_SECONDS, DRAFT_BOOST, DRAFT_POWER, MAX_COINS, COIN_RESPAWN, type KartState } from './model.ts';
import { _ob, emit, stats, giveBoost, spinOut } from './core.ts';

/**
 * Slipstream: a racer tucked in behind another at speed (3–20 m back, nearly
 * in line, heading the same way) charges a draft; held long enough, it fires a
 * boost past them.
 */
export function updateDrafts(state: KartState): void {
  for (const r of state.racers) {
    if (r.respawn > 0 || !r.grounded || r.speed < 18 || r.boost > 0 || r.finish !== null) { r.draft = Math.max(0, r.draft - STEP * 2); continue; }
    const fx = Math.sin(r.yaw), fz = Math.cos(r.yaw);
    let tucked = false;
    for (const o of state.racers) {
      if (o === r || o.respawn > 0 || o.speed < 15) continue;
      const dx = o.x - r.x, dz = o.z - r.z;
      const ahead = dx * fx + dz * fz, side = Math.abs(dx * fz - dz * fx);
      if (ahead > 3 && ahead < 20 && side < 2.4 && Math.abs(o.y - r.y) < 2 && Math.abs(wrapAngle(o.yaw - r.yaw)) < 0.35) { tucked = true; break; }
    }
    if (!tucked) { r.draft = Math.max(0, r.draft - STEP * 2); continue; }
    r.draft += STEP;
    if (r.draft >= DRAFT_SECONDS) { r.draft = 0; giveBoost(r, DRAFT_BOOST, DRAFT_POWER, 0.3); emit(state, 'draft', r); }
  }
}

export function collideRacers(state: KartState): void {
  const rs = state.racers;
  for (let i = 0; i < rs.length; i++) for (let j = i + 1; j < rs.length; j++) {
    const a = rs[i], b = rs[j];
    if (a.respawn > 0 || b.respawn > 0) continue;
    const dx = b.x - a.x, dz = b.z - a.z, d2 = dx * dx + dz * dz;
    if (d2 > 2.3 * 2.3 || d2 < 1e-6 || Math.abs(a.y - b.y) > 1.6) continue;
    const d = Math.sqrt(d2), nx = dx / d, nz = dz / d, overlap = 2.3 - d;
    const wa = stats(a).weight + (a.star > 0 ? 3 : 0), wb = stats(b).weight + (b.star > 0 ? 3 : 0);
    const ta = wb / (wa + wb), tb = wa / (wa + wb);
    a.x -= nx * overlap * ta; a.z -= nz * overlap * ta;
    b.x += nx * overlap * tb; b.z += nz * overlap * tb;
    const rel = (b.vx - a.vx) * nx + (b.vz - a.vz) * nz;
    if (rel < 0) {
      const impulse = -rel * 1.3;
      a.vx -= nx * impulse * ta; a.vz -= nz * impulse * ta;
      b.vx += nx * impulse * tb; b.vz += nz * impulse * tb;
      if (-rel > 3) emit(state, 'bump', a);
    }
    if (a.star > 0 && b.star <= 0) spinOut(state, b);
    if (b.star > 0 && a.star <= 0) spinOut(state, a);
  }
}

/** Rolling boulders and crushers (poses are a pure function of race time). */
export function collideObstacles(state: KartState, track: Track): void {
  for (const o of track.obstacles) {
    obstacleAt(track, o, state.raceTime, _ob);
    for (const r of state.racers) {
      if (r.respawn > 0 || r.star > 0 || r.ghost > 0 || r.spin > 0.3) continue;
      const dx = r.x - _ob[0], dz = r.z - _ob[2];
      if (o.kind === 'roller') {
        if (dx * dx + dz * dz > (ROLLER_RADIUS + 1.1) ** 2 || Math.abs(r.y + 0.6 - _ob[1]) > ROLLER_RADIUS + 1) continue;
        const d = Math.hypot(dx, dz) || 1;
        r.x = _ob[0] + dx / d * (ROLLER_RADIUS + 1.15); r.z = _ob[2] + dz / d * (ROLLER_RADIUS + 1.15);
        r.speed *= 0.5;
        if (spinOut(state, r, 0.9)) emit(state, 'bump', r);
      } else {
        // A crusher only hurts while it is (nearly) down.
        if (_ob[3] > 1.6 || dx * dx + dz * dz > (CRUSHER_HALF + 0.9) ** 2 || Math.abs(r.y - _ob[1]) > 3) continue;
        r.speed = Math.min(r.speed, 4);
        if (spinOut(state, r, 1.4)) emit(state, 'crush', r);
      }
    }
  }
}

export function updateBoxes(state: KartState): void {
  for (const box of state.boxes) {
    if (box.timer > 0) { box.timer = Math.max(0, box.timer - STEP); continue; }
    for (const r of state.racers) {
      if (r.respawn > 0) continue;
      const dx = r.x - box.x, dz = r.z - box.z;
      if (dx * dx + dz * dz < 2.4 * 2.4 && Math.abs(r.y + 0.6 - box.y) < 2.2) {
        box.timer = 2;
        emit(state, 'box', r);
        if (!r.item && r.roll <= 0 && r.finish === null) r.roll = 1.1;
        break;
      }
    }
  }
}

/** Coins: driving through one adds it to the racer's purse (capped); it reappears later. No event per coin (the HUD counts). */
export function updateCoins(state: KartState): void {
  for (const c of state.coins) {
    if (c.timer > 0) { c.timer = Math.max(0, c.timer - STEP); continue; }
    for (const r of state.racers) {
      if (r.respawn > 0 || r.finish !== null) continue;
      const dx = r.x - c.x, dz = r.z - c.z;
      if (dx * dx + dz * dz < 1.9 * 1.9 && Math.abs(r.y + 0.6 - c.y) < 2.2) {
        c.timer = COIN_RESPAWN;
        r.coins = Math.min(MAX_COINS, r.coins + 1);
        break;
      }
    }
  }
}
