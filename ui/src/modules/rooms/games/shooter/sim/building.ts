// Building: placing and editing structures, damage and collapse of
// unsupported pieces, and harvested props regrowing.
import { groundAt, raycast, type World } from '../collide.ts';
import { BUILD_COST, MATERIALS } from '../weapons.ts';
import { canPlace, makeBuild, MAX_BUILDS, maxHp, planBuild, refreshGeometry, unsupported, type Build } from '../builds.ts';
import { STEP, EYE, type Fighter, type ShooterState } from './model.ts';
import { emit, fwd } from './core.ts';

// ------------------------------------------------------------ building

/** Static ground (terrain + props) for structural support checks. */
const groundFor = (world: World) => (x: number, z: number, feet: number) => groundAt(world, x, z, feet);

export function tryBuild(state: ShooterState, world: World, p: Fighter): void {
  if (p.mats[p.mat] < BUILD_COST || state.builds.length >= MAX_BUILDS) return;
  const slot = planBuild(p.piece, p.x, p.y, p.z, p.yaw, p.pitch, state.builds);
  if (!canPlace(slot, state.builds, groundFor(world))) return;
  const b = makeBuild(state.nextId++, slot, p.mat, p.id);
  state.builds.push(b);
  state.buildRev++;
  p.mats[p.mat] -= BUILD_COST;
  p.cooldown = 0.12;
  const a = b.aabb;
  emit(state, 'build', p.id, (a.minX + a.maxX) / 2, a.minY, (a.minZ + a.maxZ) / 2, { value: b.kind, flag: b.mat });
}

/** Edits the survivor's own structure in the crosshair: walls cycle solid → door → window, ramps flip. */
export function tryEdit(state: ShooterState, world: World, p: Fighter): void {
  const [fx, fz] = fwd(p.yaw), cp = Math.cos(p.pitch);
  const hit = raycast(world, p.x, p.y + EYE, p.z, fx * cp, Math.sin(p.pitch), fz * cp, 6, true, state.builds);
  if (hit.kind !== 'build') return;
  const b = state.builds[hit.index];
  if (b.owner !== p.id || b.kind === 1) return;
  if (b.kind === 0) b.variant = (b.variant + 1) % 3;
  else b.dir = (b.dir + 2) % 4;
  refreshGeometry(b);
  state.buildRev++;
  p.cooldown = Math.max(p.cooldown, 0.2);
  emit(state, 'edit', p.id, (b.aabb.minX + b.aabb.maxX) / 2, b.aabb.minY + 1.5, (b.aabb.minZ + b.aabb.maxZ) / 2, { value: b.kind, flag: b.variant });
}

/** Damages a structure; destroyed pieces fall, and so does anything they were holding up. */
export function damageBuild(state: ShooterState, world: World, b: Build, amount: number): void {
  b.hp -= amount;
  state.buildRev++;
  if (b.hp > 0) return;
  removeBuilds(state, [state.builds.indexOf(b)]);
  const falling = unsupported(state.builds, groundFor(world));
  if (falling.length) removeBuilds(state, falling);
}
function removeBuilds(state: ShooterState, indices: number[]): void {
  for (let k = indices.length - 1; k >= 0; k--) {
    const i = indices[k];
    if (i < 0) continue;
    const b = state.builds[i], a = b.aabb;
    state.builds.splice(i, 1);
    emit(state, 'break', b.owner, (a.minX + a.maxX) / 2, (a.minY + a.maxY) / 2, (a.minZ + a.maxZ) / 2, { value: b.kind, flag: b.mat });
  }
  state.buildRev++;
}
export function growBuilds(state: ShooterState): void {
  for (const b of state.builds) {
    if (b.grow <= 0) continue;
    const m = MATERIALS[b.mat];
    b.grow = Math.max(0, b.grow - STEP);
    b.hp = Math.min(maxHp(b), b.hp + (m.hp * (1 - m.start) / m.time) * STEP);
    // Finished going up: guests and the renderer drop the "under construction" look.
    if (b.grow === 0) state.buildRev++;
  }
}
/** Forgets a harvested prop (it is whole again; a felled tree stands back up). */
export function dropNode(state: ShooterState, index: number): void {
  const n = state.nodes[index];
  state.nodes.splice(index, 1);
  if (n.key < 0 && state.felled.includes(-1 - n.key)) state.felled = state.felled.filter(i => i !== -1 - n.key);
}
export function stepNodes(state: ShooterState): void {
  for (let i = state.nodes.length - 1; i >= 0; i--) if ((state.nodes[i].regrow -= STEP) <= 0) dropNode(state, i);
}
