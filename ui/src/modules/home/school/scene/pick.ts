// Otto School — picking and projection: what's under the pointer (a door in
// the corridor; a kid, its monitor, the headmaster or the way out in a
// room), where an actor or door lands on screen, which monitors the camera
// can see, and the door the corridor camera faces.

import type * as THREE_NS from 'three';
import { CORRIDOR_HALF, toWorld, type Room, type School } from '../model.ts';
import type { SchoolPick, SchoolView, ScreenPoint } from './api.ts';
import type { createCamera } from './camera.ts';
import type { Character, Door, Obj, Painted, Three } from './shared.ts';

export interface PickHost {
  view(): SchoolView;
  school(): School | null;
  /** The open room, if any. */
  room(): Room | null;
  head(): Character | null;
}

export interface PickScene {
  canvas: HTMLCanvasElement;
  camera: THREE_NS.PerspectiveCamera;
  roomGroup: THREE_NS.Group;
  doors: Map<string, Door>;
  characters: Map<string, Character>;
  screens: Map<string, Painted>;
  rig: Pick<ReturnType<typeof createCamera>, 'cam' | 'lookDir'>;
}

export function createPicker(T: Three, s: PickScene, host: PickHost) {
  const { canvas, camera, roomGroup, doors, characters, screens } = s;
  const { cam, lookDir } = s.rig;

  const ray = new T.Raycaster();
  const ndc = new T.Vector2();
  function pick(cx: number, cy: number): SchoolPick | null {
    const view = host.view();
    const openRoom = host.room();
    const head = host.head();
    const rect = canvas.getBoundingClientRect();
    if (!rect.width || !rect.height) return null;
    ndc.set(((cx - rect.left) / rect.width) * 2 - 1, -((cy - rect.top) / rect.height) * 2 + 1);
    ray.setFromCamera(ndc, camera);
    const targets: Obj[] = [];
    if (view.kind === 'corridor') {
      for (const d of doors.values()) if (d.hit) targets.push(d.hit);
    } else {
      for (const c of characters.values()) if (c.obj.visible) targets.push(c.hit);
      if (head) targets.push(head.hit);
      for (const s of screens.values()) targets.push(s.mesh);
      roomGroup.traverse((o) => {
        if ((o.userData.pick as SchoolPick | undefined)?.kind === 'door') targets.push(o);
      });
    }
    // Data updates can rebuild hit meshes between rendered frames. Raycaster
    // reads matrixWorld without refreshing it, so sync only the pick targets
    // (and their parents), rather than waiting for the next GPU render.
    for (const target of targets) target.updateWorldMatrix(true, false);
    const hits = ray.intersectObjects(targets, false);
    // The headmaster is big: a kid (or its monitor) behind him still wins.
    const hit = hits.find((h) => (h.object.userData.pick as SchoolPick | undefined)?.kind !== 'head') ?? hits[0];
    if (!hit) return null;
    const desk = hit.object.userData.desk as string | undefined;
    if (desk) {
      const k = openRoom?.kids.find((x) => `${x.row}:${x.col}` === desk);
      return k ? { kind: 'kid', id: k.id } : null;
    }
    return (hit.object.userData.pick as SchoolPick | undefined) ?? null;
  }

  function project(p: SchoolPick): ScreenPoint | null {
    const school = host.school();
    const head = host.head();
    let w: THREE_NS.Vector3 | null = null;
    if (p.kind === 'kid') {
      const c = characters.get(p.id);
      if (c && c.obj.visible) w = c.obj.position.clone().setY(1.3);
    } else if (p.kind === 'head') {
      if (head) w = head.obj.position.clone().setY(1.95);
    } else {
      const r = school?.rooms.find((x) => x.id === p.id);
      if (r) w = new T.Vector3(r.doorX, 2.75, -CORRIDOR_HALF + 0.05);
    }
    if (!w) return null;
    const rect = canvas.getBoundingClientRect();
    const v = w.project(camera);
    const x = rect.left + ((v.x + 1) / 2) * rect.width;
    const y = rect.top + ((1 - v.y) / 2) * rect.height;
    const visible = v.z < 1 && v.z > -1 && x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
    return { x, y, visible };
  }

  function visibleScreens(): string[] {
    const view = host.view();
    const r = host.room();
    if (!r) return [];
    const frustum = new T.Frustum().setFromProjectionMatrix(new T.Matrix4().multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse));
    const out: { id: string; d: number }[] = [];
    for (const k of r.kids) {
      const w = toWorld(r, k.seat);
      const pt = new T.Vector3(w.x, 1, w.z - 0.55);
      if (view.kind === 'screen' && view.kidId === k.id) out.push({ id: k.id, d: -1 });
      else if (frustum.containsPoint(pt)) out.push({ id: k.id, d: pt.distanceTo(cam.pos) });
    }
    return out.sort((a, b) => a.d - b.d).map((x) => x.id);
  }

  function facingDoor(): string | null {
    const view = host.view();
    const school = host.school();
    if (view.kind !== 'corridor' || !school) return null;
    const f = lookDir(cam.yaw, 0);
    let best: { id: string; s: number } | null = null;
    for (const r of school.rooms) {
      const to = new T.Vector3(r.doorX - cam.pos.x, 0, -CORRIDOR_HALF - cam.pos.z);
      const d = to.length();
      const s = to.normalize().dot(f);
      if (d < 6 && s > 0.3 && (!best || s - d * 0.05 > best.s)) best = { id: r.id, s: s - d * 0.05 };
    }
    return best?.id ?? null;
  }

  return { pick, project, visibleScreens, facingDoor };
}
