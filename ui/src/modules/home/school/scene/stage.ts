// Otto School — the stage: the WebGL renderer (software rasterisers drop
// shadows and half the lamps), the image-based environment, the window sun,
// its fill and the warm ceiling lamps, the ceiling planes and the kit
// materials the theme retints. `applyTheme` / `aimRoomLights` set them up
// for the time of day and the open room.

import type * as THREE_NS from 'three';
import type { SchoolAssets } from '../assets.ts';
import { CEIL_Y, ceilingLights } from '../furnish.ts';
import { toWorld, type Room } from '../model.ts';
import type { MountOpts } from './api.ts';
import { LAMPS, type Three } from './shared.ts';

export type Stage = ReturnType<typeof createStage>;

export function createStage(
  T: Three,
  assets: SchoolAssets,
  RoomEnvironment: new () => THREE_NS.Scene,
  hdr: THREE_NS.DataTexture | null,
  host: HTMLElement,
  opts: MountOpts,
) {
  const renderer = new T.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' });
  const displayScale = Number.isFinite(window.devicePixelRatio) && window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
  renderer.setPixelRatio(Math.min(2, displayScale));
  renderer.outputColorSpace = T.SRGBColorSpace;
  renderer.toneMapping = T.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.05;
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = T.PCFShadowMap;
  // A software rasteriser (SwiftShader / llvmpipe — CI, VMs, no GPU) can't
  // afford soft shadows at 2× pixels: drop both so the room stays live (and
  // light the room with half the lamps).
  let software = false;
  try {
    const gl = renderer.getContext();
    const info = gl.getExtension('WEBGL_debug_renderer_info');
    const name = String(info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER));
    if (/swiftshader|llvmpipe|softpipe|software/i.test(name)) {
      software = true;
      renderer.shadowMap.enabled = false;
      renderer.setPixelRatio(Math.min(1, displayScale));
    }
  } catch {
    /* keep the defaults */
  }
  const canvas = renderer.domElement;
  canvas.setAttribute('role', 'img');
  canvas.setAttribute('aria-label', opts.label);
  canvas.style.display = 'block';
  canvas.style.width = '100%';
  canvas.style.height = '100%';
  canvas.style.touchAction = 'none';
  host.appendChild(canvas);

  const scene = new T.Scene();
  const pmrem = new T.PMREMGenerator(renderer);
  if (hdr) hdr.mapping = T.EquirectangularReflectionMapping;
  const envRT = hdr ? pmrem.fromEquirectangular(hdr) : pmrem.fromScene(new RoomEnvironment(), 0.04);
  hdr?.dispose();
  scene.environment = envRT.texture;
  scene.environmentIntensity = hdr ? 0.8 : 0.55;

  const camera = new T.PerspectiveCamera(55, 1, 0.05, 200);
  const hemi = new T.HemisphereLight(0xdfefff, 0x8a7a66, 1.1);
  // The sun comes in low through the window wall (west), so desks throw long
  // soft shadows across the floor; the fill is the bounce off the far wall.
  const sun = new T.DirectionalLight(0xfff1dc, 2.4);
  sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048);
  sun.shadow.bias = -0.0004;
  sun.shadow.normalBias = 0.02;
  sun.shadow.radius = 3;
  const fill = new T.DirectionalLight(0xcfe3ff, 0.5);
  scene.add(hemi, sun, sun.target, fill, fill.target);
  // Warm ceiling lamps: a fixed, always-visible set (a changing light count
  // would recompile every shader). They hang under the open room's
  // Ceiling_Light panels, or along the corridor's.
  const lamps: THREE_NS.PointLight[] = [];
  for (let i = 0; i < (software ? LAMPS / 2 : LAMPS); i++) {
    const l = new T.PointLight(0xffd6a0, 0, 9, 2);
    lamps.push(l);
    scene.add(l);
  }
  function corridorLamps(): void {
    lamps.forEach((l, i) => l.position.set(1 + i * 4 * (LAMPS / lamps.length), CEIL_Y - 0.45, 0));
  }
  corridorLamps();

  // Ceilings: one plane per room / corridor, facing down (never casts).
  const ceilGeo = new T.PlaneGeometry(1, 1);
  ceilGeo.rotateX(Math.PI / 2);
  const ceilMat = new T.MeshStandardMaterial({ color: 0xf2eee6, roughness: 0.95, metalness: 0 });
  function ceiling(group: THREE_NS.Group, cx: number, cz: number, w: number, d: number): void {
    const m = new T.Mesh(ceilGeo, ceilMat);
    m.position.set(cx, CEIL_Y, cz);
    m.scale.set(w, 1, d);
    m.receiveShadow = false;
    m.castShadow = false;
    group.add(m);
  }

  // Theme-driven kit materials: lamp panels glow, window views dim at night.
  const glowMats: { mat: THREE_NS.MeshStandardMaterial; kind: 'panel' | 'window'; color: THREE_NS.Color; emissive: THREE_NS.Color; intensity: number }[] = [];
  {
    const seen = new Set<THREE_NS.Material>();
    for (const root of assets.kit.values())
      root.traverse((o) => {
        const mats = (o as THREE_NS.Mesh).material;
        for (const m of Array.isArray(mats) ? mats : mats ? [mats] : []) {
          if (seen.has(m)) continue;
          seen.add(m);
          const sm = m as THREE_NS.MeshStandardMaterial;
          const kind = m.name === 'LightPanel' ? 'panel' : m.name === 'WindowView' ? 'window' : null;
          if (kind && sm.emissive) glowMats.push({ mat: sm, kind, color: sm.color.clone(), emissive: sm.emissive.clone(), intensity: sm.emissiveIntensity });
        }
      });
  }
  const nightTint = new T.Color(0x4a5f9a);
  const one3 = new T.Color(1, 1, 1);

  return { renderer, canvas, scene, pmrem, envRT, hdr, camera, hemi, sun, fill, lamps, corridorLamps, ceilGeo, ceilMat, ceiling, glowMats, nightTint, one3 };
}

