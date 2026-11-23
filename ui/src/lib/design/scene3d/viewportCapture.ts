// Extracted from Scene3DViewport.svelte: PNG snapshot, 8-frame turntable
// sprite-sheet renderer, and GLB/glTF/OBJ model loader.
import type * as THREE_NS from 'three';
import type { OrbitControls as OrbitControlsT } from 'three/examples/jsm/controls/OrbitControls.js';
import type { GLTFLoader as GLTFLoaderT } from 'three/examples/jsm/loaders/GLTFLoader.js';
import type { Three } from './build';
import { dropGameHelpers } from './gameHelpers';

/** SkeletonUtils.clone, loaded with the first model: a plain clone(true) leaves a skinned
 *  mesh bound to the template's bones, so it draws at the origin whatever its object's place. */
let skeletonClone: ((o: THREE_NS.Object3D) => THREE_NS.Object3D) | null = null;

/** A copy of a loaded model for one scene object (skinned meshes keep their own bones). */
export function cloneModel(template: THREE_NS.Object3D): THREE_NS.Object3D {
  return skeletonClone ? skeletonClone(template) : template.clone(true);
}

/** Load a GLB/glTF (or OBJ when text starts with OBJ directives) into a Three Group with shadows enabled. */
export async function loadModelAsset(
  THREE: Three,
  loader: GLTFLoaderT,
  url: string,
): Promise<THREE_NS.Group> {
  try {
    const gltf = await loader.loadAsync(url);
    skeletonClone ??= (await import('three/examples/jsm/utils/SkeletonUtils.js')).clone;
    dropGameHelpers(gltf.scene);
    gltf.scene.traverse((n) => {
      const m = n as THREE_NS.Mesh;
      if (m.isMesh) {
        m.castShadow = true;
        m.receiveShadow = true;
      }
    });
    if (gltf.animations?.length) {
      gltf.scene.userData.ottoAnimations = gltf.animations;
    }
    return gltf.scene;
  } catch (gltfErr) {
    // Fallback for Wavefront OBJ text blobs (e.g. Hunyuan3D-Omni `.obj` exports).
    const resp = await fetch(url);
    const text = await resp.text();
    if (!/(?:^|\n)(?:v|vn|vt|f|o|g)\s/.test(text)) throw gltfErr;
    const { OBJLoader } = await import('three/examples/jsm/loaders/OBJLoader.js');
    const obj = new OBJLoader().parse(text);
    const defaultMat = new THREE.MeshStandardMaterial({ color: 0xcbd5e1, roughness: 0.6, metalness: 0.1 });
    obj.traverse((n) => {
      const m = n as THREE_NS.Mesh;
      if (m.isMesh) {
        if (!m.material) m.material = defaultMat;
        m.castShadow = true;
        m.receiveShadow = true;
      }
    });
    return obj;
  }
}

export async function renderSnapshotPng(
  renderer: THREE_NS.WebGLRenderer,
  scene: THREE_NS.Scene,
  camera: THREE_NS.PerspectiveCamera,
): Promise<Blob | null> {
  renderer.render(scene, camera);
  return new Promise((res) => renderer.domElement.toBlob((b) => res(b), 'image/png'));
}

export async function renderTurntableSheet(
  THREE: Three,
  renderer: THREE_NS.WebGLRenderer,
  scene: THREE_NS.Scene,
  camera: THREE_NS.PerspectiveCamera,
  orbit: OrbitControlsT,
  helpers: THREE_NS.Group,
  gizmoHelper: THREE_NS.Object3D | null,
  invalidate: () => void,
  frames = 8,
  cell = 480,
): Promise<Blob | null> {
  const cols = Math.min(4, frames);
  const rows = Math.ceil(frames / cols);
  const sheet = document.createElement('canvas');
  const src = renderer.domElement;
  const aspect = src.width / Math.max(1, src.height);
  const cw = cell;
  const ch = Math.round(cell / aspect);
  sheet.width = cw * cols;
  sheet.height = ch * rows;
  const ctx = sheet.getContext('2d');
  if (!ctx) return null;
  const pos = camera.position.clone();
  const helpersWere = helpers.visible;
  const gizmoWas = gizmoHelper?.visible ?? false;
  helpers.visible = false;
  if (gizmoHelper) gizmoHelper.visible = false;
  const offset = pos.clone().sub(orbit.target);
  try {
    for (let i = 0; i < frames; i++) {
      const o = offset.clone().applyAxisAngle(new THREE.Vector3(0, 1, 0), (i / frames) * Math.PI * 2);
      camera.position.copy(orbit.target).add(o);
      camera.lookAt(orbit.target);
      renderer.render(scene, camera);
      ctx.drawImage(src, (i % cols) * cw, Math.floor(i / cols) * ch, cw, ch);
    }
  } finally {
    camera.position.copy(pos);
    camera.lookAt(orbit.target);
    helpers.visible = helpersWere;
    if (gizmoHelper) gizmoHelper.visible = gizmoWas;
    invalidate();
  }
  return new Promise((res) => sheet.toBlob((b) => res(b), 'image/png'));
}
