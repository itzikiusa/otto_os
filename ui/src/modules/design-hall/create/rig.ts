// Whether a made piece can move: its skeleton and its moves, read from the
// model file itself (a GLB's JSON part: `skins` and `animations`), so
// what Create says ("Skeleton added · 3 moves") is what the file really
// carries (rigRead.ts reads only the start of the file). The moves are listed in one plain order
// (Stand, Walk, Run, then the rest). Unit-tested in unit/createProgress.test.ts.
import { moveLabel } from './plan.ts';

export interface GlbRig {
  skeleton: boolean;
  /** The clip names, as the file has them. */
  clips: string[];
}

/** Skeleton and clips from a glTF JSON document. */
export function rigFromGltf(json: { skins?: unknown[]; animations?: { name?: string }[] }): GlbRig {
  return { skeleton: (json.skins?.length ?? 0) > 0, clips: (json.animations ?? []).map((a, k) => a.name?.trim() || `Clip ${k + 1}`) };
}

/** The JSON document of a GLB from its first bytes; null while there aren't enough yet (or it isn't a GLB). */
export function glbJson(bytes: Uint8Array): unknown | null {
  if (bytes.length < 20) return null;
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (v.getUint32(0, true) !== 0x46546c67 || v.getUint32(16, true) !== 0x4e4f534a) return null;
  const len = v.getUint32(12, true);
  if (bytes.length < 20 + len) return null;
  return JSON.parse(new TextDecoder().decode(bytes.subarray(20, 20 + len)));
}

const ORDER = ['Stand', 'Walk', 'Run'];
/** The moves to offer, in one plain order: Stand, Walk, Run first, then the rest as the file lists them. */
export function orderMoves<T extends { name: string }>(clips: T[]): T[] {
  const rank = (c: T) => {
    const i = ORDER.indexOf(moveLabel(c.name));
    return i < 0 ? ORDER.length : i;
  };
  return clips.map((c, k) => ({ c, k })).sort((a, b) => rank(a.c) - rank(b.c) || a.k - b.k).map((x) => x.c);
}

/** "Skeleton added · 3 moves", "Skeleton added", or null without a skeleton. */
export function rigWords(r: GlbRig | null): string | null {
  if (!r?.skeleton) return null;
  const n = r.clips.length;
  return n ? `Skeleton added · ${n} ${n === 1 ? 'move' : 'moves'}` : 'Skeleton added';
}
