/**
 * CPU drivers: follow a racing line ahead on the centre spline, take the inside
 * of corners, drift through long bends for mini-turbos (normal/hard), dodge
 * bananas and use items situationally. Easy CPUs wobble and never drift.
 */
import { clamp, type Rng, wrapAngle } from '../common/math.ts';
import { pointAt, sampleAt, type Track } from './tracks.ts';
import type { KartInput, KartState, Racer } from './sim.ts';

const _pt = [0, 0, 0, 0];

export function cpuInput(state: KartState, track: Track, r: Racer, rng: Rng, cruise: boolean): KartInput {
  const input: KartInput = { throttle: 1, steer: 0, drift: false, item: false, respawn: false };
  const speed = Math.max(0, r.speed);
  const look = 8 + speed * 0.5;
  const curveAhead = track.curve[sampleAt(track, r.lastS + look * 0.8)];
  const curveFar = track.curve[sampleAt(track, r.lastS + look * 1.8)];
  // Inside line: positive curvature turns left, and lateral > 0 is left.
  const inside = clamp((curveAhead + curveFar) * 14, -0.55, 0.55);
  let lane = clamp(r.cpuLane * 0.5 + inside, -0.7, 0.7) * track.halfWidth;
  // Dodge bananas sitting on the line ahead.
  for (const hz of state.hazards) {
    const dx = hz.x - r.x, dz = hz.z - r.z, d = Math.hypot(dx, dz);
    if (d > 26 || d < 1) continue;
    const ahead = dx * Math.sin(r.yaw) + dz * Math.cos(r.yaw);
    if (ahead < 0) continue;
    const side = dx * Math.cos(r.yaw) - dz * Math.sin(r.yaw); // > 0: hazard on the left
    if (Math.abs(side) < 2.6) lane += (side > 0 ? -1 : 1) * 4.5;
  }
  lane = clamp(lane, -track.halfWidth * 0.8, track.halfWidth * 0.8);
  pointAt(track, r.lastS + look, lane, _pt);
  const angle = wrapAngle(Math.atan2(_pt[0] - r.x, _pt[2] - r.z) - r.yaw);
  input.steer = clamp(-angle * 2.4, -1, 1);
  if (state.difficulty === 'easy') {
    r.cpuMistake -= 1 / 60;
    if (r.cpuMistake < -3 && rng.chance(0.004)) r.cpuMistake = 0.6;
    if (r.cpuMistake > 0) input.steer = clamp(input.steer + Math.sin(state.raceTime * 7 + r.id) * 0.6, -1, 1);
  }
  if (Math.abs(angle) > 1.2) input.throttle = 0.5;
  if (r.wrong > 1.5 || Math.abs(angle) > 2.2) input.steer = angle > 0 ? -1 : 1;
  if (cruise) { input.throttle = 0.6; return input; }

  // Drift through sustained bends at speed.
  const bend = Math.abs(curveAhead) > 0.022 && Math.sign(curveAhead) === Math.sign(curveFar);
  const wantLevel = state.difficulty === 'hard' ? 2 : 1;
  if (state.difficulty !== 'easy' && speed > 17) {
    if (r.drift === 0 && bend && r.grounded) {
      input.drift = true;
      input.steer = curveAhead > 0 ? -0.8 : 0.8;
    } else if (r.drift !== 0) {
      // Drift -1 turns left; its outside is the right-hand side (lateral < 0).
      const outside = r.lateral * r.drift;
      const wantsTurn = angle * -r.drift;
      input.drift = (r.driftLevel < wantLevel || Math.abs(curveAhead) > 0.012) && outside < track.halfWidth * 0.55 && wantsTurn > -0.2;
      input.steer = clamp(r.drift * (wantsTurn * 3 - 0.2), -1, 1);
    }
  }

  // Items.
  if (r.item && r.roll <= 0) {
    r.cpuItemDelay -= 1 / 60;
    if (r.cpuItemDelay <= 0) {
      const behind = state.racers.some(o => o.id !== r.id && o.travel < r.travel && r.travel - o.travel < 18);
      const ahead = state.racers.find(o => o.place === r.place - 1);
      const gap = ahead ? ahead.travel - r.travel : Infinity;
      let use = false;
      switch (r.item) {
        case 'mushroom': case 'triple': use = Math.abs(curveAhead) < 0.012 || r.surface === 'grass'; break;
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
