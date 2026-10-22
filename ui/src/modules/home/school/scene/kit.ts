// Otto School — kit instancing: turns furnish.ts placements into
// InstancedMeshes (one per kit mesh), clones the live children the runtime
// paints or moves (screens, board title, door leaves / plates / signs), and
// frees a group's owned GPU objects. Also hands the scene its per-frame
// scratch math objects.

import type * as THREE_NS from 'three';
import type { SchoolAssets } from '../assets.ts';
import { KIT_NODES, type KitNode, type Placement } from '../furnish.ts';
import { SCREEN_H, SCREEN_W } from '../screens.ts';
import { LIVE_CHILDREN, NO_CAST, type MeshDesc, type Obj, type Painted, type Three } from './shared.ts';

/** Per-mount kit helpers (they cache each kit node's mesh descriptions). */
export function createKit(T: Three, assets: SchoolAssets) {
  const descCache = new Map<string, { meshes: MeshDesc[]; live: { name: string; obj: Obj; rel: THREE_NS.Matrix4 }[] }>();
  const warned = new Set<string>();
  function kitDesc(node: string) {
    const hit = descCache.get(node);
    if (hit) return hit;
    const root = assets.kit.get(node);
    if (!root) {
      if (!warned.has(node)) {
        warned.add(node);
        if (KIT_NODES.includes(node as KitNode)) assets.missing.push(node);
      }
      return null;
    }
    root.updateMatrixWorld(true);
    const inv = new T.Matrix4().copy(root.matrixWorld).invert();
    const meshes: MeshDesc[] = [];
    const live: { name: string; obj: Obj; rel: THREE_NS.Matrix4 }[] = [];
    for (const child of root.children) {
      if (LIVE_CHILDREN.has(child.name)) {
        live.push({ name: child.name, obj: child, rel: new T.Matrix4().multiplyMatrices(inv, child.matrixWorld) });
        continue;
      }
      child.traverse((o) => {
        const m = o as THREE_NS.Mesh;
        if (!m.isMesh || (m as unknown as THREE_NS.SkinnedMesh).isSkinnedMesh) return;
        meshes.push({ geometry: m.geometry, material: m.material, rel: new T.Matrix4().multiplyMatrices(inv, m.matrixWorld) });
      });
    }
    // The root itself may be the mesh.
    const rm = root as THREE_NS.Mesh;
    if (rm.isMesh) meshes.push({ geometry: rm.geometry, material: rm.material, rel: new T.Matrix4() });
    const d = { meshes, live };
    descCache.set(node, d);
    return d;
  }

  const tmpM = new T.Matrix4();
  const tmpR = new T.Matrix4();
  const tmpQ = new T.Quaternion();
  const tmpV = new T.Vector3();
  const one = new T.Vector3(1, 1, 1);
  const tmpS = new T.Vector3();
  const up = new T.Vector3(0, 1, 0);
  function placeMatrix(p: Placement, out = new T.Matrix4()): THREE_NS.Matrix4 {
    tmpQ.setFromAxisAngle(up, p.rotY);
    return out.compose(tmpV.set(p.x, p.y, p.z), tmpQ, one);
  }

  /** Instanced statics for `placements` into `group`. Returns, per tag, the
   *  (instancedMesh, index, rel) triples so the caller can move one instance
   *  (a chair sliding back). */
  function instance(group: THREE_NS.Group, placements: Placement[], shadows: boolean) {
    const byNode = new Map<string, Placement[]>();
    for (const p of placements) {
      const l = byNode.get(p.node);
      if (l) l.push(p);
      else byNode.set(p.node, [p]);
    }
    const handles = new Map<string, { mesh: THREE_NS.InstancedMesh; index: number; rel: THREE_NS.Matrix4; base: THREE_NS.Matrix4 }[]>();
    for (const [node, list] of byNode) {
      const d = kitDesc(node);
      if (!d) continue;
      for (const md of d.meshes) {
        const im = new T.InstancedMesh(md.geometry, md.material, list.length);
        im.castShadow = shadows && !NO_CAST.has(node);
        im.receiveShadow = shadows;
        im.frustumCulled = false;
        list.forEach((p, i) => {
          const base = placeMatrix(p);
          im.setMatrixAt(i, tmpM.multiplyMatrices(base, md.rel));
          if (p.tag && node === 'Chair') {
            const h = handles.get(p.tag) ?? [];
            h.push({ mesh: im, index: i, rel: md.rel, base });
            handles.set(p.tag, h);
          }
        });
        im.instanceMatrix.needsUpdate = true;
        im.userData.owned = true;
        group.add(im);
      }
    }
    return handles;
  }

  function paintable(src: Obj, w: number, h: number): Painted | null {
    const m = src as THREE_NS.Mesh;
    if (!m.isMesh) return null;
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    const ctx = c.getContext('2d');
    if (!ctx) return null;
    const tex = new T.CanvasTexture(c);
    tex.colorSpace = T.SRGBColorSpace;
    tex.anisotropy = 4;
    const mat = new T.MeshBasicMaterial({ map: tex, toneMapped: false });
    const mesh = new T.Mesh(m.geometry, mat);
    mesh.userData.owned = true;
    return { mesh, canvas: c, ctx, tex, key: '' };
  }

  /** The live children of one placement (screen, board title, door bits). */
  function liveChildren(group: THREE_NS.Group, p: Placement) {
    const d = kitDesc(p.node);
    const out: Record<string, { painted?: Painted; pivot?: { obj: Obj; base: THREE_NS.Matrix4 } }> = {};
    if (!d) return out;
    const base = placeMatrix(p);
    for (const l of d.live) {
      const world = new T.Matrix4().multiplyMatrices(base, l.rel);
      if (l.name === 'DoorLeaf') {
        const obj = l.obj.clone(true);
        obj.matrixAutoUpdate = false;
        obj.matrix.copy(world);
        obj.traverse((o) => {
          o.castShadow = true;
          o.receiveShadow = true;
        });
        group.add(obj);
        out.DoorLeaf = { pivot: { obj, base: world } };
        continue;
      }
      const size = l.name === 'Screen' ? [SCREEN_W, SCREEN_H] : l.name === 'BoardTitle' ? [768, 150] : l.name === 'NamePlate' ? [640, 160] : [320, 224];
      const painted = paintable(l.obj, size[0], size[1]);
      if (!painted) continue;
      painted.mesh.matrixAutoUpdate = false;
      painted.mesh.matrix.copy(world);
      painted.mesh.matrixWorldNeedsUpdate = true;
      group.add(painted.mesh);
      out[l.name] = { painted };
    }
    return out;
  }

  function clearGroup(g: THREE_NS.Group): void {
    for (const o of [...g.children]) {
      o.traverse((x) => {
        if (!x.userData.owned) return;
        if ((x as THREE_NS.InstancedMesh).isInstancedMesh) (x as THREE_NS.InstancedMesh).dispose();
        const m = x as THREE_NS.Mesh;
        const mat = m.material as THREE_NS.MeshBasicMaterial | undefined;
        if (mat && !Array.isArray(mat) && mat.map instanceof T.CanvasTexture) {
          mat.map.dispose();
          mat.dispose();
        }
      });
      g.remove(o);
    }
  }

  return { tmpM, tmpR, tmpQ, tmpV, one, tmpS, up, kitDesc, placeMatrix, instance, paintable, liveChildren, clearGroup };
}

/** The one 2× monitor canvas (the focused kid's screen borrows it). */
export function hiResCanvas(T: Three) {
  const c = document.createElement('canvas');
  c.width = SCREEN_W * 2;
  c.height = SCREEN_H * 2;
  const ctx = c.getContext('2d')!;
  const tex = new T.CanvasTexture(c);
  tex.colorSpace = T.SRGBColorSpace;
  tex.anisotropy = 8;
  return { canvas: c, ctx, tex, tag: null as string | null };
}
