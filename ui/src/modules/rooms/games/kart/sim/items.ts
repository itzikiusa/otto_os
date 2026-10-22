// Kart items in play: using a held item, launching shells, bananas on the
// road, and the shell / bomb flight and blast logic.
import { approach, clamp, wrapAngle } from '../../common/math.ts';
import { advance, F_RAILS, locate, pointAt, type Track } from '../tracks.ts';
import { STEP, MAX_HAZARDS, MAX_SHELLS, MUSHROOM_SECONDS, MUSHROOM_POWER, GOLDEN_SECONDS, SHELL_KINDS, SHELL, GREEN_GRACE, BANANA_GRACE, GRAVITY, type ShellKind, type Racer, type Shell, type KartState } from './model.ts';
import { _near, _pt, _spot, emit, giveBoost, spinOut } from './core.ts';

/** The best-placed unfinished racer other than `r` (a blue shell's target), or undefined. */
function leaderBesides(state: KartState, r: Racer): Racer | undefined {
  let best: Racer | undefined;
  for (const o of state.racers) if (o !== r && o.finish === null && (!best || o.place < best.place)) best = o;
  return best;
}

function launch(state: KartState, r: Racer, kind: ShellKind, back: boolean, target: number): void {
  const yaw = back ? wrapAngle(r.yaw + Math.PI) : r.yaw, fx = Math.sin(yaw), fz = Math.cos(yaw);
  const spec = SHELL[kind];
  // Thrown ahead it leaves at the shell's speed (or the kart's, if faster); a bomb is lobbed.
  const speed = kind === 'bomb' ? spec.speed + (back ? -12 : Math.max(0, r.speed) * 0.4) : Math.max(spec.speed, back ? 0 : r.speed + 8);
  state.shells.push({
    id: state.nextId++, kind, x: r.x + fx * 2.8, y: r.y + 0.6, z: r.z + fz * 2.8, yaw, target, owner: r.id,
    life: spec.life, hint: r.hint, seg: r.seg, vy: kind === 'bomb' ? (back ? 4 : 8) : 0, speed,
  });
  if (state.shells.length > MAX_SHELLS) state.shells.shift();
  emit(state, 'throw', r, SHELL_KINDS.indexOf(kind));
}

export function useItem(state: KartState, track: Track, r: Racer, back: boolean): void {
  const fx = Math.sin(r.yaw), fz = Math.cos(r.yaw);
  switch (r.item) {
    case 'mushroom':
    case 'triple':
      giveBoost(r, MUSHROOM_SECONDS, MUSHROOM_POWER, 0.45); emit(state, 'boost', r, 1);
      break;
    case 'golden':
      // The first press opens the window; every press inside it is another mushroom. Not used up per press.
      if (r.golden <= 0) r.golden = GOLDEN_SECONDS;
      giveBoost(r, MUSHROOM_SECONDS, MUSHROOM_POWER, 0.45); emit(state, 'boost', r, 3);
      return;
    case 'banana': {
      // Dropped just behind the kart.
      state.hazards.push({ id: state.nextId++, x: r.x - fx * 2.6, y: r.y, z: r.z - fz * 2.6, owner: r.id, age: 0 });
      if (state.hazards.length > MAX_HAZARDS) state.hazards.shift();
      emit(state, 'drop', r);
      break;
    }
    case 'green': launch(state, r, 'green', back, -1); break;
    case 'red': {
      const ahead = state.racers.find(o => o.place === r.place - 1 && o.finish === null);
      launch(state, r, 'red', false, ahead ? ahead.id : -1);
      break;
    }
    case 'blue': launch(state, r, 'blue', false, leaderBesides(state, r)?.id ?? -1); break;
    case 'bomb': launch(state, r, 'bomb', back, -1); break;
    case 'star':
      r.star = 7; r.spin = 0; emit(state, 'star', r);
      break;
    case 'bolt': {
      // Lightning strikes every opponent, never the user (who keeps every bit of speed): a short spin, then
      // shrunk and slowed — longest for the leaders — and whatever they were holding is lost. A star or a
      // post-rescue moment shrugs it off.
      const n = state.racers.length;
      let hits = 0;
      for (const o of state.racers) {
        if (o === r || o.finish !== null || o.respawn > 0 || o.star > 0 || o.ghost > 0) continue;
        spinOut(state, o, 0.7);
        o.slow = Math.max(o.slow, 2 + 2.5 * (1 - (o.place - 1) / Math.max(1, n - 1)));
        if (o.roll <= 0) { o.item = null; o.itemCount = 0; o.golden = 0; }
        emit(state, 'zap', o);
        hits++;
      }
      emit(state, 'bolt', r, hits);
      break;
    }
  }
  r.itemCount--;
  if (r.itemCount <= 0) { r.item = null; r.itemCount = 0; }
}

