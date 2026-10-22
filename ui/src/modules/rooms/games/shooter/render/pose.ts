/**
 * Procedural poses for the articulated soldiers and the infected: one pure
 * function from what a body is doing (speed and stride phase, air, crouch,
 * slide, aim, weapon grip, recoil, pickaxe swing, knocked, dying, infected)
 * to joint angles. No three.js: the crowd renderer turns these into instance
 * matrices, and unit tests check the poses directly. Angles are radians;
 * hips and shoulders pitch forward when positive, knees and elbows bend when
 * positive, the root tilts forward when positive (negative falls back).
 */
import { clamp } from '../../common/math.ts';

/** How the body is holding things: nothing, a pistol / heal (one hand), a long gun, the pickaxe. */
export type Grip = 0 | 1 | 2 | 3;
export const GRIP_NONE = 0, GRIP_ONE = 1, GRIP_TWO = 2, GRIP_PICK = 3;

export interface PoseInput {
  /** Ground speed (m/s) and stride phase (radians, advanced by distance walked). */
  speed: number; phase: number;
  air: boolean; sprint: boolean;
  /** 0..1 blends: crouched, sliding, sights up, recoil kick, pickaxe swing progress (0 idle). */
  crouch: number; slide: number; aim: number; kick: number; swing: number;
  grip: Grip;
  /** View pitch (radians, up positive) the head and gun follow. */
  pitch: number;
  /** Knocked (crawling) and death progress 0..1 (0 alive). */
  down: boolean; dead: number;
  infected: boolean;
  /** Seconds, for idle breathing and the infected's sway. */
  time: number;
}
export interface Pose {
  /** Root: lowered by `drop` metres, pitched (forward +) about the hips, rolled. */
  drop: number; tilt: number; roll: number;
  spine: number; twist: number; head: number;
  hipL: number; hipR: number; kneeL: number; kneeR: number;
  /** Shoulders: forward raise, outward roll; elbows bend. */
  shL: number; shR: number; rollL: number; rollR: number; elL: number; elR: number;
  /** The held item relative to the chest: offset (m), pitch and yaw. */
  gunX: number; gunY: number; gunZ: number; gunPitch: number; gunYaw: number;
}

export const emptyPose = (): Pose => ({
  drop: 0, tilt: 0, roll: 0, spine: 0, twist: 0, head: 0, hipL: 0, hipR: 0, kneeL: 0, kneeR: 0,
  shL: 0, shR: 0, rollL: 0, rollR: 0, elL: 0, elR: 0, gunX: 0, gunY: 0, gunZ: 0, gunPitch: 0, gunYaw: 0,
});

/** Stride length (m per full cycle) for advancing the phase: phase += speed / STRIDE * 2PI * dt. */
export const STRIDE = 1.6;

