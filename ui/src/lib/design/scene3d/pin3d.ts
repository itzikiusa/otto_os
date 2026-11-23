// Pure 3D polygon/surface comment anchor helpers: formatting, cluster merging,
// camera framing math, and screen-space projection for 3D-anchored pins.
// Zero DOM / Three.js runtime imports so every helper is unit-tested in Node.

export type SelectionKind3d = 'point' | 'polygon' | 'region' | 'mesh';

export interface CameraPose3d {
  pos: [number, number, number];
  target: [number, number, number];
  fov?: number | null;
}

/** Structural match to `CommentPin3d` (backend `otto_design::comments::CommentPin3d`). */
export interface Pin3dAnchor {
  selection_kind: SelectionKind3d;
  mesh_name?: string | null;
  object_id?: string | null;
  face_index?: number | null;
  face_indices?: number[];
  vertices?: [number, number, number] | null;
  position: [number, number, number];
  normal?: [number, number, number] | null;
  camera?: CameraPose3d | null;
}

export interface ProjectedPin3d {
  id: string;
  n: number;
  x: number;
  y: number;
  visible: boolean;
  backfacing: boolean;
  muted: boolean;
  label: string;
  body: string;
  anchor: Pin3dAnchor;
}

const MAX_REGION_FACES = 2048;

export const round3 = (n: number): number => (Number.isFinite(n) ? Math.round(n * 1000) / 1000 : 0);

export function roundVec3(v: readonly [number, number, number]): [number, number, number] {
  return [round3(v[0]), round3(v[1]), round3(v[2])];
}

/** Dedupe + cap face indices while preserving insertion order (matches backend 2,048 cap). */
export function normalizeFaceIndices(indices: readonly number[], max = MAX_REGION_FACES): number[] {
  const seen = new Set<number>();
  const out: number[] = [];
  for (const idx of indices) {
    if (!Number.isInteger(idx) || idx < 0 || seen.has(idx)) continue;
    seen.add(idx);
    out.push(idx);
    if (out.length >= max) break;
  }
  return out;
}

/** Toggle/append a face onto an existing polygon/region selection on the same mesh. */
export function mergeFaceIntoSelection(
  current: Pin3dAnchor | null,
  hit: Pin3dAnchor,
): Pin3dAnchor {
  if (
    !current ||
    hit.face_index == null ||
    (current.mesh_name ?? '') !== (hit.mesh_name ?? '') ||
    (current.object_id ?? '') !== (hit.object_id ?? '')
  ) {
    return hit;
  }
  const existing = current.face_indices?.length
    ? [...current.face_indices]
    : current.face_index != null
      ? [current.face_index]
      : [];
  const next = existing.includes(hit.face_index)
    ? existing.filter((f) => f !== hit.face_index)
    : [...existing, hit.face_index];
  const deduped = normalizeFaceIndices(next);
  if (deduped.length === 0) return hit;
  return {
    ...hit,
    selection_kind: deduped.length > 1 ? 'region' : 'polygon',
    face_index: deduped[0],
    face_indices: deduped,
  };
}

/** Compact chip label shown on comment cards & floating polygon readout. */
export function pin3dLabel(pin: Pin3dAnchor): string {
  const mesh = pin.mesh_name?.trim() || pin.object_id?.trim() || 'Mesh';
  const count = pin.face_indices?.length ?? 0;
  switch (pin.selection_kind) {
    case 'polygon':
      return pin.face_index != null ? `${mesh} · Face #${pin.face_index}` : `${mesh} · Polygon`;
    case 'region': {
      if (count > 1) return `${mesh} · ${count} faces (#${pin.face_indices![0]}…)`;
      return pin.face_index != null ? `${mesh} · Face #${pin.face_index}` : `${mesh} · Region`;
    }
    case 'mesh':
      return `Mesh: ${mesh}`;
    case 'point':
    default:
      return pin.position
        ? `${mesh} (${round3(pin.position[0])}, ${round3(pin.position[1])}, ${round3(pin.position[2])})`
        : mesh;
  }
}

/** Detailed human + agent-readable description (screen readers, Markdown dispatch, regenerate prompts). */
export function pin3dWords(pin: Pin3dAnchor): string {
  const parts: string[] = [];
  const mesh = pin.mesh_name?.trim() || pin.object_id?.trim();
  const kindLabel =
    pin.selection_kind === 'polygon'
      ? '3D polygon'
      : pin.selection_kind === 'region'
        ? '3D polygon region'
        : pin.selection_kind === 'mesh'
          ? '3D mesh'
          : '3D point';
  parts.push(mesh ? `${kindLabel} on "${mesh}"` : kindLabel);
  if (pin.face_index != null) {
    const extra = pin.face_indices && pin.face_indices.length > 1 ? ` (+${pin.face_indices.length - 1} faces)` : '';
    parts.push(`face #${pin.face_index}${extra}`);
  } else if (pin.face_indices && pin.face_indices.length > 0) {
    parts.push(`${pin.face_indices.length} faces`);
  }
  if (pin.vertices) {
    parts.push(`verts [${pin.vertices.join(', ')}]`);
  }
  if (pin.position) {
    parts.push(`at (${round3(pin.position[0])}, ${round3(pin.position[1])}, ${round3(pin.position[2])})`);
  }
  return parts.join(' · ');
}

/** Whether a surface normal points away from the camera position (for subtle backface pin dimming). */
export function isBackfacing(
  point: readonly [number, number, number],
  normal: readonly [number, number, number] | null | undefined,
  cameraPos: readonly [number, number, number],
): boolean {
  if (!normal) return false;
  const vx = cameraPos[0] - point[0];
  const vy = cameraPos[1] - point[1];
  const vz = cameraPos[2] - point[2];
  const dot = vx * normal[0] + vy * normal[1] + vz * normal[2];
  return dot < -0.08;
}

/** Convert normalized device coords (-1..1) into viewport pixel coords (clamped status). */
export function ndcToViewportPx(
  ndcX: number,
  ndcY: number,
  ndcZ: number,
  width: number,
  height: number,
): { x: number; y: number; inFront: boolean; inBounds: boolean } {
  const inFront = Number.isFinite(ndcZ) && ndcZ >= -1 && ndcZ <= 1;
  const x = ((ndcX + 1) * 0.5) * width;
  const y = ((1 - ndcY) * 0.5) * height;
  const inBounds = inFront && x >= -24 && x <= width + 24 && y >= -24 && y <= height + 24;
  return { x, y, inFront, inBounds };
}

/** Compute a clean camera pose framing a 3D point + surface normal when no saved camera exists. */
export function defaultCameraForAnchor(
  anchor: Pin3dAnchor,
  distance = 2.4,
): CameraPose3d | null {
  if (anchor.camera) return anchor.camera;
  if (!anchor.position) return null;
  const [px, py, pz] = anchor.position;
  const n = anchor.normal ?? [0.5, 0.45, 0.74];
  const len = Math.hypot(n[0], n[1], n[2]) || 1;
  const nx = n[0] / len;
  const ny = n[1] / len;
  const nz = n[2] / len;
  const d = Math.max(0.6, distance);
  return {
    pos: roundVec3([px + nx * d, py + Math.max(ny, 0.25) * d, pz + nz * d]),
    target: roundVec3([px, py, pz]),
    fov: 45,
  };
}
