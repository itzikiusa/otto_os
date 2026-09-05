// doc → three.js builders shared by the viewport and the GLB exporter. `three` is
// lazy-loaded by the caller (it is ~650 kB) and passed in as the module namespace
// so this file stays free of a static import. Rotation in the doc is DEGREES —
// converted here, and only here, to radians.
import type * as THREE_NS from 'three';
import type { Scene3dLight, Scene3dMaterial, Scene3dObject, Vec3 } from './types';

export type Three = typeof THREE_NS;

export const DEG = Math.PI / 180;
export const RAD = 180 / Math.PI;

export const degToRad = (v: Vec3): Vec3 => [v[0] * DEG, v[1] * DEG, v[2] * DEG];
export const radToDeg = (v: Vec3): Vec3 => [v[0] * RAD, v[1] * RAD, v[2] * RAD];

/** Every object the viewer creates is tagged so picking / export can filter helpers out. */
export const USERDATA_ID = 'scene3dId';
export const USERDATA_KIND = 'scene3dKind';

export function applyTransform(target: THREE_NS.Object3D, o: Scene3dObject): void {
  target.position.set(o.position[0], o.position[1], o.position[2]);
  target.rotation.set(o.rotation[0] * DEG, o.rotation[1] * DEG, o.rotation[2] * DEG);
  // A zero scale component makes the matrix non-invertible (raycasts + gizmo break).
  target.scale.set(o.scale[0] || 1e-4, o.scale[1] || 1e-4, o.scale[2] || 1e-4);
}

export function makeGeometry(THREE: Three, type: Scene3dObject['type']): THREE_NS.BufferGeometry {
  switch (type) {
    case 'sphere':
      return new THREE.SphereGeometry(0.5, 32, 24);
    case 'cylinder':
      return new THREE.CylinderGeometry(0.5, 0.5, 1, 32);
    case 'cone':
      return new THREE.ConeGeometry(0.5, 1, 32);
    case 'torus':
      return new THREE.TorusGeometry(0.4, 0.12, 16, 48);
    case 'plane':
      return new THREE.PlaneGeometry(1, 1);
    case 'text':
      return new THREE.PlaneGeometry(2, 0.5);
    case 'box':
    default:
      return new THREE.BoxGeometry(1, 1, 1);
  }
}

export function makeMaterial(THREE: Three, m: Scene3dMaterial | undefined, opts: { doubleSided?: boolean } = {}): THREE_NS.MeshStandardMaterial {
  const mat = new THREE.MeshStandardMaterial({
    color: m?.color ?? '#94a3b8',
    metalness: m?.metalness ?? 0.1,
    roughness: m?.roughness ?? 0.7,
    emissive: m?.emissive ?? '#000000',
    wireframe: m?.wireframe ?? false,
  });
  if (m?.opacity !== undefined && m.opacity < 1) {
    mat.transparent = true;
    mat.opacity = m.opacity;
  }
  if (opts.doubleSided) mat.side = THREE.DoubleSide;
  return mat;
}