export function solvePose(i: PoseInput, out: Pose): Pose {
  const amp = clamp(i.speed / 5.5, 0, 1.25), s = Math.sin(i.phase), c = Math.cos(i.phase);
  const breathe = Math.sin(i.time * 1.7) * 0.02;
  out.drop = 0; out.tilt = 0; out.roll = 0; out.twist = 0;
  out.spine = 0.04 + amp * (i.sprint ? 0.22 : 0.08) + breathe;
  out.head = clamp(i.pitch, -0.9, 0.9) * 0.6;
  // Gait: hips swing opposite, knees bend on the swing-through.
  out.hipL = s * amp * 0.62; out.hipR = -s * amp * 0.62;
  out.kneeL = Math.max(0, -c) * amp * 1.05 + 0.06; out.kneeR = Math.max(0, c) * amp * 1.05 + 0.06;
  if (i.air) { out.hipL = 0.55; out.hipR = 0.15; out.kneeL = 0.95; out.kneeR = 0.45; }
  // Arms: swing opposite the legs, unless something is held.
  out.shL = -s * amp * 0.55; out.shR = s * amp * 0.55; out.rollL = 0.08; out.rollR = 0.08;
  out.elL = 0.25 + amp * 0.55; out.elR = 0.25 + amp * 0.55;
  out.gunX = 0.14; out.gunY = 0.02; out.gunZ = 0.28; out.gunPitch = -0.45; out.gunYaw = 0;
  const p = clamp(i.pitch, -1.1, 1.1);
  if (i.grip === GRIP_TWO) {
    if (i.sprint && i.aim < 0.2) {
      // Sprinting: the gun across the chest, muzzle up and left.
      out.shR = 0.55; out.elR = 1.55; out.shL = 0.95; out.elL = 1.25; out.rollL = -0.3;
      out.gunX = 0.02; out.gunY = 0.06; out.gunZ = 0.24; out.gunPitch = 0.55; out.gunYaw = 0.95;
    } else {
      // Ready (low) to aimed (stock in the shoulder, sights at the eye).
      const a = i.aim;
      out.shR = 0.75 + a * 0.55 + p * 0.75; out.elR = 1.55 - a * 0.45; out.rollR = 0.35 - a * 0.2;
      out.shL = 1.15 + a * 0.3 + p * 0.75; out.elL = 0.55 - a * 0.15; out.rollL = -0.45 + a * 0.1;
      out.gunX = 0.13 - a * 0.06; out.gunY = 0.02 + a * 0.24; out.gunZ = 0.26 + a * 0.04;
      out.gunPitch = lerpN(-0.42, 0, a) + p;
    }
  } else if (i.grip === GRIP_ONE) {
    const a = Math.max(i.aim, 0.35);
    out.shR = 0.6 + a * 0.85 + p * 0.8; out.elR = 0.9 - a * 0.75; out.rollR = 0.2 - a * 0.3;
    if (i.aim > 0.3) { out.shL = 1.2 + p * 0.8; out.elL = 0.4; out.rollL = -0.45; }
    out.gunX = 0.14 - a * 0.12; out.gunY = 0.02 + a * 0.2; out.gunZ = 0.3 + a * 0.18; out.gunPitch = lerpN(-0.4, 0, a) + p;
  } else if (i.grip === GRIP_PICK) {
    // Carried on the shoulder; a swing goes overhead and chops down.
    const t = i.swing;
    out.shR = t > 0 ? 2.6 - Math.sin(t * Math.PI) * 0.2 - t * 2.5 : 0.5; out.elR = t > 0 ? 0.4 + (1 - t) * 0.5 : 1.3;
    out.gunX = 0.16; out.gunY = t > 0 ? 0.35 - t * 0.45 : 0.18; out.gunZ = 0.2 + t * 0.25; out.gunPitch = t > 0 ? 1.2 - t * 2.4 : 1.0;
  }
  // Recoil: shoulders and spine kick back.
  if (i.kick > 0 && (i.grip === GRIP_ONE || i.grip === GRIP_TWO)) {
    out.shR -= i.kick * 0.14; out.shL -= i.kick * 0.1; out.spine -= i.kick * 0.06; out.gunPitch += i.kick * 0.18; out.gunZ -= i.kick * 0.05;
  }
  if (i.crouch > 0) {
    const k = i.crouch;
    out.drop += k * 0.42; out.spine += k * 0.22;
    out.hipL += k * 1.05; out.hipR += k * 0.55; out.kneeL += k * 1.75; out.kneeR += k * 1.25;
  }
  if (i.slide > 0) {
    // Feet first: lean back, front leg out straight, back leg folded under.
    const k = i.slide;
    out.drop += k * 0.62; out.tilt -= k * 0.42; out.spine -= k * 0.1;
    out.hipL = lerpN(out.hipL, 1.45, k); out.kneeL = lerpN(out.kneeL, 0.12, k);
    out.hipR = lerpN(out.hipR, 0.45, k); out.kneeR = lerpN(out.kneeR, 1.8, k);
    if (i.grip === GRIP_NONE) { out.shL = lerpN(out.shL, -0.6, k); out.elL = 0.2; }
  }
  if (i.infected) {
    // Shambling: arms reaching forward, hunched, head lolling, a limp.
    const sway = Math.sin(i.time * 2.3 + i.phase * 0.5);
    out.spine = 0.38 + amp * 0.15; out.roll = sway * 0.08; out.head = -0.25 + sway * 0.12; out.twist = sway * 0.1;
    out.shL = 1.35 + Math.sin(i.time * 3.1) * 0.15; out.shR = 1.25 + Math.cos(i.time * 2.7) * 0.15;
    out.elL = 0.25; out.elR = 0.35; out.rollL = -0.12; out.rollR = -0.08;
    out.kneeR += 0.25; out.hipR *= 0.6;
    if (i.kick > 0) { out.shL += i.kick * 0.9; out.shR += i.kick * 0.9; out.spine += i.kick * 0.25; }
  }
  if (i.down) {
    // On hands and knees.
    out.drop = 0.52; out.tilt = 1.05; out.spine = 0.1; out.head = -0.6;
    out.hipL = 1.5 + s * amp * 0.3; out.hipR = 1.5 - s * amp * 0.3; out.kneeL = 1.55; out.kneeR = 1.55;
    out.shL = 1.35 - s * amp * 0.3; out.shR = 1.35 + s * amp * 0.3; out.elL = 0.2; out.elR = 0.2; out.rollL = 0.1; out.rollR = 0.1;
  }
  if (i.dead > 0) {
    // Fall back and lie flat, limbs loose.
    const k = clamp(i.dead, 0, 1), e = k * k * (3 - 2 * k);
    out.tilt = lerpN(out.tilt, -1.5, e); out.drop = lerpN(out.drop, 0.82, e); out.spine = lerpN(out.spine, -0.15, e); out.head = lerpN(out.head, -0.3, e);
    out.hipL = lerpN(out.hipL, 0.25, e); out.hipR = lerpN(out.hipR, -0.05, e); out.kneeL = lerpN(out.kneeL, 0.5, e); out.kneeR = lerpN(out.kneeR, 0.15, e);
    out.shL = lerpN(out.shL, 2.6, e); out.shR = lerpN(out.shR, 0.4, e); out.rollL = lerpN(out.rollL, 0.6, e); out.rollR = lerpN(out.rollR, 0.9, e);
    out.elL = lerpN(out.elL, 0.3, e); out.elR = lerpN(out.elR, 0.2, e);
  }
  return out;
}
const lerpN = (a: number, b: number, t: number): number => a + (b - a) * t;
