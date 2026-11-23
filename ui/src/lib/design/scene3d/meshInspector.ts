// Mesh topology inspection, sub-mesh hierarchy extraction, shading modes
// (Shaded / Wireframe / Matcap / Normals), wireframe overlay, and 1.8m scale
// reference helper. Pure stats/budget helpers stay testable without Three.js.
import type * as THREE_NS from 'three';
import type { Three } from './build';

/** Must match `USERDATA_ID` in `./build.ts` (kept inline so pure unit tests need zero Three runtime imports). */
const SCENE3D_USERDATA_ID = 'scene3dId';

export type ShadingMode = 'shaded' | 'wireframe' | 'matcap' | 'normals';

export interface SubMeshInfo {
  /** Stable key within the current Three.js scene graph (uuid). */
  uid: string;
  name: string;
  objectId: string | null;
  triangles: number;
  vertices: number;
  materialName: string;
  visible: boolean;
  depth: number;
}

export interface MeshTopologyStats {
  triangles: number;
  vertices: number;
  subMeshes: number;
  materials: number;
  bounds: [number, number, number] | null;
}

export const POLY_BUDGET_PRESETS = [
  { id: 'low_poly_prop', label: 'Low-Poly Prop', maxTris: 5_000 },
  { id: 'environment_asset', label: 'Environment Asset', maxTris: 15_000 },
  { id: 'stylized_character', label: 'Stylized Character', maxTris: 25_000 },
  { id: 'hero_mesh', label: 'Hero Mesh', maxTris: 60_000 },
] as const;

export function polyBudgetStatus(
  triangles: number,
  maxTris: number,
): { pct: number; tone: 'ok' | 'warn' | 'over'; label: string } {
  const safeMax = Math.max(1, maxTris);
  const pct = Math.round((triangles / safeMax) * 100);
  const tone = pct > 100 ? 'over' : pct >= 80 ? 'warn' : 'ok';
  return {
    pct,
    tone,
    label: `${triangles.toLocaleString()} / ${safeMax.toLocaleString()} tris (${pct}%)`,
  };
}

export function formatCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

/** Count triangles & vertices of a BufferGeometry. */
function geoCounts(geo: THREE_NS.BufferGeometry | undefined): { triangles: number; vertices: number } {
  if (!geo) return { triangles: 0, vertices: 0 };
  const pos = geo.getAttribute('position');
  const vertices = pos ? pos.count : 0;
  const idx = geo.getIndex();
  const triangles = idx ? Math.floor(idx.count / 3) : Math.floor(vertices / 3);
  return { triangles, vertices };
}

/** Walk `content` and collect every renderable Mesh + aggregate topology stats. */
export function inspectSceneTopology(
  THREE: Three,
  content: THREE_NS.Object3D,
): { meshes: SubMeshInfo[]; stats: MeshTopologyStats } {
  const meshes: SubMeshInfo[] = [];
  const matIds = new Set<string>();
  let totalTris = 0;
  let totalVerts = 0;

  const walk = (node: THREE_NS.Object3D, depth: number, ownerObjectId: string | null) => {
    const nextOwner = (node.userData?.[SCENE3D_USERDATA_ID] as string | undefined) ?? ownerObjectId;
    const m = node as THREE_NS.Mesh;
    if (m.isMesh && m.name !== 'placeholder' && !m.userData?.ottoWireOverlay) {
      const { triangles, vertices } = geoCounts(m.geometry);
      totalTris += triangles;
      totalVerts += vertices;
      const mats = Array.isArray(m.material) ? m.material : m.material ? [m.material] : [];
      for (const mat of mats) matIds.add(mat.uuid);
      const firstMat = mats[0];
      meshes.push({
        uid: m.uuid,
        name: m.name?.trim() || nextOwner || `Mesh ${meshes.length + 1}`,
        objectId: nextOwner,
        triangles,
        vertices,
        materialName: firstMat?.name?.trim() || firstMat?.type || 'Material',
        visible: m.visible,
        depth,
      });
    }
    for (const child of node.children) walk(child, depth + 1, nextOwner);
  };
  walk(content, 0, null);

  let bounds: [number, number, number] | null = null;
  if (meshes.length > 0) {
    const box = new THREE.Box3().setFromObject(content);
    if (!box.isEmpty()) {
      const s = box.getSize(new THREE.Vector3());
      bounds = [
        Math.round(s.x * 100) / 100,
        Math.round(s.y * 100) / 100,
        Math.round(s.z * 100) / 100,
      ];
    }
  }

  return {
    meshes,
    stats: {
      triangles: totalTris,
      vertices: totalVerts,
      subMeshes: meshes.length,
      materials: matIds.size,
      bounds,
    },
  };
}

