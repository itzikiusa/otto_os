/** Procedural canvas textures (original, generated at runtime — no image files). */
import * as THREE from 'three';
import { Rng } from './math';

/** A canvas and its 2D context (shared by the kart's `render/surfaces.ts`). */
export function canvas(w: number, h: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement('canvas');
  c.width = w; c.height = h;
  return [c, c.getContext('2d')!];
}
/** A repeating, anisotropic texture over `c`; `color: false` for data maps
 *  (normal / roughness) that must not be sRGB-decoded. */
export function finish(c: HTMLCanvasElement, repeat = true, color = true): THREE.CanvasTexture {
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = color ? THREE.SRGBColorSpace : THREE.NoColorSpace;
  if (repeat) t.wrapS = t.wrapT = THREE.RepeatWrapping;
  t.anisotropy = 8;
  return t;
}

export function checkerTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(128, 32);
  for (let x = 0; x < 16; x++) for (let y = 0; y < 4; y++) {
    g.fillStyle = (x + y) % 2 ? '#111111' : '#f5f5f5';
    g.fillRect(x * 8, y * 8, 8, 8);
  }
  return finish(c, false);
}
/** Ramp stripes. */
export function stripeTexture(a = '#ffcc00', b = '#222222'): THREE.CanvasTexture {
  const [c, g] = canvas(64, 64);
  g.fillStyle = a; g.fillRect(0, 0, 64, 64);
  g.fillStyle = b;
  for (let i = -64; i < 64; i += 32) { g.beginPath(); g.moveTo(i, 64); g.lineTo(i + 16, 64); g.lineTo(i + 80, 0); g.lineTo(i + 64, 0); g.fill(); }
  return finish(c);
}
/** Name tag sprite texture. */
export function labelTexture(text: string, color: string): THREE.CanvasTexture {
  const [c, g] = canvas(256, 64);
  g.font = '600 30px system-ui, sans-serif';
  const w = Math.min(244, g.measureText(text).width + 28);
  g.fillStyle = 'rgba(0,0,0,0.55)';
  g.beginPath(); g.roundRect((256 - w) / 2, 8, w, 46, 14); g.fill();
  g.fillStyle = color; g.textAlign = 'center'; g.textBaseline = 'middle';
  g.fillText(text, 128, 32, 230);
  return finish(c, false);
}

// ------------------------------------------------------------ surfaces (Storm Royale)

/** Surface families the shooter's architecture and terrain are painted with. */
export type SurfaceKind = 'plaster' | 'wood' | 'roof' | 'metal' | 'concrete' | 'brick' | 'rubble' | 'charred' | 'sandbag' | 'asphalt' | 'tile' | 'glass' | 'rust' | 'hazard' | 'planter' | 'grass' | 'dirt';
/** Speckle: `n` 1-3 px dots, light and dark, of strength `k`. */
function speckle(g: CanvasRenderingContext2D, rng: Rng, size: number, n: number, k: number): void {
  for (let i = 0; i < n; i++) {
    const v = rng.range(-1, 1);
    g.fillStyle = v > 0 ? `rgba(255,255,255,${v * k})` : `rgba(0,0,0,${-v * k})`;
    g.fillRect(rng.int(size), rng.int(size), 1 + rng.int(3), 1 + rng.int(3));
  }
}
/** Hairline cracks: random-walk polylines. */
function cracks(g: CanvasRenderingContext2D, rng: Rng, size: number, n: number, alpha: number): void {
  g.strokeStyle = `rgba(20,18,16,${alpha})`;
  for (let i = 0; i < n; i++) {
    let x = rng.range(0, size), y = rng.range(0, size), a = rng.range(0, Math.PI * 2);
    g.lineWidth = rng.range(0.6, 1.8);
    g.beginPath(); g.moveTo(x, y);
    for (let s = 0; s < 14; s++) { a += rng.range(-0.7, 0.7); x += Math.cos(a) * rng.range(3, 9); y += Math.sin(a) * rng.range(3, 9); g.lineTo(x, y); }
    g.stroke();
  }
}
/** Vertical grime streaks running down from the top (rain stains, soot). */
function streaks(g: CanvasRenderingContext2D, rng: Rng, size: number, n: number, rgb: string, alpha: number): void {
  for (let i = 0; i < n; i++) {
    const x = rng.range(0, size), w = rng.range(2, 9), h = rng.range(size * 0.2, size * 0.8), y = rng.chance(0.5) ? 0 : rng.range(0, size * 0.5);
    const grad = g.createLinearGradient(0, y, 0, y + h);
    grad.addColorStop(0, `rgba(${rgb},${alpha})`); grad.addColorStop(1, `rgba(${rgb},0)`);
    g.fillStyle = grad; g.fillRect(x, y, w, h);
  }
}
/**
 * A tiling, mostly neutral (light grey) albedo for one surface family, meant
 * to multiply a vertex colour that carries the hue: plaster stucco, wood
 * planks, roof tiles, riveted metal, board-marked concrete, brick courses,
 * rubble, soot, sandbags, cracked asphalt, ceramic tile, dark glazing, rust,
 * hazard stripes, soil, grass and dirt. 256 px, one repeat per 3 m.
 */
