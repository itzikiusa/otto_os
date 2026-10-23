// Otto School — the camera rig: an eye-height walk along the corridor, an
// orbit inside the open room (clamped to its walls), eased flights between
// the two, and WASD / arrow-key input. The scene owns what it reads (the
// open room, the corridor length, the view) and is told when to redraw.

import type * as THREE_NS from 'three';
import { CORRIDOR_HALF, type Room } from '../model.ts';
import type { SchoolView } from './api.ts';
import { EYE, type Three } from './shared.ts';

export interface CameraHost {
  /** The open room (orbit bounds), if any. */
  room(): Room | null;
  /** The corridor's length, once a school is in. */
  corridorLength(): number | undefined;
  view(): SchoolView;
  /** The camera moved: draw the next frame. */
  touch(): void;
}

export function createCamera(T: Three, reduced: boolean, host: CameraHost) {
  const cam = {
    pos: new T.Vector3(0.7, EYE, CORRIDOR_HALF - 0.6),
    target: new T.Vector3(6, 1.4, -1.6),
    yaw: Math.PI / 2 + 0.62,
    pitch: -0.08,
    dist: 6,
    tween: null as null | { from: [THREE_NS.Vector3, THREE_NS.Vector3]; to: [THREE_NS.Vector3, THREE_NS.Vector3]; t: number; ms: number; done: () => void },
  };
  const keys = new Set<string>();

  function lookDir(yaw: number, pitch: number, out = new T.Vector3()): THREE_NS.Vector3 {
    return out.set(Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), Math.cos(yaw) * Math.cos(pitch));
  }

  function fly(pos: THREE_NS.Vector3, target: THREE_NS.Vector3, ms: number): Promise<void> {
    if (cam.tween) cam.tween.done();
    if (reduced || ms <= 0) {
      cam.pos.copy(pos);
      cam.target.copy(target);
      host.touch();
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      cam.tween = { from: [cam.pos.clone(), cam.target.clone()], to: [pos.clone(), target.clone()], t: 0, ms, done: resolve };
      // The render loop idles when nothing moves: a flight restarts it.
      host.touch();
    });
  }

  /** Standing by the lockers, looking along the corridor at the doors. */
  function corridorPose(x: number): [THREE_NS.Vector3, THREE_NS.Vector3] {
    const pos = new T.Vector3(x, EYE, CORRIDOR_HALF - 0.6);
    return [pos, pos.clone().add(lookDir(Math.PI / 2 + 0.62, -0.08))];
  }

  function roomPose(r: Room): [THREE_NS.Vector3, THREE_NS.Vector3] {
    const pos = new T.Vector3(r.cx + r.width / 2 - 0.9, 2.55, r.cz + r.depth / 2 - 0.9);
    const target = new T.Vector3(r.cx - r.width * 0.12, 0.75, r.cz - r.depth * 0.12);
    return [pos, target];
  }

  function syncOrbitFromCam(): void {
    const off = new T.Vector3().subVectors(cam.pos, cam.target);
    cam.dist = Math.max(0.5, off.length());
    cam.yaw = Math.atan2(off.x, off.z);
    cam.pitch = Math.asin(Math.max(-1, Math.min(1, off.y / cam.dist)));
  }

  function syncWalkFromCam(): void {
    const d = new T.Vector3().subVectors(cam.target, cam.pos).normalize();
    cam.yaw = Math.atan2(d.x, d.z);
    cam.pitch = Math.asin(Math.max(-1, Math.min(1, d.y)));
  }

  function clampRoom(p: THREE_NS.Vector3, r: Room, pad = 0.3): void {
    p.x = Math.min(r.cx + r.width / 2 - pad, Math.max(r.cx - r.width / 2 + pad, p.x));
    p.z = Math.min(r.cz + r.depth / 2 - pad, Math.max(r.cz - r.depth / 2 + pad, p.z));
  }

  function applyOrbit(): void {
    const r = host.room();
    if (!r) return;
    clampRoom(cam.target, r, 0.8);
    cam.target.y = Math.min(1.6, Math.max(0.3, cam.target.y));
    const p = cam.target.clone().add(lookDir(cam.yaw, cam.pitch).multiplyScalar(cam.dist));
    clampRoom(p, r);
    p.y = Math.min(2.85, Math.max(0.45, p.y));
    cam.pos.copy(p);
  }

  function applyWalk(): void {
    const len = host.corridorLength() ?? 18;
    cam.pos.x = Math.min(len - 0.5, Math.max(0.5, cam.pos.x));
    cam.pos.z = Math.min(CORRIDOR_HALF - 0.5, Math.max(-CORRIDOR_HALF + 0.5, cam.pos.z));
    cam.pos.y = EYE;
    cam.target.copy(cam.pos).add(lookDir(cam.yaw, cam.pitch));
  }

  function stepInput(dt: number): void {
    if (cam.tween || keys.size === 0 || host.view().kind === 'screen') return;
    const s = dt / 1000;
    const fwd = (keys.has('w') || keys.has('arrowup') ? 1 : 0) - (keys.has('s') || keys.has('arrowdown') ? 1 : 0);
    const side = (keys.has('d') ? 1 : 0) - (keys.has('a') ? 1 : 0);
    const turn = (keys.has('arrowright') ? 1 : 0) - (keys.has('arrowleft') ? 1 : 0);
    const zoom = (keys.has('-') ? 1 : 0) - (keys.has('=') || keys.has('+') ? 1 : 0);
    if (host.view().kind === 'corridor') {
      cam.yaw -= turn * 1.8 * s;
      const f = new T.Vector3(Math.sin(cam.yaw), 0, Math.cos(cam.yaw));
      const rgt = new T.Vector3(-f.z, 0, f.x);
      cam.pos.addScaledVector(f, fwd * 3.2 * s).addScaledVector(rgt, side * 2.6 * s);
      applyWalk();
    } else {
      cam.yaw += turn * 1.4 * s;
      const f = new T.Vector3(-Math.sin(cam.yaw), 0, -Math.cos(cam.yaw));
      const rgt = new T.Vector3(-f.z, 0, f.x);
      cam.target.addScaledVector(f, fwd * 2.6 * s).addScaledVector(rgt, side * 2.6 * s);
      cam.dist = Math.min(14, Math.max(1.2, cam.dist * (1 + zoom * 1.2 * s)));
      applyOrbit();
    }
    host.touch();
  }

  /** Mouse drag: look around the corridor / orbit the room. False while a
   *  flight runs or leaning over a monitor. */
  function drag(dx: number, dy: number): boolean {
    if (cam.tween || host.view().kind === 'screen') return false;
    if (host.view().kind === 'corridor') {
      cam.yaw -= dx * 0.0042;
      cam.pitch = Math.max(-0.6, Math.min(0.6, cam.pitch - dy * 0.0035));
      applyWalk();
    } else {
      cam.yaw -= dx * 0.005;
      cam.pitch = Math.max(0.05, Math.min(1.25, cam.pitch + dy * 0.004));
      applyOrbit();
    }
    host.touch();
    return true;
  }

  /** Wheel: walk the corridor / dolly the orbit. */
  function wheel(dy: number): boolean {
    if (cam.tween || host.view().kind === 'screen') return false;
    if (host.view().kind === 'corridor') {
      cam.pos.addScaledVector(new T.Vector3(Math.sin(cam.yaw), 0, Math.cos(cam.yaw)), -dy * 0.006);
      applyWalk();
    } else {
      cam.dist = Math.min(14, Math.max(1.2, cam.dist * (1 + dy * 0.0012)));
      applyOrbit();
    }
    host.touch();
    return true;
  }

  return { cam, keys, lookDir, fly, corridorPose, roomPose, syncOrbitFromCam, syncWalkFromCam, applyOrbit, applyWalk, stepInput, drag, wheel };
}
