// scene3d v2 states — named snapshots (Idle / Hover / Flipped…) as per-object
// overrides of the base document, and the tween math between two of them.
// Pure (type imports only): the studio viewport, the embed runtime and the
// unit tests share it. A state never rewrites the base document; the viewer
// computes the pose of every object for "base + overrides" and interpolates.
import type { Easing, Scene3dDoc, Scene3dObject, Scene3dState, Scene3dStateOverride, Vec3 } from './types';

/** Transition defaults when a state doesn't say. */
export const DEFAULT_DURATION_MS = 400;
export const DEFAULT_EASING: Easing = 'ease-in-out';
export const MAX_DURATION_MS = 10_000;

/** What the renderer applies per object. Colours are resolved `#rrggbb` (or null = material's own). */
export interface Pose {
  position: Vec3;
  rotation: Vec3; // degrees
  scale: Vec3;
  visible: boolean;
  /** null = the material's own opacity. */
  opacity: number | null;
  color: string | null;
  emissive: string | null;
}

// ── Easing ───────────────────────────────────────────────────────────────────

/** Map linear progress t ∈ [0,1] through an easing curve. Always 0 at 0 and 1 at 1. */
export function ease(name: Easing | undefined, t: number): number {
  const x = Math.min(1, Math.max(0, Number.isFinite(t) ? t : 1));
  switch (name ?? DEFAULT_EASING) {
    case 'linear':
      return x;
    case 'ease-in':
      return x * x * x;
    case 'ease-out':
      return 1 - (1 - x) ** 3;
    case 'spring': {
      if (x === 0 || x === 1) return x;
      // Damped oscillation that overshoots once and settles on 1.
      return 1 - Math.exp(-6 * x) * Math.cos(10 * x);
    }
    case 'ease-in-out':
    default:
      return x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2;
  }
}

/** Eased progress of a transition `elapsed` ms in; a 0 ms transition is already done. */
export function transitionProgress(elapsedMs: number, durationMs: number, easing?: Easing): number {
  if (durationMs <= 0) return 1;
  return ease(easing, elapsedMs / durationMs);
}

/** Duration + easing of a transition INTO `state` (reduced motion → instant). */
export function stateTiming(state: Scene3dState | null | undefined, reducedMotion = false): { duration: number; easing: Easing } {
  const easing = state?.easing ?? DEFAULT_EASING;
  if (reducedMotion) return { duration: 0, easing };
  const d = state?.duration_ms ?? DEFAULT_DURATION_MS;
  return { duration: Math.min(MAX_DURATION_MS, Math.max(0, Number.isFinite(d) ? d : DEFAULT_DURATION_MS)), easing };
}

// ── Poses ────────────────────────────────────────────────────────────────────

export function basePose(o: Scene3dObject): Pose {
  return {
    position: [...o.position] as Vec3,
    rotation: [...o.rotation] as Vec3,
    scale: [...o.scale] as Vec3,
    visible: o.visible !== false,
    opacity: null,
    color: null,
    emissive: null,
  };
}

export function findState(doc: Scene3dDoc, id: string | null | undefined): Scene3dState | null {
  if (!id) return null;
  return doc.states?.find((s) => s.id === id) ?? null;
}

/** The state a viewer starts in: `default_state`, else the first state, else none (base). */
export function initialState(doc: Scene3dDoc): string | null {
  if (doc.default_state && findState(doc, doc.default_state)) return doc.default_state;
  return doc.states?.[0]?.id ?? null;
}

/**
 * Where an edit of `objectId` goes while `shownStateId` is showing: the base
 * document (null), or — when a non-default state is showing, or the showing
 * state already overrides this object — that state's overrides (Spline-style).
 */
export function editTargetState(doc: Scene3dDoc, shownStateId: string | null | undefined, objectId: string): string | null {
  const st = findState(doc, shownStateId);
  if (!st) return null;
  if (st.id !== initialState(doc) || st.overrides?.[objectId]) return st.id;
  return null;
}

function applyOverride(p: Pose, ov: Scene3dStateOverride, color: (ref: string) => string): Pose {
  return {
    position: ov.position ? ([...ov.position] as Vec3) : p.position,
    rotation: ov.rotation ? ([...ov.rotation] as Vec3) : p.rotation,
    scale: ov.scale ? ([...ov.scale] as Vec3) : p.scale,
    visible: ov.visible ?? p.visible,
    opacity: ov.opacity ?? p.opacity,
    color: ov.color ? color(ov.color) : p.color,
    emissive: ov.emissive ? color(ov.emissive) : p.emissive,
  };
}

/**
 * The pose of every object in `stateId` (base + that state's overrides). An
 * unknown / null state is the base document. `color` resolves hex / token
 * override colours.
 */
export function statePoses(doc: Scene3dDoc, stateId: string | null | undefined, color: (ref: string) => string = (c) => c): Map<string, Pose> {
  const st = findState(doc, stateId);
  const out = new Map<string, Pose>();
  for (const o of doc.objects) {
    const ov = st?.overrides?.[o.id];
    out.set(o.id, ov ? applyOverride(basePose(o), ov, color) : basePose(o));
  }
  return out;
}

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;
const lerpVec = (a: Vec3, b: Vec3, t: number): Vec3 => [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)];

function hexToRgb(h: string): [number, number, number] | null {
  const m = /^#([0-9a-f]{6})$/i.exec(h);
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** Interpolate two `#rrggbb` colours in sRGB (null / malformed → snap at the end). */
export function lerpColor(a: string | null, b: string | null, t: number): string | null {
  if (a === b) return a;
  const ra = a ? hexToRgb(a) : null;
  const rb = b ? hexToRgb(b) : null;
  if (!ra || !rb) return t >= 1 ? b : a;
  const c = [0, 1, 2].map((i) => Math.round(lerp(ra[i], rb[i], t)));
  return `#${c.map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

/**
 * Pose between `a` and `b` at eased progress `t` (the easing may overshoot:
 * `t` outside [0,1] extrapolates transforms but not colours/visibility).
 * Visibility: something appearing shows immediately; something disappearing
 * stays until the end (so it can fade via opacity first).
 */
export function lerpPose(a: Pose, b: Pose, t: number): Pose {
  const tc = Math.min(1, Math.max(0, t));
  const oa = a.opacity ?? 1;
  const ob = b.opacity ?? 1;
  return {
    position: lerpVec(a.position, b.position, t),
    rotation: lerpVec(a.rotation, b.rotation, t),
    scale: lerpVec(a.scale, b.scale, t),
    visible: b.visible ? true : tc < 1 ? a.visible : false,
    opacity: a.opacity === null && b.opacity === null ? null : lerp(oa, ob, tc),
    color: lerpColor(a.color, b.color, tc),
    emissive: lerpColor(a.emissive, b.emissive, tc),
  };
}

/** Interpolate whole pose maps (objects missing on one side snap to the other). */
export function lerpPoses(a: Map<string, Pose>, b: Map<string, Pose>, t: number): Map<string, Pose> {
  const out = new Map<string, Pose>();
  for (const [id, pb] of b) {
    const pa = a.get(id);
    out.set(id, pa ? lerpPose(pa, pb, t) : pb);
  }
  return out;
}