export function surfaceTexture(kind: SurfaceKind, seed = 1): THREE.CanvasTexture {
  const S = 256;
  const [c, g] = canvas(S, S);
  const rng = new Rng(hashKind(kind) ^ seed);
  const base = (v: number) => { g.fillStyle = `rgb(${v},${v},${v})`; g.fillRect(0, 0, S, S); };
  switch (kind) {
    case 'plaster':
      base(236); speckle(g, rng, S, 5000, 0.07);
      for (let i = 0; i < 26; i++) { g.fillStyle = `rgba(120,110,95,${rng.range(0.03, 0.08)})`; g.beginPath(); g.ellipse(rng.range(0, S), rng.range(0, S), rng.range(10, 50), rng.range(8, 30), rng.range(0, 3), 0, Math.PI * 2); g.fill(); }
      streaks(g, rng, S, 10, '70,64,56', 0.12); cracks(g, rng, S, 3, 0.35);
      break;
    case 'wood':
      base(210);
      for (let y = 0; y < S; y += 32) {
        g.fillStyle = `rgba(${60 + rng.int(40)},${40 + rng.int(20)},${20},${rng.range(0.08, 0.2)})`; g.fillRect(0, y, S, 32);
        for (let i = 0; i < 30; i++) { g.fillStyle = 'rgba(60,35,15,0.12)'; g.fillRect(rng.int(S), y + 3 + rng.int(26), 20 + rng.int(60), 1); }
        g.fillStyle = 'rgba(40,24,10,0.55)'; g.fillRect(0, y, S, 2); g.fillRect((y * 53) % S, y, 2, 32);
      }
      break;
    case 'roof':
      base(200);
      for (let row = 0; row < 16; row++) for (let col = 0; col < 9; col++) {
        const x = col * 32 + (row % 2) * 16, y = row * 16, v = 150 + rng.int(70);
        g.fillStyle = `rgb(${v},${v},${v})`; g.fillRect(x + 1, y + 1, 30, 14);
        g.fillStyle = 'rgba(0,0,0,0.25)'; g.fillRect(x + 1, y + 12, 30, 3);
      }
      break;
    case 'metal': case 'rust': {
      base(kind === 'rust' ? 190 : 214);
      // Corrugation: soft vertical ribs.
      for (let x = 0; x < S; x += 16) { const grad = g.createLinearGradient(x, 0, x + 16, 0); grad.addColorStop(0, 'rgba(0,0,0,0.16)'); grad.addColorStop(0.5, 'rgba(255,255,255,0.12)'); grad.addColorStop(1, 'rgba(0,0,0,0.16)'); g.fillStyle = grad; g.fillRect(x, 0, 16, S); }
      speckle(g, rng, S, 2000, 0.06);
      g.fillStyle = 'rgba(30,30,30,0.6)'; g.fillRect(0, 126, S, 3);
      for (let x = 8; x < S; x += 32) { g.beginPath(); g.arc(x, 132, 2.2, 0, Math.PI * 2); g.fill(); }
      streaks(g, rng, S, kind === 'rust' ? 34 : 12, kind === 'rust' ? '140,60,20' : '90,70,50', kind === 'rust' ? 0.45 : 0.15);
      if (kind === 'rust') for (let i = 0; i < 40; i++) { g.fillStyle = `rgba(150,${60 + rng.int(40)},20,${rng.range(0.1, 0.35)})`; g.beginPath(); g.arc(rng.range(0, S), rng.range(0, S), rng.range(3, 16), 0, Math.PI * 2); g.fill(); }
      break;
    }
    case 'concrete': case 'rubble': case 'charred': {
      base(kind === 'charred' ? 120 : 214); speckle(g, rng, S, 7000, 0.1);
      if (kind === 'concrete') {
        // Board-formed panels and tie holes.
        g.fillStyle = 'rgba(0,0,0,0.12)'; for (let y = 0; y < S; y += 64) g.fillRect(0, y, S, 2);
        for (let x = 0; x < S; x += 128) g.fillRect(x, 0, 2, S);
        g.fillStyle = 'rgba(0,0,0,0.3)'; for (let x = 32; x < S; x += 64) for (let y = 32; y < S; y += 64) { g.beginPath(); g.arc(x, y, 2.5, 0, Math.PI * 2); g.fill(); }
        streaks(g, rng, S, 16, '60,58,52', 0.14); cracks(g, rng, S, 4, 0.4);
      } else if (kind === 'rubble') {
        for (let i = 0; i < 160; i++) { const v = 120 + rng.int(110); g.fillStyle = `rgba(${v},${v - 6},${v - 12},0.9)`; g.beginPath(); const x = rng.range(0, S), y = rng.range(0, S), r = rng.range(3, 12); g.moveTo(x + r, y); for (let k = 1; k < 6; k++) g.lineTo(x + Math.cos(k * 1.26) * r * rng.range(0.6, 1.2), y + Math.sin(k * 1.26) * r * rng.range(0.6, 1.2)); g.fill(); }
        cracks(g, rng, S, 6, 0.5);
      } else {
        for (let i = 0; i < 60; i++) { g.fillStyle = `rgba(10,8,6,${rng.range(0.15, 0.5)})`; g.beginPath(); g.ellipse(rng.range(0, S), rng.range(0, S), rng.range(6, 40), rng.range(4, 20), rng.range(0, 3), 0, Math.PI * 2); g.fill(); }
        for (let i = 0; i < 30; i++) { g.fillStyle = `rgba(255,${120 + rng.int(60)},40,${rng.range(0.05, 0.15)})`; g.fillRect(rng.int(S), rng.int(S), 2, 2); }
        cracks(g, rng, S, 10, 0.6);
      }
      break;
    }
    case 'brick':
      base(205);
      for (let row = 0; row < 24; row++) for (let col = -1; col < 9; col++) {
        const x = col * 32 + (row % 2) * 16, y = row * 10.67, v = 150 + rng.int(80);
        g.fillStyle = `rgb(${v},${v},${v})`; g.fillRect(x + 1.5, y + 1.5, 29, 7.6);
      }
      speckle(g, rng, S, 3000, 0.08); streaks(g, rng, S, 10, '40,30,25', 0.18); cracks(g, rng, S, 2, 0.3);
      break;
    case 'sandbag':
      base(200);
      for (let row = 0; row < 10; row++) for (let col = -1; col < 6; col++) {
        const x = col * 52 + (row % 2) * 26, y = row * 25.6, v = 170 + rng.int(60);
        g.fillStyle = `rgb(${v},${v},${v})`; g.beginPath(); g.roundRect(x + 2, y + 2, 48, 22, 10); g.fill();
        g.strokeStyle = 'rgba(0,0,0,0.25)'; g.lineWidth = 2; g.stroke();
      }
      speckle(g, rng, S, 3000, 0.08);
      break;
    case 'asphalt':
      base(150); speckle(g, rng, S, 9000, 0.12); cracks(g, rng, S, 8, 0.55);
      for (let i = 0; i < 6; i++) { g.fillStyle = 'rgba(0,0,0,0.25)'; g.beginPath(); g.ellipse(rng.range(0, S), rng.range(0, S), rng.range(8, 22), rng.range(6, 14), rng.range(0, 3), 0, Math.PI * 2); g.fill(); }
      break;
    case 'tile':
      base(232);
      g.fillStyle = 'rgba(0,0,0,0.18)'; for (let i = 0; i <= S; i += 32) { g.fillRect(i, 0, 2, S); g.fillRect(0, i, S, 2); }
      speckle(g, rng, S, 1500, 0.05); streaks(g, rng, S, 14, '80,70,50', 0.18); cracks(g, rng, S, 3, 0.4);
      break;
    case 'glass': {
      base(70);
      const grad = g.createLinearGradient(0, 0, S, S); grad.addColorStop(0, 'rgba(255,255,255,0.18)'); grad.addColorStop(0.5, 'rgba(255,255,255,0)'); grad.addColorStop(1, 'rgba(255,255,255,0.1)');
      g.fillStyle = grad; g.fillRect(0, 0, S, S);
      g.fillStyle = 'rgba(200,200,200,0.7)'; for (let i = 0; i <= S; i += 64) { g.fillRect(i, 0, 4, S); g.fillRect(0, i, S, 4); }
      cracks(g, rng, S, 5, 0.5);
      break;
    }
    case 'hazard':
      base(225);
      g.fillStyle = 'rgba(25,25,25,0.85)';
      for (let i = -S; i < S; i += 48) { g.beginPath(); g.moveTo(i, S); g.lineTo(i + 24, S); g.lineTo(i + S + 24, 0); g.lineTo(i + S, 0); g.fill(); }
      speckle(g, rng, S, 2000, 0.1);
      break;
    case 'planter': case 'dirt':
      base(170); speckle(g, rng, S, 9000, 0.16);
      for (let i = 0; i < 80; i++) { g.fillStyle = `rgba(60,45,30,${rng.range(0.1, 0.3)})`; g.beginPath(); g.arc(rng.range(0, S), rng.range(0, S), rng.range(2, 8), 0, Math.PI * 2); g.fill(); }
      break;
    case 'grass':
      base(205);
      for (let i = 0; i < 2600; i++) { const v = rng.range(-1, 1); g.strokeStyle = v > 0 ? `rgba(255,255,230,${v * 0.18})` : `rgba(0,30,0,${-v * 0.2})`; g.lineWidth = 1; const x = rng.range(0, S), y = rng.range(0, S); g.beginPath(); g.moveTo(x, y); g.lineTo(x + rng.range(-2, 2), y - rng.range(3, 8)); g.stroke(); }
      break;
  }
  const t = finish(c);
  t.anisotropy = 8;
  return t;
}
function hashKind(kind: string): number { let h = 7; for (let i = 0; i < kind.length; i++) h = (h * 31 + kind.charCodeAt(i)) | 0; return h >>> 0; }

