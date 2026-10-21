/**
 * CPU drivers: follow a racing line ahead along their chosen route (a lap's
 * worth of branch picks in `cpuRoute`), take the inside of corners, drift
 * through long bends for mini-turbos (normal/hard), keep to the middle of
 * narrow rail-less decks, dodge bananas, rollers and crushers, and use items
 * situationally. Easy CPUs wobble, never drift and do not read hazards.
 */
import { clamp, type Rng, wrapAngle } from '../common/math.ts';
import { advance, F_RAILS, obstacleAt, pointAt, sampleAt, type Spot, type Track } from './tracks.ts';
import type { KartInput, KartState, Racer } from './sim.ts';

const _pt = [0, 0, 0, 0];
const _ob = [0, 0, 0, 0];
const _spot: Spot = { seg: 0, s: 0 };

/** Curvature `d` metres ahead along the racer's route. */
function curveAhead(track: Track, r: Racer, d: number): number {
  advance(track, r.seg, r.segS, d, r.cpuRoute, _spot);
  const p = track.paths[_spot.seg];
  return p.curve[sampleAt(p, _spot.s)];
}

/** Whether the road `d` metres ahead on the racer's route is a rail-less deck over a drop. */
function exposedAt(track: Track, r: Racer, d: number): boolean {
  advance(track, r.seg, r.segS, d, r.cpuRoute, _spot);
  const p = track.paths[_spot.seg], i = sampleAt(p, _spot.s);
  return p.drop[i] > 0.5 && !(p.flags[i] & F_RAILS);
}

export function cpuInput(state: KartState, track: Track, r: Racer, rng: Rng, cruise: boolean): KartInput {
  const input: KartInput = { throttle: 1, steer: 0, drift: false, item: false, respawn: false };
  const speed = Math.max(0, r.speed);
  // Narrow decks without rails (here or just ahead): hold the middle, look closer, don't drift.
  const exposed = exposedAt(track, r, 0) || exposedAt(track, r, 18);
  const look = exposed ? 5 + speed * 0.28 : 8 + speed * 0.5;
  const curveNear = curveAhead(track, r, look * 0.8);
  const curveFar = curveAhead(track, r, look * 1.8);
  advance(track, r.seg, r.segS, look, r.cpuRoute, _spot);
  const target = track.paths[_spot.seg];
  const hw = target.halfWidth;
  // Inside line: positive curvature turns left, and lateral > 0 is left.
  const inside = clamp((curveNear + curveFar) * 14, -0.55, 0.55);
  let lane = clamp(r.cpuLane * 0.5 + inside, -0.7, 0.7) * hw * (exposed ? 0.25 : 1);
  // Dodge bananas sitting on the line ahead.
  for (const hz of state.hazards) {
    const dx = hz.x - r.x, dz = hz.z - r.z, d = Math.hypot(dx, dz);
    if (d > 26 || d < 1) continue;
    const ahead = dx * Math.sin(r.yaw) + dz * Math.cos(r.yaw);
    if (ahead < 0) continue;
    const side = dx * Math.cos(r.yaw) - dz * Math.sin(r.yaw); // > 0: hazard on the left
    if (Math.abs(side) < 2.6) lane += (side > 0 ? -1 : 1) * 4.5;
  }
  if (state.difficulty !== 'easy') {
    // Rollers and crushers: steer for where they will be on arrival.
    for (const o of track.obstacles) {
      if (o.seg !== _spot.seg && o.seg !== r.seg) continue;
      const d = o.s - r.segS;
      if (o.seg !== r.seg || d < 0 || d > 34) continue;
      const eta = d / Math.max(8, speed);
      obstacleAt(track, o, state.raceTime + eta, _ob);
      const path = track.paths[o.seg];
      if (o.kind === 'roller') {
        if (Math.abs(lane - _ob[3]) < 3.4) lane = _ob[3] > 0 ? _ob[3] - 4.2 : _ob[3] + 4.2;
      } else if (_ob[3] < 3) {
        const lat = o.lane * path.halfWidth * 0.5;
        if (Math.abs(lane - lat) < 3.6) lane = lat > 0 ? lat - 4.4 : lat + 4.4;
      }
    }
  }
  lane = clamp(lane, -hw * 0.8, hw * 0.8);
  pointAt(target, _spot.s, lane, _pt);
  const angle = wrapAngle(Math.atan2(_pt[0] - r.x, _pt[2] - r.z) - r.yaw);
  input.steer = clamp(-angle * 2.4, -1, 1);
  if (state.difficulty === 'easy') {
    r.cpuMistake -= 1 / 60;
    if (r.cpuMistake < -3 && rng.chance(0.004)) r.cpuMistake = 0.6;
    if (r.cpuMistake > 0 && !exposed) input.steer = clamp(input.steer + Math.sin(state.raceTime * 7 + r.id) * 0.6, -1, 1);
  }
  if (Math.abs(angle) > 1.2) input.throttle = 0.5;
  else if (exposed && Math.abs(curveNear) > 0.012 && speed > 22) input.throttle = 0.3;
  if (r.wrong > 1.5 || Math.abs(angle) > 2.2) input.steer = angle > 0 ? -1 : 1;
  if (cruise) { input.throttle = 0.6; return input; }

  // Drift through sustained bends at speed.
  const bend = Math.abs(curveNear) > 0.022 && Math.sign(curveNear) === Math.sign(curveFar);
  const wantLevel = state.difficulty === 'hard' ? 2 : 1;
  if (state.difficulty !== 'easy' && speed > 17 && !exposed && !r.glide) {
    if (r.drift === 0 && bend && r.grounded) {
      input.drift = true;
      input.steer = curveNear > 0 ? -0.8 : 0.8;
    } else if (r.drift !== 0) {
      // Drift -1 turns left; its outside is the right-hand side (lateral < 0).
      const outside = r.lateral * r.drift;
      const wantsTurn = angle * -r.drift;
      input.drift = (r.driftLevel < wantLevel || Math.abs(curveNear) > 0.012) && outside < track.paths[r.seg].halfWidth * 0.55 && wantsTurn > -0.2;
      input.steer = clamp(r.drift * (wantsTurn * 3 - 0.2), -1, 1);
    }
  }
  // Mid-air: trick for the landing boost.
  if (!r.grounded && r.trickReady && r.air > 0.15 && state.difficulty !== 'easy' && rng.chance(0.2)) input.drift = !r.prevDrift;

  // Items.
  if (r.item && r.roll <= 0) {
    r.cpuItemDelay -= 1 / 60;
    if (r.cpuItemDelay <= 0) {
      const behind = state.racers.some(o => o.id !== r.id && o.travel < r.travel && r.travel - o.travel < 18);
      const ahead = state.racers.find(o => o.place === r.place - 1);
      const gap = ahead ? ahead.travel - r.travel : Infinity;
      let use = false;
      switch (r.item) {
        case 'mushroom': case 'triple': use = (Math.abs(curveNear) < 0.012 || r.surface === 'grass') && !exposed; break;
        case 'banana': use = behind || rng.chance(0.004); break;
        case 'bomb': use = gap < 70 || rng.chance(0.003); break;
        case 'star': use = gap < 30 || rng.chance(0.004); break;
        case 'bolt': use = r.place > 2; break;
      }
      if (use) { input.item = !r.prevItem; r.cpuItemDelay = rng.range(0.4, 2.2); }
    }
  }
  return input;
}
