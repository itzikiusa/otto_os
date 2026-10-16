/** Procedural canvas textures (original, generated at runtime — no image files). */
import * as THREE from 'three';
import { Rng } from './math';

function canvas(w: number, h: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement('canvas');
  c.width = w; c.height = h;
  return [c, c.getContext('2d')!];
}
function finish(c: HTMLCanvasElement, repeat = true): THREE.CanvasTexture {
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  if (repeat) t.wrapS = t.wrapT = THREE.RepeatWrapping;
  t.anisotropy = 8;
  return t;
}

/** Asphalt with speckle, worn tyre lanes and white edge lines; u across, v along. */
export function asphaltTexture(base = '#55595f', line = '#f2f2f2', dashes = true): THREE.CanvasTexture {
  const [c, g] = canvas(256, 256);
  const rng = new Rng(7);
  g.fillStyle = base; g.fillRect(0, 0, 256, 256);
  for (let i = 0; i < 2600; i++) {
    const v = rng.range(-26, 26);
    g.fillStyle = `rgba(${v > 0 ? 255 : 0},${v > 0 ? 255 : 0},${v > 0 ? 255 : 0},${Math.abs(v) / 260})`;
    g.fillRect(rng.int(256), rng.int(256), 2, 2);
  }
  g.fillStyle = 'rgba(0,0,0,0.08)';
  g.fillRect(56, 0, 34, 256); g.fillRect(166, 0, 34, 256);
  g.fillStyle = line;
  g.fillRect(6, 0, 7, 256); g.fillRect(243, 0, 7, 256);
  if (dashes) { g.globalAlpha = 0.75; g.fillRect(125, 0, 6, 120); g.globalAlpha = 1; }
  return finish(c);
}
/** Red/white rumble strip, alternating along v. */
export function curbTexture(a = '#d8322f', b = '#f4f4f4'): THREE.CanvasTexture {
  const [c, g] = canvas(32, 64);
  g.fillStyle = a; g.fillRect(0, 0, 32, 32);
  g.fillStyle = b; g.fillRect(0, 32, 32, 32);
  return finish(c);
}
export function checkerTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(128, 32);
  for (let x = 0; x < 16; x++) for (let y = 0; y < 4; y++) {
    g.fillStyle = (x + y) % 2 ? '#111111' : '#f5f5f5';
    g.fillRect(x * 8, y * 8, 8, 8);
  }
  return finish(c, false);
}
/** Dash-panel chevrons (bright, used with an emissive-ish basic material). */
export function chevronTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(128, 128);
  g.fillStyle = '#ff8a00'; g.fillRect(0, 0, 128, 128);
  g.strokeStyle = '#fff36b'; g.lineWidth = 16; g.lineJoin = 'miter';
  for (const y of [34, 82]) { g.beginPath(); g.moveTo(16, y + 26); g.lineTo(64, y - 10); g.lineTo(112, y + 26); g.stroke(); }
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
/** Subtle ground detail that multiplies vertex colors. */
export function groundDetailTexture(): THREE.CanvasTexture {
  const [c, g] = canvas(128, 128);
  const rng = new Rng(11);
  g.fillStyle = '#ffffff'; g.fillRect(0, 0, 128, 128);
  for (let i = 0; i < 900; i++) {
    const v = Math.floor(rng.range(200, 255));
    g.fillStyle = `rgb(${v},${v},${v})`;
    g.fillRect(rng.int(128), rng.int(128), rng.int(3) + 1, rng.int(3) + 1);
  }
  return finish(c);
}