/** Cracked city asphalt for streets: patched, oil-stained, with faded lane dashes down the middle (v along the street). */
export function streetTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(256, 512);
  const rng = new Rng(41);
  g.fillStyle = '#3f4144'; g.fillRect(0, 0, 256, 512);
  speckle(g, rng, 512, 12000, 0.1);
  for (let i = 0; i < 10; i++) { g.fillStyle = `rgba(${rng.chance(0.5) ? '20,20,22' : '90,90,92'},0.35)`; g.fillRect(rng.range(10, 200), rng.range(0, 480), rng.range(20, 60), rng.range(20, 80)); }
  for (let i = 0; i < 12; i++) { g.fillStyle = 'rgba(0,0,0,0.25)'; g.beginPath(); g.ellipse(rng.range(20, 236), rng.range(0, 512), rng.range(6, 20), rng.range(10, 30), 0, 0, Math.PI * 2); g.fill(); }
  g.fillStyle = 'rgba(230,200,80,0.55)';
  for (let y = 0; y < 512; y += 128) g.fillRect(124, y + rng.range(0, 10), 8, 64);
  g.fillStyle = 'rgba(235,235,235,0.45)'; g.fillRect(14, 0, 6, 512); g.fillRect(236, 0, 6, 512);
  cracks(g, rng, 256, 22, 0.6);
  const t = finish(c);
  t.anisotropy = 8;
  return t;
}
/** Sidewalk slabs: square flags with joints and stains. */
export function pavementTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(256, 256);
  const rng = new Rng(43);
  g.fillStyle = '#8f8c86'; g.fillRect(0, 0, 256, 256);
  for (let x = 0; x < 4; x++) for (let y = 0; y < 4; y++) { const v = 130 + rng.int(30); g.fillStyle = `rgb(${v},${v - 2},${v - 6})`; g.fillRect(x * 64 + 2, y * 64 + 2, 60, 60); }
  speckle(g, rng, 256, 5000, 0.08); cracks(g, rng, 256, 6, 0.5);
  return finish(c);
}
/** Square decal sprites (transparent): a bullet hole, a blood splat, a scorch mark. */
export function decalTexture(kind: 'hole' | 'blood' | 'scorch'): THREE.CanvasTexture {
  const [c, g] = canvas(64, 64);
  const rng = new Rng(kind === 'hole' ? 3 : kind === 'blood' ? 5 : 9);
  if (kind === 'hole') {
    const grad = g.createRadialGradient(32, 32, 1, 32, 32, 20);
    grad.addColorStop(0, 'rgba(0,0,0,1)'); grad.addColorStop(0.25, 'rgba(20,18,16,0.95)'); grad.addColorStop(0.5, 'rgba(60,56,50,0.5)'); grad.addColorStop(1, 'rgba(0,0,0,0)');
    g.fillStyle = grad; g.fillRect(0, 0, 64, 64);
    g.strokeStyle = 'rgba(20,18,16,0.6)'; g.lineWidth = 1;
    for (let i = 0; i < 7; i++) { const a = rng.range(0, Math.PI * 2); g.beginPath(); g.moveTo(32 + Math.cos(a) * 5, 32 + Math.sin(a) * 5); g.lineTo(32 + Math.cos(a) * rng.range(12, 22), 32 + Math.sin(a) * rng.range(12, 22)); g.stroke(); }
  } else if (kind === 'blood') {
    g.fillStyle = 'rgba(110,8,8,0.9)';
    g.beginPath(); g.arc(32, 32, 12, 0, Math.PI * 2); g.fill();
    for (let i = 0; i < 18; i++) { const a = rng.range(0, Math.PI * 2), d = rng.range(10, 26), r = rng.range(1.5, 5); g.fillStyle = `rgba(${90 + rng.int(40)},6,6,${rng.range(0.6, 0.95)})`; g.beginPath(); g.arc(32 + Math.cos(a) * d, 32 + Math.sin(a) * d, r, 0, Math.PI * 2); g.fill(); }
  } else {
    const grad = g.createRadialGradient(32, 32, 2, 32, 32, 31);
    grad.addColorStop(0, 'rgba(8,6,4,0.95)'); grad.addColorStop(0.6, 'rgba(20,16,12,0.6)'); grad.addColorStop(1, 'rgba(0,0,0,0)');
    g.fillStyle = grad; g.fillRect(0, 0, 64, 64);
  }
  return finish(c, false);
}
/** A soft four-pointed muzzle flash star (additive). */
export function flashTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(64, 64);
  const grad = g.createRadialGradient(32, 32, 0, 32, 32, 30);
  grad.addColorStop(0, 'rgba(255,255,230,1)'); grad.addColorStop(0.25, 'rgba(255,210,120,0.9)'); grad.addColorStop(1, 'rgba(255,120,30,0)');
  g.fillStyle = grad; g.beginPath(); g.arc(32, 32, 30, 0, Math.PI * 2); g.fill();
  g.fillStyle = 'rgba(255,240,200,0.9)';
  for (const a of [0, Math.PI / 2, Math.PI, Math.PI * 1.5]) { g.beginPath(); g.moveTo(32 + Math.cos(a) * 31, 32 + Math.sin(a) * 31); g.lineTo(32 + Math.cos(a + 0.18) * 6, 32 + Math.sin(a + 0.18) * 6); g.lineTo(32 + Math.cos(a - 0.18) * 6, 32 + Math.sin(a - 0.18) * 6); g.fill(); }
  return finish(c, false);
}
/** Crack overlay for damaged player structures: white web on black, read as a mask. */
export function crackMaskTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(128, 128);
  const rng = new Rng(19);
  g.fillStyle = '#000'; g.fillRect(0, 0, 128, 128);
  for (let i = 0; i < 9; i++) {
    let x = rng.range(0, 128), y = rng.range(0, 128), a = rng.range(0, Math.PI * 2);
    // Earlier cracks are brighter: they show first as damage grows.
    const v = Math.round(255 - i * 22);
    g.strokeStyle = `rgb(${v},${v},${v})`; g.lineWidth = rng.range(1.2, 2.6);
    g.beginPath(); g.moveTo(x, y);
    for (let s = 0; s < 12; s++) { a += rng.range(-0.8, 0.8); x += Math.cos(a) * 8; y += Math.sin(a) * 8; g.lineTo(x, y); }
    g.stroke();
  }
  const t = new THREE.CanvasTexture(c);
  t.wrapS = t.wrapT = THREE.RepeatWrapping;
  return t;
}
