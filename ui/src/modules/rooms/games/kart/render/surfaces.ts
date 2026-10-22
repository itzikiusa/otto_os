/**
 * Kart course surfaces, generated at runtime (no image files): world-space
 * asphalt with matching normal and roughness maps (so a branch's tarmac and
 * the main road's are the very same pixels where they overlap at a fork),
 * worn racing grooves, edge paint, corner tyre marks, rumble curbs, barrier
 * blocks, sponsor banners, animated dash-panel chevrons, the start grid and
 * one texture per special surface (sand, snow, cinders, ice, boardwalk
 * planks, metal decking). Albedos are near-neutral where a material colour
 * carries the theme's hue.
 */
import * as THREE from 'three';
import { Rng } from '../../common/math';

function canvas(w: number, h: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement('canvas');
  c.width = w; c.height = h;
  return [c, c.getContext('2d')!];
}
function texture(c: HTMLCanvasElement, color = true, repeat = true): THREE.CanvasTexture {
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = color ? THREE.SRGBColorSpace : THREE.NoColorSpace;
  if (repeat) t.wrapS = t.wrapT = THREE.RepeatWrapping;
  t.anisotropy = 8;
  return t;
}
/** Runs `draw` at the tile offsets that touch the canvas, so a mark crossing an edge wraps (tiles seamlessly). */
function wrapped(g: CanvasRenderingContext2D, w: number, h: number, draw: () => void): void {
  for (const dx of [-w, 0, w]) for (const dy of [-h, 0, h]) { g.save(); g.translate(dx, dy); draw(); g.restore(); }
}
/** Random-walk polyline from (x, y). */
function walk(g: CanvasRenderingContext2D, rng: Rng, x: number, y: number, steps: number, step: number, turn: number): void {
  let a = rng.range(0, Math.PI * 2);
  g.beginPath(); g.moveTo(x, y);
  for (let s = 0; s < steps; s++) { a += rng.range(-turn, turn); x += Math.cos(a) * step; y += Math.sin(a) * step; g.lineTo(x, y); }
  g.stroke();
}
/** Tangent-space normal map from a grey height canvas (wrapping), `strength` = slope gain. */
function normalFrom(src: HTMLCanvasElement, strength: number): THREE.CanvasTexture {
  const w = src.width, h = src.height;
  const data = src.getContext('2d')!.getImageData(0, 0, w, h).data;
  const [c, g] = canvas(w, h);
  const out = g.createImageData(w, h);
  const H = (x: number, y: number) => data[(((y + h) % h) * w + ((x + w) % w)) * 4] / 255;
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    // Canvas y runs down while texture v runs up (flipY): dh/dv = -dh/dy.
    const nx = -(H(x + 1, y) - H(x - 1, y)) * strength, ny = (H(x, y + 1) - H(x, y - 1)) * strength;
    const l = Math.hypot(nx, ny, 1), k = (y * w + x) * 4;
    out.data[k] = (nx / l * 0.5 + 0.5) * 255; out.data[k + 1] = (ny / l * 0.5 + 0.5) * 255; out.data[k + 2] = (1 / l * 0.5 + 0.5) * 255; out.data[k + 3] = 255;
  }
  g.putImageData(out, 0, 0);
  return texture(c, false);
}

/** Albedo + normal + roughness for one surface. */
export interface SurfaceSet { map: THREE.CanvasTexture; normalMap: THREE.CanvasTexture; roughnessMap: THREE.CanvasTexture }

/**
 * Race asphalt, one tile per 7 m of world XZ: near-white albedo (the road
 * colour tints it) with soft blotches, aggregate stones and quartz chips, tar
 * snakes over sealed cracks (darker, smoother) and hairline cracks; the
 * height canvas raises the stones and sinks the cracks for the normal map.
 */
