// Fit the authored camera to the model for the viewport's aspect: keep the
// camera's direction and target, and dolly OUT just far enough that every
// corner of the content's bounds lands inside the frustum (with a margin, plus
// a reserved band at the top for the viewport's overlay buttons). Never moves
// closer than the authored distance, so a framing that already fits is kept.
// Pure math (no three.js) so it is unit-tested directly.

export type V3 = readonly [number, number, number];

export interface FitInput {
  /** Authored camera position and orbit target. */
  position: V3;
  target: V3;
  /** Vertical field of view, degrees (three.js PerspectiveCamera.fov). */
  fov: number;
  /** Viewport width / height. */
  aspect: number;
  /** Content bounds (world space). */
  min: V3;
  max: V3;
  /** Fractions of the viewport height reserved at the top (overlay buttons)
   *  and bottom (states bar, stats pill), 0..0.4 each. */
  topReserve?: number;
  bottomReserve?: number;
  /** Fraction of each half-extent kept free around the model, 0..0.5. */
  margin?: number;
  /** May come closer than the authored distance (a model with no authored framing to keep). */
  closer?: boolean;
  /** Each piece's own box: framed corner by corner instead of `min`/`max` (see {@link fitCamera}). */
  boxes?: (readonly [V3, V3])[];
}

const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a: V3, b: V3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const norm = (a: V3): V3 => {
  const l = Math.hypot(a[0], a[1], a[2]);
  return l > 1e-9 ? [a[0] / l, a[1] / l, a[2] / l] : [0, 0, 0];
};

/** The camera position that frames the bounds (the authored one when it already does). */
export function fitCameraPosition(i: FitInput): V3 {
  const back = sub(i.position, i.target); // target → camera
  const dist = Math.hypot(back[0], back[1], back[2]);
  if (!(dist > 0) || !(i.aspect > 0) || !(i.fov > 0)) return i.position;
  const w = norm(back);
  let right = norm(cross([0, 1, 0], w));
  if (right[0] === 0 && right[1] === 0 && right[2] === 0) right = [1, 0, 0]; // looking straight up/down
  const up = cross(w, right);
  const tanV = Math.tan((i.fov * Math.PI) / 360);
  const tanH = tanV * i.aspect;
  const k = 1 - Math.min(0.5, Math.max(0, i.margin ?? 0.12));
  // NDC y runs -1..1; a reserved band takes twice its fraction off that half's reach.
  const band = (f: number | undefined): number => Math.max(0.1, k - 2 * Math.min(0.4, Math.max(0, f ?? 0)));
  const kTop = band(i.topReserve);
  const kBottom = band(i.bottomReserve);
  let need = i.closer ? 0 : dist;
  for (const p of corners(i)) {
    const q = sub(p, i.target);
    const along = dot(q, w); // toward the camera
    const x = Math.abs(dot(q, right));
    const y = dot(q, up);
    need = Math.max(need, along + x / (tanH * k), along + (y > 0 ? y / (tanV * kTop) : -y / (tanV * kBottom)));
  }
  if (!Number.isFinite(need) || need <= 0 || (!i.closer && need <= dist + 1e-6)) return i.position;
  return [i.target[0] + w[0] * need, i.target[1] + w[1] * need, i.target[2] + w[2] * need];
}

function corners(i: FitInput): V3[] {
  return (i.boxes ?? [[i.min, i.max] as const]).flatMap(([lo, hi]) =>
    Array.from({ length: 8 }, (_, c): V3 => [c & 1 ? hi[0] : lo[0], c & 2 ? hi[1] : lo[1], c & 4 ? hi[2] : lo[2]]),
  );
}

/** The camera that frames the content, its position and orbit target. With `boxes` (a group of
 *  pieces spread over the ground) the fit reads each piece's own corners, and the target moves
 *  until the pieces sit in the middle of the picture as the camera sees them (a big piece near
 *  the camera otherwise pulls the group to one side), so the group fills the view instead of the
 *  empty corners of its joint box. */
export function fitCamera(i: FitInput): { position: V3; target: V3 } {
  if (!i.boxes?.length || !(i.aspect > 0) || !(i.fov > 0)) return { position: fitCameraPosition(i), target: i.target };
  const back = sub(i.position, i.target);
  const w = norm(back);
  const right = norm(cross([0, 1, 0], w));
  const up = cross(w, right);
  const tanV = Math.tan((i.fov * Math.PI) / 360);
  const tanH = tanV * i.aspect;
  const at = (t: V3): V3 => fitCameraPosition({ ...i, target: t, position: [t[0] + back[0], t[1] + back[1], t[2] + back[2]] });
  let target = i.target;
  for (let n = 0; n < 4; n++) {
    const pos = at(target);
    const d = Math.hypot(...sub(pos, target));
    const xs: number[] = [];
    const ys: number[] = [];
    for (const p of corners(i)) {
      const q = sub(p, pos);
      const z = Math.max(1e-6, -dot(q, w));
      xs.push(dot(q, right) / (z * tanH));
      ys.push(dot(q, up) / (z * tanV));
    }
    const dx = ((Math.min(...xs) + Math.max(...xs)) / 2) * d * tanH;
    const dy = ((Math.min(...ys) + Math.max(...ys)) / 2) * d * tanV;
    target = [0, 1, 2].map((k) => target[k] + right[k] * dx + up[k] * dy) as unknown as V3;
  }
  return { position: at(target), target };
}
