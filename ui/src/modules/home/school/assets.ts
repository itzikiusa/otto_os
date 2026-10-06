// Otto School — loading the Blender-built glTF assets (ui/public/school/,
// built by ui/assets-src/school/blender/build_school.py; contract in
// ui/assets-src/school/CONTRACT.md). three.js and its loaders arrive through
// the caller's dynamic import, so nothing here weighs on the Home chunk.

import type * as THREE_NS from 'three';
import type { GLTF, GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import type { CharacterKey } from './model.ts';

type Three = typeof THREE_NS;

export const CHARACTERS: readonly CharacterKey[] = ['claude', 'codex', 'grok', 'agy', 'shell', 'custom'];

export interface SchoolAssets {
  /** Kit pieces by node name (templates — clone, never add directly). */
  kit: Map<string, THREE_NS.Object3D>;
  kids: Map<CharacterKey, GLTF>;
  head: GLTF;
  /** Kit nodes / files that failed to load (the scene skips them). */
  missing: string[];
  dispose(): void;
}

/** Where the files are served from (Vite's base + `school/`). */
export function assetBase(): string {
  const base = (import.meta.env?.BASE_URL as string | undefined) ?? '/';
  return `${base.endsWith('/') ? base : `${base}/`}school/`;
}

export async function loadSchoolAssets(T: Three, Loader: typeof GLTFLoader, base = assetBase()): Promise<SchoolAssets> {
  const loader = new Loader();
  const load = (f: string) => loader.loadAsync(`${base}${f}`);
  const missing: string[] = [];
  const [kitGltf, head, ...kidGltfs] = await Promise.all([
    load('school-kit.glb'),
    load('headmaster-otto.glb'),
    ...CHARACTERS.map((c) =>
      load(`kid-${c}.glb`).catch(() => {
        missing.push(`kid-${c}.glb`);
        return null;
      }),
    ),
  ]);
  const kids = new Map<CharacterKey, GLTF>();
  CHARACTERS.forEach((c, i) => {
    const g = kidGltfs[i];
    if (g) kids.set(c, g);
  });
  // Every provider falls back to the custom kid, then to any kid at all.
  const fallback = kids.get('custom') ?? [...kids.values()][0];
  if (!fallback) throw new Error('The school characters failed to load.');
  for (const c of CHARACTERS) if (!kids.has(c)) kids.set(c, fallback);

  const kit = new Map<string, THREE_NS.Object3D>();
  for (const child of kitGltf.scene.children) kit.set(child.name, child);
  kitGltf.scene.updateMatrixWorld(true);

  const all = [kitGltf.scene, head.scene, ...new Set([...kids.values()].map((g) => g.scene))];
  return {
    kit,
    kids,
    head,
    missing,
    dispose() {
      const seen = new Set<unknown>();
      for (const root of all)
        root.traverse((o) => {
          const m = o as THREE_NS.Mesh;
          if (m.geometry && !seen.has(m.geometry)) {
            seen.add(m.geometry);
            m.geometry.dispose();
          }
          const mats = Array.isArray(m.material) ? m.material : m.material ? [m.material] : [];
          for (const mat of mats) {
            if (seen.has(mat)) continue;
            seen.add(mat);
            for (const v of Object.values(mat)) if (v instanceof T.Texture && !seen.has(v)) (seen.add(v), v.dispose());
            mat.dispose();
          }
        });
    },
  };
}