export function asphaltSurface(seed = 7): SurfaceSet {
  const S = 512, rng = new Rng(seed);
  const [ac, a] = canvas(S, S), [hc, h] = canvas(S, S), [rc, r] = canvas(S, S);
  a.fillStyle = 'rgb(226,226,228)'; a.fillRect(0, 0, S, S);
  h.fillStyle = 'rgb(118,118,118)'; h.fillRect(0, 0, S, S);
  r.fillStyle = 'rgb(222,222,222)'; r.fillRect(0, 0, S, S);
  // Patches of newer / older surface.
  for (let i = 0; i < 46; i++) {
    const x = rng.range(0, S), y = rng.range(0, S), rad = rng.range(40, 140), dark = rng.chance(0.5), k = rng.range(0.015, 0.045);
    wrapped(a, S, S, () => {
      const grad = a.createRadialGradient(x, y, 0, x, y, rad);
      grad.addColorStop(0, dark ? `rgba(40,40,46,${k})` : `rgba(255,255,255,${k})`); grad.addColorStop(1, 'rgba(0,0,0,0)');
      a.fillStyle = grad; a.fillRect(x - rad, y - rad, rad * 2, rad * 2);
    });
    if (dark) wrapped(r, S, S, () => {
      const grad = r.createRadialGradient(x, y, 0, x, y, rad);
      grad.addColorStop(0, `rgba(0,0,0,${k * 1.4})`); grad.addColorStop(1, 'rgba(0,0,0,0)');
      r.fillStyle = grad; r.fillRect(x - rad, y - rad, rad * 2, rad * 2);
    });
  }
  // Aggregate: stones of three sizes, each raised in the height map.
  for (let i = 0; i < 22000; i++) {
    const x = rng.int(S), y = rng.int(S), size = rng.chance(0.85) ? 1 + rng.int(2) : 3, v = rng.range(-1, 1);
    a.fillStyle = v > 0 ? `rgba(255,255,255,${v * 0.18})` : `rgba(0,0,0,${-v * 0.22})`; a.fillRect(x, y, size, size);
    h.fillStyle = `rgba(255,255,255,${0.25 + rng.range(0, 0.35)})`; h.fillRect(x, y, size, size);
    if (rng.chance(0.3)) { r.fillStyle = `rgba(255,255,255,${rng.range(0.05, 0.15)})`; r.fillRect(x, y, size, size); }
  }
  for (let i = 0; i < 700; i++) { a.fillStyle = `rgba(255,255,250,${rng.range(0.25, 0.5)})`; a.fillRect(rng.int(S), rng.int(S), 1, 1); }
  // Tar snakes: sealed cracks, dark, glossy, slightly proud.
  for (let i = 0; i < 3; i++) {
    const x = rng.range(0, S), y = rng.range(0, S), seed2 = rng.int(1e9), width = rng.range(2.5, 4);
    for (const [g, style] of [[a, 'rgba(18,18,20,0.22)'], [h, 'rgba(150,150,150,0.4)'], [r, 'rgba(150,150,150,0.7)']] as [CanvasRenderingContext2D, string][]) {
      g.strokeStyle = style; g.lineWidth = width; g.lineJoin = 'round'; g.lineCap = 'round';
      wrapped(g, S, S, () => walk(g, new Rng(seed2), x, y, 26, 9, 0.6));
    }
  }
  // Hairline cracks: thin and sunken.
  for (let i = 0; i < 4; i++) {
    const x = rng.range(0, S), y = rng.range(0, S), seed2 = rng.int(1e9);
    for (const [g, style] of [[a, 'rgba(10,10,12,0.18)'], [h, 'rgba(0,0,0,0.45)']] as [CanvasRenderingContext2D, string][]) {
      g.strokeStyle = style; g.lineWidth = 1.2;
      wrapped(g, S, S, () => walk(g, new Rng(seed2), x, y, 14, 6, 0.9));
    }
  }
  return { map: texture(ac), normalMap: normalFrom(hc, 2.2), roughnessMap: texture(rc, false) };
}

