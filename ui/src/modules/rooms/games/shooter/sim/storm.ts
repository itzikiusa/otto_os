// The storm (or toxic cloud): picking each phase's next safe circle and
// moving / shrinking it, with damage to anyone caught outside.
import { clamp, Rng } from '../../common/math.ts';
import { HALF, mapFor, dryLand, type ShooterMap } from '../map.ts';
import type { World } from '../collide.ts';
import { isRoyale } from '../modes/index.ts';
import { STEP, STORM, START_RADIUS, KILL_STORM, FROM_STORM, SUPPLY_STAGES, type ShooterState } from './model.ts';
import { emit } from './core.ts';
import { hurt } from './combat.ts';
import { spawnDrop } from './loot.ts';

export function pickStormTarget(state: ShooterState, rng: Rng): void {
  const s = state.storm;
  const next = STORM[Math.min(STORM.length - 1, s.stage + 1)];
  const map = mapFor(state.map);
  // New circle inside the current one, centre kept on dry land.
  for (let tries = 0; tries < 30; tries++) {
    const room = Math.max(0, Math.min(s.r, START_RADIUS * 0.7) - next.radius);
    const a = rng.range(0, Math.PI * 2), d = rng.range(0, room * 0.85);
    const x = clamp(s.cx + Math.cos(a) * d, -HALF * 0.7, HALF * 0.7), z = clamp(s.cz + Math.sin(a) * d, -HALF * 0.7, HALF * 0.7);
    if (dryLand(map, x, z) || tries === 29) { s.toX = x; s.toZ = z; break; }
  }
  s.toR = next.radius;
}
/**
 * The storm (Apocalypse Royale: a toxic cloud that hits half as hard again).
 * In the arena modes and Zombies it is a fixed boundary instead: the schedule
 * never runs, only the damage outside it.
 */
export function stepStorm(state: ShooterState, map: ShooterMap, world: World, rng: Rng): void {
  if (state.phase !== 'play') return;
  const s = state.storm, scale = state.stormScale || 1;
  if (isRoyale(state.mode)) s.timer -= STEP;
  if (!isRoyale(state.mode)) { /* fixed boundary */ } else if (!s.shrinking && s.timer <= 0) {
    const next = STORM[Math.min(STORM.length - 1, s.stage + 1)];
    s.stage = Math.min(STORM.length - 1, s.stage + 1);
    s.shrinking = true; s.timer = next.shrink * scale;
    s.fromX = s.cx; s.fromZ = s.cz; s.fromR = s.r;
    s.damage = Math.round(next.damage * (state.mode === 'apoc' ? 1.5 : 1));
    emit(state, 'storm', -1, s.toX, 0, s.toZ, { value: s.stage, flag: 1 });
    if (SUPPLY_STAGES.includes(s.stage)) spawnDrop(state, map, world, rng);
  } else if (s.shrinking) {
    const total = STORM[s.stage].shrink * scale, k = clamp(1 - s.timer / total, 0, 1);
    s.cx = s.fromX + (s.toX - s.fromX) * k; s.cz = s.fromZ + (s.toZ - s.fromZ) * k; s.r = s.fromR + (s.toR - s.fromR) * k;
    if (s.timer <= 0) {
      s.shrinking = false; s.cx = s.toX; s.cz = s.toZ; s.r = s.toR;
      if (s.stage < STORM.length - 1) { s.timer = STORM[s.stage + 1].wait * scale; pickStormTarget(state, rng); emit(state, 'storm', -1, s.toX, 0, s.toZ, { value: s.stage + 1, flag: 0 }); }
      else s.timer = 9999;
    }
  }
  for (const p of state.players) {
    if (p.mode !== 'ground' && p.mode !== 'glide' && p.mode !== 'down') continue;
    if (Math.hypot(p.x - s.cx, p.z - s.cz) <= s.r) { p.stormT = 0; continue; }
    p.stormT += STEP;
    if (p.stormT >= 1) { p.stormT -= 1; hurt(state, p, Math.max(1, s.damage), FROM_STORM, false, false, KILL_STORM); }
  }
}