/** Find a mesh inside `content` by its Three.js `uuid`. */
export function findMeshByUid(content: THREE_NS.Object3D, uid: string): THREE_NS.Mesh | null {
  let found: THREE_NS.Mesh | null = null;
  content.traverse((o) => {
    if (!found && o.uuid === uid && (o as THREE_NS.Mesh).isMesh) found = o as THREE_NS.Mesh;
  });
  return found;
}

export interface ShadingController {
  apply(content: THREE_NS.Object3D, mode: ShadingMode, wireframeOverlay: boolean): void;
  restore(content: THREE_NS.Object3D): void;
  dispose(): void;
}

/** Non-destructively switches shading modes (Shaded / Wireframe / Matcap / Normals)
 *  and adds/removes per-mesh wireframe overlays without corrupting original PBR materials. */
export function createShadingController(THREE: Three): ShadingController {
  const originals = new WeakMap<THREE_NS.Mesh, THREE_NS.Material | THREE_NS.Material[]>();
  const wireMat = new THREE.MeshBasicMaterial({ color: 0x94a3b8, wireframe: true });
  const matcapMat = new THREE.MeshMatcapMaterial({ color: 0xd8dce8 });
  const normalMat = new THREE.MeshNormalMaterial({ flatShading: false });
  const overlayMat = new THREE.MeshBasicMaterial({
    color: 0x38bdf8,
    wireframe: true,
    transparent: true,
    opacity: 0.32,
    depthTest: true,
    polygonOffset: true,
    polygonOffsetFactor: -1,
    polygonOffsetUnits: -1,
  });

  function removeOverlays(content: THREE_NS.Object3D): void {
    const toRemove: THREE_NS.Object3D[] = [];
    content.traverse((o) => {
      if (o.userData?.ottoWireOverlay) toRemove.push(o);
    });
    for (const o of toRemove) o.removeFromParent();
  }

  function restore(content: THREE_NS.Object3D): void {
    removeOverlays(content);
    content.traverse((o) => {
      const m = o as THREE_NS.Mesh;
      if (!m.isMesh || m.userData?.ottoWireOverlay) return;
      const orig = originals.get(m);
      if (orig) {
        m.material = orig;
        originals.delete(m);
      }
    });
  }

  function apply(content: THREE_NS.Object3D, mode: ShadingMode, wireframeOverlay: boolean): void {
    removeOverlays(content);
    const meshes: THREE_NS.Mesh[] = [];
    content.traverse((o) => {
      const m = o as THREE_NS.Mesh;
      if (m.isMesh && m.name !== 'placeholder' && !m.userData?.ottoWireOverlay) meshes.push(m);
    });
    for (const m of meshes) {
      if (!originals.has(m)) originals.set(m, m.material);
      const orig = originals.get(m)!;
      if (mode === 'shaded') m.material = orig;
      else if (mode === 'wireframe') m.material = wireMat;
      else if (mode === 'matcap') m.material = matcapMat;
      else if (mode === 'normals') m.material = normalMat;

      if (wireframeOverlay && mode !== 'wireframe' && m.geometry) {
        const wire = new THREE.Mesh(m.geometry, overlayMat);
        wire.name = '__wire_overlay';
        wire.userData.ottoWireOverlay = true;
        wire.renderOrder = 10;
        wire.raycast = () => {}; // never intercept polygon picking
        m.add(wire);
      }
    }
  }

  return {
    apply,
    restore,
    dispose() {
      wireMat.dispose();
      matcapMat.dispose();
      normalMat.dispose();
      overlayMat.dispose();
    },
  };
}

/** Build a lightweight 1.8m human-height + 1m³ prop scale reference wireframe cage. */
export function buildScaleReference(THREE: Three): THREE_NS.Group {
  const g = new THREE.Group();
  g.name = 'scale-reference';
  const cageGeo = new THREE.EdgesGeometry(new THREE.BoxGeometry(0.6, 1.8, 0.35));
  const cageMat = new THREE.LineBasicMaterial({ color: 0x64748b, transparent: true, opacity: 0.55 });
  const human = new THREE.LineSegments(cageGeo, cageMat);
  human.position.set(-1.3, 0.9, 0);
  const meterGeo = new THREE.EdgesGeometry(new THREE.BoxGeometry(1, 1, 1));
  const meterMat = new THREE.LineBasicMaterial({ color: 0x94a3b8, transparent: true, opacity: 0.4 });
  const cube = new THREE.LineSegments(meterGeo, meterMat);
  cube.position.set(1.3, 0.5, 0);
  g.add(human, cube);
  return g;
}