/**
 * Racing grooves laid over the road (u across, v along, alpha): two worn
 * rubbered lanes, rubber streaks, and oil drips down the middle.
 */
export function grooveTexture(): THREE.CanvasTexture {
  const W = 128, H = 512, rng = new Rng(17);
  const [c, g] = canvas(W, H);
  for (const u of [0.27, 0.73]) {
    const x = u * W, half = W * 0.11;
    const grad = g.createLinearGradient(x - half, 0, x + half, 0);
    grad.addColorStop(0, 'rgba(14,14,16,0)'); grad.addColorStop(0.5, 'rgba(14,14,16,0.26)'); grad.addColorStop(1, 'rgba(14,14,16,0)');
    g.fillStyle = grad; g.fillRect(x - half, 0, half * 2, H);
    for (let i = 0; i < 22; i++) { g.fillStyle = `rgba(8,8,10,${rng.range(0.04, 0.12)})`; g.fillRect(x + rng.range(-half * 0.7, half * 0.7), 0, rng.range(0.6, 2), H); }
  }
  for (let i = 0; i < 40; i++) {
    const x = W / 2 + rng.range(-5, 5), y = rng.range(0, H), rx = rng.range(0.6, 1.6), ry = rng.range(1, 3);
    wrapped(g, 0, H, () => { g.fillStyle = `rgba(6,6,8,${rng.range(0.03, 0.08)})`; g.beginPath(); g.ellipse(x, y, rx, ry, 0, 0, Math.PI * 2); g.fill(); });
  }
  return texture(c);
}

/** Road paint (u across the line, v along): white with worn speckle in the alpha. */
export function paintTexture(color = '#f4f4f0'): THREE.CanvasTexture {
  const [c, g] = canvas(16, 256);
  const rng = new Rng(23);
  g.fillStyle = color; g.fillRect(0, 0, 16, 256);
  g.globalCompositeOperation = 'destination-out';
  for (let i = 0; i < 260; i++) { g.fillStyle = `rgba(0,0,0,${rng.range(0.2, 0.7)})`; g.fillRect(rng.int(16), rng.int(256), 1 + rng.int(2), 1 + rng.int(3)); }
  return texture(c);
}

/** Tyre marks (u across the road, v along; alpha): pairs of dark drift arcs swinging across the inside lanes. */
export function skidTexture(): THREE.CanvasTexture {
  const W = 256, H = 512, rng = new Rng(29);
  const [c, g] = canvas(W, H);
  g.lineCap = 'round';
  for (let p = 0; p < 7; p++) {
    const x0 = rng.range(0.12, 0.88) * W, amp = rng.range(6, 26), k = 1 + rng.int(2), ph = rng.range(0, 6.28), gauge = rng.range(20, 28), alpha = rng.range(0.22, 0.5);
    const env0 = rng.range(0, 6.28);
    for (const off of [-gauge / 2, gauge / 2]) {
      for (let y = 0; y < H; y += 4) {
        const t0 = (y / H) * Math.PI * 2, t1 = ((y + 4) / H) * Math.PI * 2;
        const env = Math.max(0, Math.sin(t0 + env0)) ** 0.7;
        if (env < 0.05) continue;
        g.strokeStyle = `rgba(12,12,14,${alpha * env})`; g.lineWidth = rng.range(4, 6);
        g.beginPath(); g.moveTo(x0 + off + Math.sin(t0 * k + ph) * amp, y); g.lineTo(x0 + off + Math.sin(t1 * k + ph) * amp, y + 4); g.stroke();
      }
    }
  }
  return texture(c);
}

/**
 * Rumble curb blocks (u across the curb profile, v along): alternating `a` /
 * `b` blocks with a dark joint and shading on the inner slope.
 */