export function updateHazards(state: KartState): void {
  for (let h = state.hazards.length - 1; h >= 0; h--) {
    const hz = state.hazards[h];
    hz.age += STEP;
    for (const r of state.racers) {
      // Its owner drives clear of a banana it just dropped.
      if (r.id === hz.owner && hz.age < BANANA_GRACE) continue;
      const dx = r.x - hz.x, dz = r.z - hz.z;
      if (dx * dx + dz * dz < 1.7 * 1.7 && Math.abs(r.y - hz.y) < 1.5 && r.respawn <= 0) {
        spinOut(state, r, 1.0);
        state.hazards.splice(h, 1);
        break;
      }
    }
  }
}

/** An explosion at a shell: everyone but its owner inside `radius` spins out. */
function blast(state: KartState, sh: Shell, radius: number): void {
  emit(state, 'explode', { id: sh.owner, x: sh.x, y: sh.y, z: sh.z }, SHELL_KINDS.indexOf(sh.kind));
  for (const o of state.racers) if (o.id !== sh.owner && Math.hypot(o.x - sh.x, o.z - sh.z) < radius && Math.abs(o.y - sh.y) < 4) spinOut(state, o, 1.3);
}

/**
 * Shells and bombs. Green: straight on, bouncing off walls and rails. Red:
 * down the road, homing in on its target once close. Blue: high over the
 * pack along the leader's route, then down onto them in a blast. Bomb: lobbed,
 * rolls to a stop, goes off on contact or when its fuse runs out. None of them
 * touches its owner (a green shell only after GREEN_GRACE).
 */
