// Otto School — the markers drawn over the actors: the selection / hover
// floor rings, a glowing status pin over every kid with something to say and
// the pulsing amber "needs you" ring under it (two InstancedMeshes,
// re-placed each frame with no allocation).

import type * as THREE_NS from 'three';
import type { SchoolAssets } from '../assets.ts';
import type { Life } from '../life.ts';
import type { Room } from '../model.ts';
import { PIN_COLORS, pinFor } from './api.ts';
import { SEATS, type Character, type Three } from './shared.ts';

export function createMarkers(T: Three, scene: THREE_NS.Scene, assets: SchoolAssets, characters: Map<string, Character>, life: Life, reduced: boolean) {
  const ringGeo = new T.RingGeometry(0.34, 0.44, 40);
  ringGeo.rotateX(-Math.PI / 2);
  const ringMat = new T.MeshBasicMaterial({ color: 0x4f8cff, transparent: true, opacity: 0.9, depthWrite: false });
  const hoverMat = new T.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.55, depthWrite: false });
  const selRing = new T.Mesh(ringGeo, ringMat);
  const hoverRing = new T.Mesh(ringGeo, hoverMat);
  selRing.visible = hoverRing.visible = false;
  selRing.renderOrder = hoverRing.renderOrder = 2;
  scene.add(selRing, hoverRing);
  const tmpM = new T.Matrix4();
  const tmpQ = new T.Quaternion();
  const tmpV = new T.Vector3();
  const tmpS = new T.Vector3();

  // Status pins (one glowing bead over each kid that has something to say)
  // and the amber "needs you" floor rings: two InstancedMeshes, re-placed
  // each frame from the actors' positions with no allocation.
  const pinGeo = new T.SphereGeometry(0.06, 14, 10);
  const pinMat = new T.MeshBasicMaterial({ toneMapped: false });
  const pins = new T.InstancedMesh(pinGeo, pinMat, SEATS);
  pins.instanceColor = new T.InstancedBufferAttribute(new Float32Array(SEATS * 3), 3);
  pins.count = 0;
  pins.frustumCulled = false;
  pins.renderOrder = 3;
  const callGeo = new T.RingGeometry(0.46, 0.62, 48);
  callGeo.rotateX(-Math.PI / 2);
  const callMat = new T.MeshBasicMaterial({ color: 0xffb020, transparent: true, opacity: 0.75, depthWrite: false, toneMapped: false });
  const calls = new T.InstancedMesh(callGeo, callMat, SEATS);
  calls.count = 0;
  calls.frustumCulled = false;
  calls.renderOrder = 2;
  scene.add(pins, calls);
  const pinColor = new T.Color();
  /** Kid model height (standing) — pins float just over the head. */
  let kidHeight = 1.3;
  {
    const any = [...assets.kids.values()][0];
    if (any) {
      const h = new T.Box3().setFromObject(any.scene).getSize(new T.Vector3()).y;
      if (Number.isFinite(h) && h > 0.6 && h < 2.5) kidHeight = h;
    }
  }

  /** Re-place the status pins + needs-you rings over the actors. `t` (ms)
   *  drives the bob / pulse; reduced motion holds them still. */
  function updatePins(r: Room, t: number): void {
    let n = 0;
    let m = 0;
    const bob = reduced ? 0 : Math.sin(t / 260) * 0.035;
    const pulse = reduced ? 1 : 1 + Math.sin(t / 220) * 0.08;
    for (const k of r.kids) {
      const c = characters.get(k.id);
      const v = life.kidView(k.id);
      if (!c || !v || !v.visible || n >= SEATS) continue;
      const kind = pinFor(k.pose, k.detention);
      if (!kind) continue;
      // Clear of the head (seated ≈ ¾ height), and of a raised hand.
      const y = v.seated ? kidHeight * 0.74 + 0.45 : kidHeight + 0.32;
      const s = kind === 'needs-you' ? 1.25 * pulse : 1;
      tmpQ.identity();
      tmpM.compose(tmpV.set(c.obj.position.x, y + (kind === 'needs-you' ? bob : 0), c.obj.position.z), tmpQ, tmpS.set(s, s, s));
      pins.setMatrixAt(n, tmpM);
      pins.setColorAt(n, pinColor.setHex(PIN_COLORS[kind]));
      n++;
      if (kind === 'needs-you' && m < SEATS) {
        tmpM.compose(tmpV.set(c.obj.position.x, 0.015, c.obj.position.z), tmpQ, tmpS.set(pulse, 1, pulse));
        calls.setMatrixAt(m++, tmpM);
      }
    }
    pins.count = n;
    calls.count = m;
    callMat.opacity = reduced ? 0.75 : 0.55 + Math.sin(t / 220) * 0.25;
    pins.instanceMatrix.needsUpdate = true;
    if (pins.instanceColor) pins.instanceColor.needsUpdate = true;
    calls.instanceMatrix.needsUpdate = true;
  }

  function placeRing(ring: THREE_NS.Mesh, id: string | null): void {
    const c = id ? characters.get(id) : null;
    ring.visible = !!c && c.obj.visible;
    if (c) ring.position.set(c.obj.position.x, 0.02, c.obj.position.z);
  }

  return { selRing, hoverRing, ringGeo, ringMat, hoverMat, pinGeo, pinMat, pins, callGeo, callMat, calls, updatePins, placeRing };
}