export function rumbleTexture(a: string, b: string): THREE.CanvasTexture {
  const [c, g] = canvas(64, 128);
  g.fillStyle = a; g.fillRect(0, 0, 64, 64);
  g.fillStyle = b; g.fillRect(0, 64, 64, 64);
  g.fillStyle = 'rgba(0,0,0,0.28)'; g.fillRect(0, 62, 64, 3); g.fillRect(0, 0, 64, 2);
  const grad = g.createLinearGradient(0, 0, 64, 0);
  grad.addColorStop(0, 'rgba(0,0,0,0.22)'); grad.addColorStop(0.3, 'rgba(0,0,0,0)'); grad.addColorStop(0.75, 'rgba(255,255,255,0.12)'); grad.addColorStop(1, 'rgba(0,0,0,0.18)');
  g.fillStyle = grad; g.fillRect(0, 0, 64, 128);
  const rng = new Rng(31);
  for (let i = 0; i < 300; i++) { g.fillStyle = `rgba(0,0,0,${rng.range(0.03, 0.12)})`; g.fillRect(rng.int(64), rng.int(128), 2, 2); }
  return texture(c);
}

/**
 * Barrier wall blocks (u: 0..0.5 the inner face bottom to top, 0.5..0.75 the
 * cap, 0.75..1 the outer face; v along, one block pair per tile): `a` / `b`
 * blocks with bevelled joints, a lit top edge and road grime at the foot.
 */
export function barrierTexture(a: string, b: string): THREE.CanvasTexture {
  const [c, g] = canvas(128, 128);
  g.fillStyle = a; g.fillRect(0, 0, 128, 64);
  g.fillStyle = b; g.fillRect(0, 64, 128, 64);
  for (const y of [0, 64]) { g.fillStyle = 'rgba(0,0,0,0.35)'; g.fillRect(0, y, 128, 3); g.fillStyle = 'rgba(255,255,255,0.2)'; g.fillRect(0, y + 3, 128, 2); }
  const grime = g.createLinearGradient(0, 0, 40, 0);
  grime.addColorStop(0, 'rgba(30,26,22,0.55)'); grime.addColorStop(1, 'rgba(30,26,22,0)');
  g.fillStyle = grime; g.fillRect(0, 0, 40, 128);
  g.fillStyle = 'rgba(255,255,255,0.22)'; g.fillRect(60, 0, 6, 128);
  g.fillStyle = 'rgba(0,0,0,0.18)'; g.fillRect(64, 0, 32, 128);
  const rng = new Rng(37);
  for (let i = 0; i < 400; i++) { g.fillStyle = `rgba(0,0,0,${rng.range(0.02, 0.1)})`; g.fillRect(rng.int(128), rng.int(128), 2, 2); }
  return texture(c);
}

/** Sponsor banner strip (v along the wall, u up the banner): made-up race sponsors in the theme palette. */
export function bannerTexture(bg: string, fg: string, accent: string): THREE.CanvasTexture {
  const W = 128, H = 1024;
  const [c, g] = canvas(W, H);
  const words = ['OTTO GP', '★ STAR CUP', 'TURBO', 'KART KIT', 'DRIFT CO.', 'SPEEDWAY'];
  const panel = H / words.length;
  words.forEach((word, k) => {
    const y = k * panel;
    g.fillStyle = k % 2 ? accent : bg; g.fillRect(0, y, W, panel);
    g.fillStyle = 'rgba(0,0,0,0.25)'; g.fillRect(0, y, W, 3);
    // The banner runs along v: draw the words rotated so they read left to right from the road.
    g.save(); g.translate(W / 2, y + panel / 2); g.rotate(-Math.PI / 2);
    g.font = 'italic 800 64px system-ui, sans-serif'; g.textAlign = 'center'; g.textBaseline = 'middle';
    g.fillStyle = k % 2 ? bg : fg; g.fillText(word, 0, 2, panel - 16);
    g.restore();
  });
  return texture(c);
}