/** Light theme = a sunny afternoon; dark theme = evening: moonlight
 *  through the windows, the lamps carry the room, the views go dusky. */
export function applyTheme(T: Three, st: Stage, dark: boolean): void {
  const { renderer, scene, hdr, hemi, sun, fill, lamps, glowMats, nightTint, one3 } = st;
  scene.background = new T.Color(dark ? 0x141b28 : 0xcfe6fb);
  hemi.color.set(dark ? 0x7d8fb8 : 0xdfefff);
  hemi.groundColor.set(dark ? 0x3a3128 : 0x8a7a66);
  hemi.intensity = dark ? 0.45 : 1.0;
  sun.intensity = dark ? 0.55 : 2.6;
  sun.color.set(dark ? 0x9fb4ff : 0xfff0d6);
  fill.intensity = dark ? 0.15 : 0.5;
  scene.environmentIntensity = (hdr ? 0.8 : 0.55) * (dark ? 0.45 : 1);
  const lampI = (dark ? 12 : 2.2) * (lamps.length < LAMPS ? 1.3 : 1);
  for (const l of lamps) l.intensity = lampI;
  for (const g of glowMats) {
    if (g.kind === 'panel') {
      g.mat.emissive.copy(g.emissive);
      g.mat.emissiveIntensity = g.intensity * (dark ? 2.2 : 1);
    } else {
      g.mat.color.copy(g.color).multiplyScalar(dark ? 0.25 : 1);
      g.mat.emissive.copy(g.emissive).multiply(dark ? nightTint : one3);
      g.mat.emissiveIntensity = g.intensity * (dark ? 0.35 : 1);
    }
  }
  renderer.toneMappingExposure = dark ? 1.0 : 1.05;
}

/** Size the sun's shadow to room `r`, aim the sun / fill at it and hang the
 *  lamps under its ceiling panels. */
export function aimRoomLights(st: Stage, r: Room): void {
  const { sun, fill, lamps } = st;
  // Shadows sized to the room.
  const span = Math.max(r.width, r.depth) * 0.75;
  const sc = sun.shadow.camera;
  sc.left = -span;
  sc.right = span;
  sc.top = span;
  sc.bottom = -span;
  sc.near = 0.5;
  sc.far = 60;
  sc.updateProjectionMatrix();
  // Low afternoon sun from the window wall: long desk shadows eastward.
  sun.position.set(r.cx - r.width / 2 - 9, 7.5, r.cz - 2.5);
  sun.target.position.set(r.cx + 1, 0, r.cz);
  fill.position.set(r.cx + 6, 8, r.cz + 8);
  fill.target.position.set(r.cx, 0, r.cz);
  // Lamps under evenly spread ceiling panels.
  const spots = ceilingLights(r);
  lamps.forEach((l, i) => {
    const at = spots[Math.min(spots.length - 1, Math.floor(((i + 0.5) * spots.length) / lamps.length))];
    const w = toWorld(r, at);
    l.position.set(w.x, CEIL_Y - 0.45, w.z);
  });
}
