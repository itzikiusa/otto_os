// Otto School — the e2e / debug snapshot (SchoolHandle.debug): the view,
// frames drawn, missing kit nodes, every actor's clip / mode / pin, the
// headmaster, the camera, what each monitor shows and which one is at 2×.

import type * as THREE_NS from 'three';
import type { SchoolAssets } from '../assets.ts';
import type { Life } from '../life.ts';
import type { Room } from '../model.ts';
import { pinFor, type SchoolDebug, type SchoolView } from './api.ts';
import type { Painted } from './shared.ts';

export interface DebugInputs {
  view: SchoolView;
  frames: number;
  assets: SchoolAssets;
  life: Life;
  screens: Map<string, Painted>;
  openRoom: Room | null;
  cam: { pos: THREE_NS.Vector3 };
  /** Desk tag on the 2× canvas. */
  hiTag: string | null;
  /** Lamps the stage hung. */
  lamps: number;
}

export function schoolDebug(d: DebugInputs): SchoolDebug {
  const { view, frames, assets, life, screens, openRoom, cam, hiTag, lamps } = d;
  const hv = life.headView();
  const out: Record<string, string> = {};
  for (const [tag, s] of screens) out[tag] = s.key.split('\u0001').slice(4).join('\n');
  return {
    view,
    frames,
    missing: [...assets.missing],
    kids: life.kidViews().map((v) => {
      const k = openRoom ? [...openRoom.kids, ...openRoom.bench].find((x) => x.id === v.id) : undefined;
      return { id: v.id, clip: v.clip, mode: v.mode, visible: v.visible, x: v.x, z: v.z, pin: k ? pinFor(k.pose, k.detention) : null };
    }),
    head: hv ? { clip: hv.clip, mode: hv.mode, x: hv.x, z: hv.z, target: life.headTarget() } : null,
    camera: { x: cam.pos.x, y: cam.pos.y, z: cam.pos.z },
    screens: out,
    hiRes: hiTag,
    lamps: openRoom ? lamps : 0,
    ceiling: true,
  };
}