/** Dash panel (alpha): two glowing chevrons per tile pointing up the tile; scroll its offset to animate. */
export function dashTexture(): THREE.CanvasTexture {
  const W = 128, H = 256;
  const [c, g] = canvas(W, H);
  const grad = g.createLinearGradient(0, 0, 0, H);
  grad.addColorStop(0, 'rgba(255,120,0,0.92)'); grad.addColorStop(0.5, 'rgba(255,176,0,0.92)'); grad.addColorStop(1, 'rgba(255,120,0,0.92)');
  g.fillStyle = grad; g.fillRect(0, 0, W, H);
  g.lineJoin = 'miter'; g.lineCap = 'butt';
  for (const y of [40, 168]) {
    for (const [w, s] of [[30, 'rgba(255,255,190,0.35)'], [18, '#fffbe0']] as [number, string][]) {
      g.strokeStyle = s; g.lineWidth = w;
      // Tip at the top of the tile (canvas top = forward, v = 1).
      g.beginPath(); g.moveTo(14, y + 62); g.lineTo(64, y + 8); g.lineTo(114, y + 62); g.stroke();
    }
  }
  return texture(c);
}

/** One painted grid box (alpha): a bracket open toward the back, the kart's nose at the top. */
export function gridSlotTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(128, 128);
  g.strokeStyle = 'rgba(245,245,240,0.92)'; g.lineWidth = 9; g.lineCap = 'square';
  g.beginPath(); g.moveTo(10, 120); g.lineTo(10, 10); g.lineTo(118, 10); g.lineTo(118, 120); g.stroke();
  g.globalCompositeOperation = 'destination-out';
  const rng = new Rng(41);
  for (let i = 0; i < 300; i++) { g.fillStyle = `rgba(0,0,0,${rng.range(0.2, 0.6)})`; g.fillRect(rng.int(128), rng.int(128), 2, 2); }
  return texture(c, true, false);
}

/** Special surfaces laid over or instead of tarmac. */
export type SurfaceKind = 'sand' | 'snow' | 'cinder' | 'mud' | 'ice' | 'planks' | 'metal';
/**
 * A tiling texture for a special surface (u across the road, v along). The
 * off-road kinds fade to transparent at both edges so they bleed into the
 * tarmac; ice is translucent with cracks and trapped bubbles; planks run
 * across the road with dark gaps and nail heads; metal is diamond plate in
 * riveted panels.
 */