export function updateShells(state: KartState, track: Track): void {
  for (let i = state.shells.length - 1; i >= 0; i--) {
    const sh = state.shells[i];
    sh.life -= STEP;
    const near = locate(track, sh.x, sh.y, sh.z, sh.seg, sh.hint, _near);
    sh.hint = near.index; sh.seg = near.seg;
    const path = track.paths[near.seg];
    // The blue shell re-targets whoever leads (never its owner); a red one keeps its pick.
    if (sh.kind === 'blue') { const owner = state.racers[sh.owner]; sh.target = owner ? leaderBesides(state, owner)?.id ?? -1 : -1; }
    const target = sh.target >= 0 ? state.racers[sh.target] : undefined;
    const ground = Math.max(track.ground(sh.x, sh.z, near, state.raceTime), near.y - 0.5);
    // Spent green and red shells just vanish; a blue one or a bomb goes off where it is.
    let gone = sh.life <= 0 && (sh.kind === 'green' || sh.kind === 'red');
    if (sh.kind === 'green' || sh.kind === 'bomb') {
      if (sh.kind === 'bomb') {
        // Lobbed: arcs, lands, rolls to a stop.
        sh.vy -= GRAVITY * STEP; sh.y += sh.vy * STEP;
        if (sh.y <= ground + 0.5) { sh.y = ground + 0.5; sh.vy = 0; sh.speed = approach(sh.speed, 0, 4, STEP); }
      } else sh.y = approach(sh.y, ground + 0.7, 10, STEP);
      sh.x += Math.sin(sh.yaw) * sh.speed * STEP;
      sh.z += Math.cos(sh.yaw) * sh.speed * STEP;
      const edge = Math.abs(near.lateral) - path.halfWidth;
      if (path.edge === 'wall' || (path.flags[near.index] & F_RAILS) !== 0) {
        if (edge > 0.9 && near.lateral * (Math.sin(sh.yaw) * near.tz - Math.cos(sh.yaw) * near.tx) > 0) {
          // Bounce: mirror the heading about the road's direction.
          sh.yaw = wrapAngle(2 * Math.atan2(near.tx, near.tz) - sh.yaw);
        }
      } else if (edge > path.outOfBounds) gone = true; // off into the scenery
    } else {
      let tx: number, tz: number;
      const close = sh.kind === 'blue' ? 40 : 45;
      if (target && target.respawn <= 0 && Math.hypot(target.x - sh.x, target.z - sh.z) < close) { tx = target.x; tz = target.z; }
      else {
        // Follow the road (onto the target's route when it is on a branch).
        advance(track, near.seg, near.s, sh.kind === 'blue' ? 24 : 16, target && target.seg > 0 ? 1 << (target.seg - 1) : 0, _spot);
        pointAt(track.paths[_spot.seg], _spot.s, 0, _pt); tx = _pt[0]; tz = _pt[2];
      }
      const want = Math.atan2(tx - sh.x, tz - sh.z), turn = sh.kind === 'blue' ? 7 : 4;
      sh.yaw = wrapAngle(sh.yaw + clamp(wrapAngle(want - sh.yaw), -turn * STEP, turn * STEP));
      sh.x += Math.sin(sh.yaw) * sh.speed * STEP;
      sh.z += Math.cos(sh.yaw) * sh.speed * STEP;
      // Red shells skim the road (over gaps and water at road height); the blue one flies high until it dives.
      const dive = sh.kind === 'blue' && target ? clamp((Math.hypot(target.x - sh.x, target.z - sh.z) - 3) / 14, 0, 1) : 1;
      sh.y = approach(sh.y, ground + 0.7 + (sh.kind === 'blue' ? 6 * dive : 0), 10, STEP);
    }
    if (!gone && sh.kind === 'blue') {
      // Only its target stops it: it passes over everyone else.
      if (target && target.respawn <= 0 && Math.hypot(target.x - sh.x, target.z - sh.z) < 3) { blast(state, sh, SHELL[sh.kind].blast); gone = true; }
      else if (sh.life <= 0) { blast(state, sh, SHELL[sh.kind].blast); gone = true; }
    } else if (!gone) {
      for (const r of state.racers) {
        if (r.respawn > 0 || r.id === sh.owner && (sh.kind !== 'green' || SHELL.green.life - sh.life < GREEN_GRACE)) continue;
        const dx = r.x - sh.x, dz = r.z - sh.z;
        if (dx * dx + dz * dz < 2.2 * 2.2 && Math.abs(r.y + 0.6 - sh.y) < 2.2) {
          if (sh.kind === 'bomb') blast(state, sh, SHELL.bomb.blast);
          else { emit(state, 'explode', { id: r.id, x: sh.x, y: sh.y, z: sh.z }, SHELL_KINDS.indexOf(sh.kind)); spinOut(state, r, 1.0); }
          gone = true;
          break;
        }
      }
      // A bomb's fuse runs out where it lies.
      if (!gone && sh.kind === 'bomb' && sh.life <= 0) { blast(state, sh, SHELL.bomb.blast); gone = true; }
    }
    if (!gone && sh.kind !== 'blue') for (let h = state.hazards.length - 1; h >= 0; h--) {
      const hz = state.hazards[h];
      if (Math.hypot(hz.x - sh.x, hz.z - sh.z) < 1.6) {
        state.hazards.splice(h, 1); gone = true;
        if (sh.kind === 'bomb') blast(state, sh, SHELL.bomb.blast);
        else emit(state, 'explode', { id: sh.owner, x: sh.x, y: sh.y, z: sh.z }, SHELL_KINDS.indexOf(sh.kind));
        break;
      }
    }
    if (gone) state.shells.splice(i, 1);
  }
}
