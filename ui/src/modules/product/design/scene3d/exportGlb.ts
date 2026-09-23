// GLB export for a scene3d document (Export ▾ → GLB in the arena). Two entry
// points: `exportObjectToGlb` takes the live content root the viewport already
// built (fast, includes loaded gltf models), `exportSceneToGlb` builds a headless
// scene from the doc when no viewport is mounted. Helpers/gizmo/grid are never
// part of the content root, so nothing leaks into the export. The Blender
// script export is server-side (`design_scene3d.rs::to_blender_script`) — not here.
//
// v2 adds USDZ (three's USDZExporter — iOS Quick Look / AR) and passes the
// brand-colour resolver through so token colours export as their hex value.
import type * as THREE_NS from 'three';
import { buildLight, buildMesh, disposeTree, applyTransform, loadThree, plainColors, type ColorResolver } from './build';
import type { Scene3dDoc } from './types';

async function exporterFor(): Promise<new () => { parseAsync(input: THREE_NS.Object3D, options?: object): Promise<ArrayBuffer | object> }> {
  const mod = await import('three/examples/jsm/exporters/GLTFExporter.js');
  return mod.GLTFExporter as unknown as new () => {
    parseAsync(input: THREE_NS.Object3D, options?: object): Promise<ArrayBuffer | object>;
  };
}

/** Export an already-built three object tree to a GLB blob. */
export async function exportObjectToGlb(root: THREE_NS.Object3D): Promise<Blob> {
  const Exporter = await exporterFor();
  const result = await new Exporter().parseAsync(root, { binary: true, onlyVisible: true, trs: true });
  if (!(result instanceof ArrayBuffer)) {
    // `binary:true` always yields an ArrayBuffer; guard anyway so a JSON fallback still downloads.
    return new Blob([JSON.stringify(result)], { type: 'model/gltf+json' });
  }
  return new Blob([result], { type: 'model/gltf-binary' });
}

/**
 * Export an already-built object tree to USDZ (Apple Quick Look / AR). Only
 * meshes with standard/physical materials carry over; lights and cameras are
 * not part of USDZ's simple profile.
 */
export async function exportObjectToUsdz(root: THREE_NS.Object3D): Promise<Blob> {
  const { USDZExporter } = await import('three/examples/jsm/exporters/USDZExporter.js');
  const bytes = await new USDZExporter().parseAsync(root);
  return new Blob([bytes], { type: 'model/vnd.usdz+zip' });
}

/**
 * Build the doc headlessly and export it. `resolveAttachment` turns a gltf object's
 * `attachment_id` (or v2 `src` URI) into a blob URL; models that fail to load are
 * skipped (the export still succeeds) and listed in `skipped`.
 */
export async function exportSceneToGlb(
  doc: Scene3dDoc,
  resolveAttachment: (aid: string) => Promise<string>,
  color: ColorResolver = plainColors,
): Promise<{ blob: Blob; skipped: string[] }> {
  const THREE = await loadThree();
  const { GLTFLoader } = await import('three/examples/jsm/loaders/GLTFLoader.js');
  const root = new THREE.Group();
  root.name = 'scene3d';
  const skipped: string[] = [];
  const groupNodes = new Map<string, THREE_NS.Group>();
  for (const g of doc.groups) {
    const gn = new THREE.Group();
    gn.name = g.name;
    gn.visible = g.visible !== false;
    groupNodes.set(g.id, gn);
  }
  const parentOf = new Map<string, string>();
  for (const g of doc.groups) for (const c of g.children) parentOf.set(c, g.id);
  const attach = (id: string, obj: THREE_NS.Object3D) => {
    const pid = parentOf.get(id);
    (pid && groupNodes.get(pid) ? groupNodes.get(pid)! : root).add(obj);
  };
  for (const [gid, gn] of groupNodes) attach(gid, gn);

  const loader = new GLTFLoader();
  const { MeshoptDecoder } = await import('three/examples/jsm/libs/meshopt_decoder.module.js');
  loader.setMeshoptDecoder(MeshoptDecoder);
  for (const o of doc.objects) {
    if (o.type === 'gltf') {
      const ref = o.src ?? o.attachment_id;
      if (!ref) continue;
      try {
        const url = await resolveAttachment(ref);
        const gltf = await loader.loadAsync(url);
        const holder = new THREE.Group();
        holder.name = o.name;
        holder.add(gltf.scene);
        applyTransform(holder, o);
        holder.visible = o.visible !== false;
        attach(o.id, holder);
      } catch {
        skipped.push(o.name || o.id);
      }
      continue;
    }
    attach(o.id, buildMesh(THREE, o, color));
  }
  for (const l of doc.lights) {
    const { light, target } = buildLight(THREE, l);
    root.add(light);
    if (target) root.add(target);
  }
  try {
    const blob = await exportObjectToGlb(root);
    return { blob, skipped };
  } finally {
    disposeTree(root);
  }
}

/** Suggested download filename for a scene export. */
export function glbFileName(title: string | undefined, ext: 'glb' | 'usdz' | 'png' = 'glb'): string {
  const base = (title ?? 'scene').replace(/[^\w.-]+/g, '-').replace(/^-+|-+$/g, '') || 'scene';
  return `${base}.${ext}`;
}