export function surfaceTexture(kind: SurfaceKind): THREE.CanvasTexture {
  const S = 256, rng = new Rng(kind.length * 977 + kind.charCodeAt(0));
  const [c, g] = canvas(S, S);
  const speckle = (n: number, k: number, size = 2) => { for (let i = 0; i < n; i++) { const v = rng.range(-1, 1); g.fillStyle = v > 0 ? `rgba(255,255,255,${v * k})` : `rgba(0,0,0,${-v * k})`; g.fillRect(rng.int(S), rng.int(S), 1 + rng.int(size), 1 + rng.int(size)); } };
  switch (kind) {
    case 'sand':
      g.fillStyle = 'rgb(226,196,138)'; g.fillRect(0, 0, S, S);
      // Wind ripples across the road, wavy so they tile along v.
      for (let y = 0; y < S; y += 11) for (let x = 0; x < S; x += 4) { g.fillStyle = 'rgba(150,110,60,0.22)'; g.fillRect(x, y + Math.sin(x / 18 + y) * 2.5, 4, 2); }
      speckle(5000, 0.14);
      break;
    case 'snow':
      g.fillStyle = 'rgb(244,248,255)'; g.fillRect(0, 0, S, S);
      for (let i = 0; i < 30; i++) { g.fillStyle = `rgba(150,180,220,${rng.range(0.06, 0.16)})`; g.beginPath(); g.ellipse(rng.range(0, S), rng.range(0, S), rng.range(10, 40), rng.range(6, 20), 0, 0, Math.PI * 2); g.fill(); }
      // Wheel ruts.
      for (const u of [0.3, 0.7]) { g.fillStyle = 'rgba(140,170,210,0.18)'; g.fillRect(u * S - 10, 0, 20, S); }
      for (let i = 0; i < 300; i++) { g.fillStyle = 'rgba(255,255,255,0.95)'; g.fillRect(rng.int(S), rng.int(S), 1, 1); }
      speckle(1500, 0.05);
      break;
    case 'cinder':
      g.fillStyle = 'rgb(66,50,46)'; g.fillRect(0, 0, S, S);
      speckle(9000, 0.22, 3);
      for (let i = 0; i < 160; i++) { g.fillStyle = `rgba(255,${80 + rng.int(80)},20,${rng.range(0.3, 0.8)})`; g.fillRect(rng.int(S), rng.int(S), 2, 2); }
      break;
    case 'mud':
      g.fillStyle = 'rgb(118,86,54)'; g.fillRect(0, 0, S, S);
      for (let i = 0; i < 22; i++) { g.fillStyle = `rgba(60,40,24,${rng.range(0.2, 0.4)})`; g.beginPath(); g.ellipse(rng.range(0, S), rng.range(0, S), rng.range(12, 34), rng.range(8, 20), rng.range(0, 3), 0, Math.PI * 2); g.fill(); }
      speckle(5000, 0.16);
      break;
    case 'ice': {
      g.fillStyle = 'rgba(196,232,255,0.78)'; g.fillRect(0, 0, S, S);
      for (let i = 0; i < 26; i++) { g.fillStyle = `rgba(255,255,255,${rng.range(0.04, 0.12)})`; g.beginPath(); g.ellipse(rng.range(0, S), rng.range(0, S), rng.range(14, 50), rng.range(6, 22), rng.range(0, 3), 0, Math.PI * 2); g.fill(); }
      g.strokeStyle = 'rgba(255,255,255,0.75)'; g.lineWidth = 1.2;
      for (let i = 0; i < 14; i++) { const x = rng.range(0, S), y = rng.range(0, S), s2 = rng.int(1e9); wrapped(g, S, S, () => walk(g, new Rng(s2), x, y, 12, 9, 0.8)); }
      for (let i = 0; i < 90; i++) { g.strokeStyle = 'rgba(255,255,255,0.55)'; g.lineWidth = 1; g.beginPath(); g.arc(rng.range(0, S), rng.range(0, S), rng.range(1, 3.5), 0, Math.PI * 2); g.stroke(); }
      return texture(c);
    }
    case 'planks': {
      g.fillStyle = 'rgb(200,160,112)'; g.fillRect(0, 0, S, S);
      for (let y = 0; y < S; y += 32) {
        const v = rng.range(-26, 22);
        g.fillStyle = `rgb(${200 + v},${158 + v},${110 + v * 0.8})`; g.fillRect(0, y, S, 32);
        for (let i = 0; i < 26; i++) { g.fillStyle = `rgba(90,56,26,${rng.range(0.08, 0.22)})`; g.fillRect(rng.int(S), y + 2 + rng.int(28), 30 + rng.int(90), 1); }
        g.fillStyle = 'rgba(40,24,10,0.85)'; g.fillRect(0, y, S, 3);
        g.fillStyle = 'rgba(60,60,60,0.8)'; for (const x of [10, S - 14]) { g.fillRect(x, y + 9, 3, 3); g.fillRect(x, y + 21, 3, 3); }
      }
      return texture(c);
    }
    case 'metal': {
      g.fillStyle = 'rgb(176,182,190)'; g.fillRect(0, 0, S, S);
      for (let y = 0; y < S; y += 16) for (let x = (y / 16) % 2 ? 8 : 0; x < S; x += 16) {
        g.fillStyle = 'rgba(255,255,255,0.35)'; g.save(); g.translate(x + 4, y + 4); g.rotate(Math.PI / 4); g.fillRect(-5, -1.5, 10, 3); g.restore();
        g.fillStyle = 'rgba(0,0,0,0.22)'; g.save(); g.translate(x + 5, y + 5); g.rotate(Math.PI / 4); g.fillRect(-5, -1, 10, 2); g.restore();
      }
      g.fillStyle = 'rgba(20,24,30,0.7)'; g.fillRect(0, 0, S, 3); g.fillRect(0, 0, 3, S); g.fillRect(S / 2, 0, 2, S);
      g.fillStyle = 'rgba(40,40,44,0.9)'; for (let x = 10; x < S; x += 40) { g.beginPath(); g.arc(x, 9, 2.6, 0, Math.PI * 2); g.fill(); }
      speckle(1500, 0.07);
      return texture(c);
    }
  }
  // Off-road kinds fade out toward both edges of their patch.
  g.globalCompositeOperation = 'destination-in';
  const fade = g.createLinearGradient(0, 0, S, 0);
  fade.addColorStop(0, 'rgba(0,0,0,0)'); fade.addColorStop(0.12, 'rgba(0,0,0,1)'); fade.addColorStop(0.88, 'rgba(0,0,0,1)'); fade.addColorStop(1, 'rgba(0,0,0,0)');
  g.fillStyle = fade; g.fillRect(0, 0, S, S);
  return texture(c);
}

