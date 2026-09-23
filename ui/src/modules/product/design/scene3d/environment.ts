// Procedural image-based lighting for scene3d v2 environments ("Studio soft",
// "Sunset", "Night"). Nothing is fetched and no HDRI ships: each preset is a
// tiny three scene — a gradient sky sphere plus a few glowing softbox panels
// (data in `presets.ts`) — prefiltered once with PMREMGenerator, the same
// technique as three's RoomEnvironment. `three` is passed in (lazy-loaded by
// the caller) so this module stays free of a static import.
import type * as THREE_NS from 'three';
import type { Three } from './build';
import { envPreset, type EnvPresetDef } from './presets';
import type { Scene3dEnvironment } from './types';

export interface BuiltEnvironment {
  /** Prefiltered environment map for `scene.environment` (null for `none`). */
  envMap: THREE_NS.Texture | null;
  /** A vertical-gradient backdrop for `scene.background`. */
  backdrop: THREE_NS.Texture | null;
  preset: EnvPresetDef;
  dispose(): void;
}

/** Build the sky + softbox scene for a preset, rotated `rotationDeg` about Y. */
function envScene(THREE: Three, preset: EnvPresetDef, rotationDeg: number): THREE_NS.Scene {
  const scene = new THREE.Scene();
  const root = new THREE.Group();
  root.rotation.y = (rotationDeg * Math.PI) / 180;
  scene.add(root);

  // Sky sphere: zenith → horizon → ground gradient in vertex colours.
  const geo = new THREE.SphereGeometry(10, 48, 24);
  const [top, mid, bottom] = preset.sky.map((c) => new THREE.Color(c));
  const pos = geo.getAttribute('position');
  const colors = new Float32Array(pos.count * 3);
  const c = new THREE.Color();
  for (let i = 0; i < pos.count; i++) {
    const y = pos.getY(i) / 10; // -1..1
    if (y >= 0) c.copy(mid).lerp(top, Math.pow(y, 0.6));
    else c.copy(mid).lerp(bottom, Math.pow(-y, 0.5));
    colors[i * 3] = c.r;
    colors[i * 3 + 1] = c.g;
    colors[i * 3 + 2] = c.b;
  }
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
  const sky = new THREE.Mesh(geo, new THREE.MeshBasicMaterial({ vertexColors: true, side: THREE.BackSide }));
  root.add(sky);

  // Softboxes: emissive-looking quads facing the origin (brightness > 1 → strong highlights).
  for (const p of preset.panels) {
    const d = new THREE.Vector3(...p.dir).normalize();
    const mat = new THREE.MeshBasicMaterial({ color: new THREE.Color(p.color).multiplyScalar(p.intensity), side: THREE.DoubleSide });
    const quad = new THREE.Mesh(new THREE.PlaneGeometry(p.size[0], p.size[1]), mat);
    quad.position.copy(d.multiplyScalar(9));
    quad.lookAt(0, 0, 0);
    root.add(quad);
  }
  return scene;
}

/** A 2×256 vertical gradient texture (top → bottom) used as a clean backdrop. */
function backdropTexture(THREE: Three, colors: [string, string]): THREE_NS.Texture | null {
  if (typeof document === 'undefined') return null;
  const canvas = document.createElement('canvas');
  canvas.width = 2;
  canvas.height = 256;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  const g = ctx.createLinearGradient(0, 0, 0, 256);
  g.addColorStop(0, colors[0]);
  g.addColorStop(1, colors[1]);
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, 2, 256);
  const tex = new THREE.CanvasTexture(canvas);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

/**
 * Build the environment for a document's `environment` block. The PMREM
 * render uses the caller's renderer; dispose the result when the preset or
 * rotation changes (or the viewer unmounts).
 */
export function buildEnvironment(THREE: Three, renderer: THREE_NS.WebGLRenderer, env: Scene3dEnvironment): BuiltEnvironment {
  const preset = envPreset(env.preset) ?? envPreset('studio-soft')!;
  if (preset.id === 'none') {
    return { envMap: null, backdrop: null, preset, dispose: () => {} };
  }
  const pmrem = new THREE.PMREMGenerator(renderer);
  const scene = envScene(THREE, preset, env.rotation ?? 0);
  const rt = pmrem.fromScene(scene, 0.035);
  pmrem.dispose();
  scene.traverse((o) => {
    const m = o as THREE_NS.Mesh;
    m.geometry?.dispose();
    (m.material as THREE_NS.Material | undefined)?.dispose();
  });
  const backdrop = backdropTexture(THREE, preset.backdrop);
  return {
    envMap: rt.texture,
    backdrop,
    preset,
    dispose: () => {
      rt.dispose();
      backdrop?.dispose();
    },
  };
}

/** Stable key: rebuild only when the preset or its rotation changes. */
export function environmentKey(env: Scene3dEnvironment | undefined): string {
  return env ? `${env.preset}:${env.rotation ?? 0}` : 'none';
}
