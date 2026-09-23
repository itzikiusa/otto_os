// doc → three.js builders shared by the viewport and the GLB exporter. `three` is
// lazy-loaded by the caller (it is ~650 kB) and passed in as the module namespace
// so this file stays free of a static import. Rotation in the doc is DEGREES —
// converted here, and only here, to radians.
//
// v2: every mesh gets a MeshPhysicalMaterial built from `presets.ts::resolveMaterial`
// (preset defaults + explicit fields), colours go through a `ColorResolver` so
// `token:color.<name>` resolves against the scene's brand kit, and boxes with a
// `radius` use three's RoundedBoxGeometry (registered by the lazy loader).
import type * as THREE_NS from 'three';
import type { Scene3dLight, Scene3dMaterial, Scene3dObject, Vec3 } from './types';
import { resolveMaterial, type ResolvedMaterial } from './presets';
import { resolveColor } from './tokens';

export type Three = typeof THREE_NS;

/** Hex / `token:` colour → `#rrggbb` (the host binds the brand kit). */
export type ColorResolver = (ref: string | undefined, fallback: string) => string;
/** No brand kit: hex as-is, tokens fall back. */
export const plainColors: ColorResolver = (ref, fallback) => resolveColor(ref, null, fallback);

type RoundedBoxCtor = new (width?: number, height?: number, depth?: number, segments?: number, radius?: number) => THREE_NS.BufferGeometry;
let roundedBox: RoundedBoxCtor | null = null;
/** Register three's `RoundedBoxGeometry` (lazy-loaded alongside `three`). */
export function setRoundedBoxGeometry(ctor: RoundedBoxCtor): void {
  roundedBox = ctor;
}

/**
 * Lazy-load `three` plus the example modules every scene3d surface needs
 * (rounded boxes). One place, so the viewport, exporters and the embed
 * runtime share the chunk.
 */
export async function loadThree(): Promise<Three> {
  const [three, rb] = await Promise.all([import('three'), import('three/examples/jsm/geometries/RoundedBoxGeometry.js')]);
  setRoundedBoxGeometry(rb.RoundedBoxGeometry as unknown as RoundedBoxCtor);
  return three as unknown as Three;
}

/** Geometry identity for reconciliation: a rebuild is needed when this changes. */
export function geometryKey(o: Scene3dObject): string {
  if (o.type === 'gltf') return `gltf:${o.src ?? o.attachment_id}`;
  if (o.type === 'box' && o.radius) return `box:r${o.radius}`;
  return o.type;
}

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

export function makeGeometry(THREE: Three, type: Scene3dObject['type'], radius = 0): THREE_NS.BufferGeometry {
  if (type === 'box' && radius > 0 && roundedBox) {
    // Unit box; the radius is in unit-box space (scale stretches it with the box).
    return new roundedBox(1, 1, 1, 4, Math.min(0.5, radius));
  }
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

/** Copy resolved fields onto a physical material (create or in-place update). */
export function applyResolved(mat: THREE_NS.MeshPhysicalMaterial, r: ResolvedMaterial): void {
  mat.color.set(r.color);
  mat.metalness = r.metalness;
  mat.roughness = r.roughness;
  mat.emissive.set(r.emissive);
  mat.emissiveIntensity = r.emissiveIntensity;
  mat.wireframe = r.wireframe;
  mat.clearcoat = r.clearcoat;
  mat.clearcoatRoughness = r.clearcoatRoughness;
  mat.transmission = r.transmission;
  mat.ior = r.ior;
  mat.thickness = r.thickness;
  mat.sheen = r.sheen;
  mat.sheenRoughness = 0.6;
  mat.sheenColor.set(r.sheen > 0 ? r.color : '#000000');
  mat.opacity = r.opacity;
  mat.transparent = r.transparent;
}

export function makeMaterial(
  THREE: Three,
  m: Scene3dMaterial | undefined,
  opts: { doubleSided?: boolean; color?: ColorResolver } = {},
): THREE_NS.MeshPhysicalMaterial {
  const mat = new THREE.MeshPhysicalMaterial();
  applyResolved(mat, resolveMaterial(m, opts.color ?? plainColors));
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
export function buildMesh(THREE: Three, o: Scene3dObject, color: ColorResolver = plainColors): THREE_NS.Mesh {
  const geom = makeGeometry(THREE, o.type, o.radius ?? 0);
  const mat = makeMaterial(THREE, o.material, { doubleSided: o.type === 'plane' || o.type === 'text', color });
  if (o.type === 'text') {
    const tex = makeTextTexture(THREE, o.text ?? o.name, color(o.material?.color, '#e2e8f0'));
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
export function updateMeshMaterial(THREE: Three, mesh: THREE_NS.Mesh, o: Scene3dObject, color: ColorResolver = plainColors): void {
  const mat = mesh.material as THREE_NS.MeshPhysicalMaterial;
  if (!mat || !('clearcoat' in mat)) return;
  const r = resolveMaterial(o.material, color);
  applyResolved(mat, r);
  if (o.type === 'text') {
    // Colour lives in the texture for text quads — redraw it.
    mat.map?.dispose();
    mat.map = makeTextTexture(THREE, o.text ?? o.name, r.color);
    mat.color.set('#ffffff');
    mat.transparent = true;
  }
  // Transmission / transparency / clearcoat toggles change the shader program;
  // three re-uses the cached program when the key is unchanged.
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