/** Soft round glow (additive sprites: drift sparks, exhaust and lamp halos). */
export function glowTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(64, 64);
  const grad = g.createRadialGradient(32, 32, 0, 32, 32, 32);
  grad.addColorStop(0, 'rgba(255,255,255,1)'); grad.addColorStop(0.25, 'rgba(255,255,255,0.65)'); grad.addColorStop(0.6, 'rgba(255,255,255,0.15)'); grad.addColorStop(1, 'rgba(255,255,255,0)');
  g.fillStyle = grad; g.fillRect(0, 0, 64, 64);
  return texture(c, true, false);
}

/** Chevron board face: white arrows on red pointing left (mirror the mesh for right-hand bends). */
export function chevronBoardTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(256, 128);
  g.fillStyle = '#d8322f'; g.fillRect(0, 0, 256, 128);
  g.fillStyle = '#ffffff';
  for (const x of [20, 100, 180]) { g.beginPath(); g.moveTo(x + 50, 14); g.lineTo(x + 6, 64); g.lineTo(x + 50, 114); g.lineTo(x + 72, 114); g.lineTo(x + 28, 64); g.lineTo(x + 72, 14); g.closePath(); g.fill(); }
  g.strokeStyle = '#ffffff'; g.lineWidth = 8; g.strokeRect(4, 4, 248, 120);
  return texture(c, true, false);
}

/** Hexagon-plated shell dome (u around, v up): white plate rims over the tint the material carries. */
export function shellTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(128, 64);
  g.fillStyle = '#ffffff'; g.fillRect(0, 0, 128, 64);
  g.strokeStyle = 'rgba(0,0,0,0.35)'; g.lineWidth = 3;
  for (let row = 0; row < 3; row++) for (let k = 0; k < 6; k++) {
    const x = k * 22 + (row % 2) * 11, y = 10 + row * 20;
    g.beginPath();
    for (let a = 0; a <= 6; a++) { const t = (a / 6) * Math.PI * 2; g.lineTo(x + Math.cos(t) * 9, y + Math.sin(t) * 9); }
    g.stroke();
  }
  return texture(c);
}

/** Landscape detail families (near-white, multiplying the terrain's vertex colours). */
export type TerrainKind = 'grass' | 'sand' | 'snow' | 'rock' | 'pave';
/**
 * Tiling landscape detail, one tile per 6 m: grass blades and clover,
 * rippled sand and pebbles, wind-crusted snow, cracked rock, or worn paving
 * flags. Mostly 200–255 grey so the vertex palette keeps the hue.
 */