/** Text is drawn on a canvas texture onto a 2×0.5 m quad — no font files to fetch. */
export function makeTextTexture(THREE: Three, text: string, color: string): THREE_NS.CanvasTexture | null {
  if (typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas');
  canvas.width = 1024;
  canvas.height = 256;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = color;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  let size = 160;
  ctx.font = `600 ${size}px -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif`;
  while (size > 24 && ctx.measureText(text).width > canvas.width - 64) {
    size -= 8;
    ctx.font = `600 ${size}px -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif`;
  }
  ctx.fillText(text, canvas.width / 2, canvas.height / 2);
  const tex = new THREE.CanvasTexture(canvas);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = 4;
  return tex;
}

/** Build a primitive/text mesh (NOT gltf — the host loads those asynchronously). */
export function buildMesh(THREE: Three, o: Scene3dObject): THREE_NS.Mesh {
  const geom = makeGeometry(THREE, o.type);
  const mat = makeMaterial(THREE, o.material, { doubleSided: o.type === 'plane' || o.type === 'text' });
  if (o.type === 'text') {
    const tex = makeTextTexture(THREE, o.text ?? o.name, o.material?.color ?? '#e2e8f0');
    if (tex) {
      mat.map = tex;
      mat.color.set('#ffffff');
      mat.transparent = true;
      mat.alphaTest = 0.02;
    }
  }
  const mesh = new THREE.Mesh(geom, mat);
  mesh.name = o.name;
  mesh.castShadow = o.type !== 'plane';
  mesh.receiveShadow = true;
  mesh.userData[USERDATA_ID] = o.id;
  mesh.userData[USERDATA_KIND] = 'object';
  applyTransform(mesh, o);
  mesh.visible = o.visible !== false;
  return mesh;
}

/** Re-apply a material patch to an existing mesh in place (avoids a rebuild on slider drag). */
export function updateMeshMaterial(THREE: Three, mesh: THREE_NS.Mesh, o: Scene3dObject): void {
  const mat = mesh.material as THREE_NS.MeshStandardMaterial;
  if (!mat || !('metalness' in mat)) return;
  const m = o.material;
  if (o.type === 'text') {
    // Colour lives in the texture for text quads — redraw it.
    mat.map?.dispose();
    const tex = makeTextTexture(THREE, o.text ?? o.name, m?.color ?? '#e2e8f0');
    mat.map = tex;
    mat.color.set('#ffffff');
  } else {
    mat.color.set(m?.color ?? '#94a3b8');
  }
  mat.metalness = m?.metalness ?? 0.1;
  mat.roughness = m?.roughness ?? 0.7;
  mat.emissive.set(m?.emissive ?? '#000000');
  mat.wireframe = m?.wireframe ?? false;
  const op = m?.opacity ?? 1;
  mat.transparent = op < 1 || o.type === 'text';
  mat.opacity = op;
  mat.needsUpdate = true;
}

export interface BuiltLight {
  light: THREE_NS.Light;
  /** Directional/spot lights aim at a target object that must be in the scene too. */
  target?: THREE_NS.Object3D;
}

export function buildLight(THREE: Three, l: Scene3dLight): BuiltLight {
  const color = l.color ?? '#ffffff';
  const intensity = l.intensity ?? 1;
  let light: THREE_NS.Light;
  let target: THREE_NS.Object3D | undefined;
  switch (l.type) {
    case 'ambient':
      light = new THREE.AmbientLight(color, intensity);
      break;
    case 'hemisphere':
      light = new THREE.HemisphereLight(color, l.ground_color ?? '#334155', intensity);
      break;
    case 'point': {
      const p = new THREE.PointLight(color, intensity, l.distance ?? 0, 2);
      p.castShadow = l.shadow ?? false;
      light = p;
      break;
    }
    case 'spot': {
      const s = new THREE.SpotLight(color, intensity, l.distance ?? 0, (l.angle ?? 30) * DEG, 0.3, 2);
      s.castShadow = l.shadow ?? false;
      target = s.target;
      light = s;
      break;
    }
    case 'directional':
    default: {
      const d = new THREE.DirectionalLight(color, intensity);
      d.castShadow = l.shadow ?? false;
      d.shadow.mapSize.set(2048, 2048);
      d.shadow.camera.near = 0.5;
      d.shadow.camera.far = 80;
      d.shadow.camera.left = d.shadow.camera.bottom = -25;
      d.shadow.camera.right = d.shadow.camera.top = 25;
      d.shadow.bias = -0.0005;
      target = d.target;
      light = d;
      break;
    }
  }
  if (l.position) light.position.set(l.position[0], l.position[1], l.position[2]);
  if (target && l.target) target.position.set(l.target[0], l.target[1], l.target[2]);
  light.name = l.name ?? l.id;
  light.visible = l.visible !== false;
  light.userData[USERDATA_ID] = l.id;
  light.userData[USERDATA_KIND] = 'light';
  return { light, target };
}

/** Update a light of the SAME type in place (position/colour/intensity/shadow/target/visible). */
export function updateLight(THREE: Three, built: THREE_NS.Light, target: THREE_NS.Object3D | undefined, l: Scene3dLight): void {
  built.name = l.name ?? l.id;
  built.visible = l.visible !== false;
  built.color.set(l.color ?? '#ffffff');
  built.intensity = l.intensity ?? 1;
  if (l.position) built.position.set(l.position[0], l.position[1], l.position[2]);
  if (target && l.target) target.position.set(l.target[0], l.target[1], l.target[2]);
  const shadowable = built as THREE_NS.DirectionalLight | THREE_NS.PointLight | THREE_NS.SpotLight;
  if ('castShadow' in shadowable && l.type !== 'ambient' && l.type !== 'hemisphere') {
    shadowable.castShadow = l.shadow ?? false;
  }
  if (l.type === 'spot') {
    const s = built as THREE_NS.SpotLight;
    s.angle = (l.angle ?? 30) * DEG;
    s.distance = l.distance ?? 0;
  } else if (l.type === 'point') {
    (built as THREE_NS.PointLight).distance = l.distance ?? 0;
  } else if (l.type === 'hemisphere') {
    (built as THREE_NS.HemisphereLight).groundColor.set(l.ground_color ?? '#334155');
  }
  void THREE;
}

/** Dispose geometry + materials (+ textures) under `root`, recursively. */
export function disposeTree(root: THREE_NS.Object3D): void {
  root.traverse((obj) => {
    const mesh = obj as THREE_NS.Mesh;
    if (mesh.geometry) mesh.geometry.dispose();
    const mats = Array.isArray(mesh.material) ? mesh.material : mesh.material ? [mesh.material] : [];
    for (const m of mats) {
      const sm = m as THREE_NS.MeshStandardMaterial;
      sm.map?.dispose();
      sm.emissiveMap?.dispose();
      sm.normalMap?.dispose();
      sm.roughnessMap?.dispose();
      sm.metalnessMap?.dispose();
      m.dispose();
    }
  });
}