export function terrainTexture(kind: TerrainKind): THREE.CanvasTexture {
  const S = 256, rng = new Rng(kind.charCodeAt(0) * 131 + 5);
  const [c, g] = canvas(S, S);
  g.fillStyle = 'rgb(236,236,236)'; g.fillRect(0, 0, S, S);
  for (let i = 0; i < 30; i++) {
    const x = rng.range(0, S), y = rng.range(0, S), r = rng.range(20, 70), k = rng.range(0.04, 0.1), dark = rng.chance(0.5);
    wrapped(g, S, S, () => { const grad = g.createRadialGradient(x, y, 0, x, y, r); grad.addColorStop(0, dark ? `rgba(0,0,0,${k})` : `rgba(255,255,255,${k})`); grad.addColorStop(1, 'rgba(0,0,0,0)'); g.fillStyle = grad; g.fillRect(x - r, y - r, r * 2, r * 2); });
  }
  switch (kind) {
    case 'grass':
      for (let i = 0; i < 5200; i++) {
        const x = rng.range(0, S), y = rng.range(0, S), v = rng.range(-1, 1);
        g.strokeStyle = v > 0 ? `rgba(255,255,220,${v * 0.22})` : `rgba(0,40,0,${-v * 0.24})`; g.lineWidth = 1.2;
        g.beginPath(); g.moveTo(x, y); g.lineTo(x + rng.range(-2.5, 2.5), y - rng.range(3, 9)); g.stroke();
      }
      break;
    case 'sand':
      for (let y = 0; y < S; y += 9) for (let x = 0; x < S; x += 3) { g.fillStyle = 'rgba(120,80,40,0.08)'; g.fillRect(x, y + Math.sin(x / 15 + y * 0.3) * 2, 3, 2); }
      for (let i = 0; i < 260; i++) { g.fillStyle = `rgba(90,60,40,${rng.range(0.15, 0.35)})`; g.beginPath(); g.arc(rng.range(0, S), rng.range(0, S), rng.range(1, 2.6), 0, Math.PI * 2); g.fill(); }
      break;
    case 'snow':
      for (let i = 0; i < 26; i++) { g.fillStyle = `rgba(140,170,215,${rng.range(0.05, 0.12)})`; g.beginPath(); g.ellipse(rng.range(0, S), rng.range(0, S), rng.range(14, 50), rng.range(5, 14), rng.range(0, 3), 0, Math.PI * 2); g.fill(); }
      for (let i = 0; i < 400; i++) { g.fillStyle = 'rgba(255,255,255,1)'; g.fillRect(rng.int(S), rng.int(S), 1, 1); }
      break;
    case 'rock':
      g.strokeStyle = 'rgba(0,0,0,0.3)'; g.lineWidth = 1.4;
      for (let i = 0; i < 18; i++) { const x = rng.range(0, S), y = rng.range(0, S), s = rng.int(1e9); wrapped(g, S, S, () => walk(g, new Rng(s), x, y, 12, 8, 0.8)); }
      for (let i = 0; i < 4000; i++) { const v = rng.range(-1, 1); g.fillStyle = v > 0 ? `rgba(255,255,255,${v * 0.12})` : `rgba(0,0,0,${-v * 0.16})`; g.fillRect(rng.int(S), rng.int(S), 2, 2); }
      break;
    case 'pave':
      for (let y = 0; y < S; y += 64) for (let x = 0; x < S; x += 64) { const v = 210 + rng.int(40); g.fillStyle = `rgba(${v},${v},${v},0.5)`; g.fillRect(x + 2, y + 2, 60, 60); }
      g.fillStyle = 'rgba(0,0,0,0.16)'; for (let i = 0; i <= S; i += 64) { g.fillRect(i, 0, 2, S); g.fillRect(0, i, S, 2); }
      for (let i = 0; i < 2500; i++) { const v = rng.range(-1, 1); g.fillStyle = v > 0 ? `rgba(255,255,255,${v * 0.08})` : `rgba(0,0,0,${-v * 0.1})`; g.fillRect(rng.int(S), rng.int(S), 2, 2); }
      break;
  }
  return texture(c);
}
