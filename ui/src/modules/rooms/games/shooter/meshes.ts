/**
 * Procedural three.js geometry for Storm Royale: the walk-in architecture
 * (one merged, vertex-coloured mesh), player structures (one merged mesh per
 * material, rewritten in place when they change), generated wood / brick /
 * metal textures, build-preview ghosts, launch pads and the pickaxe.
 * Everything here is made at runtime; no image or model files.
 */
import * as THREE from 'three';
import { Rng } from '../common/math';
import { BLOCK_PALETTE, type Block, type Box } from './map';
import { baseY, LEVEL, maxHp, TILE, tileX, type Build } from './builds';

/** Corner order: bottom 0-3 (x0z0, x1z0, x1z1, x0z1), top 4-7 in the same order. */
const FACES = [[0, 1, 2, 3], [4, 7, 6, 5], [3, 2, 6, 7], [1, 0, 4, 5], [2, 1, 5, 6], [0, 3, 7, 4]];
const _a = new THREE.Vector3(), _b = new THREE.Vector3(), _n = new THREE.Vector3();

class MeshWriter {
  pos: number[] = []; nor: number[] = []; uv: number[] = []; col: number[] = []; idx: number[] = [];
  private uvScale: number;
  constructor(uvScale: number) { this.uvScale = uvScale; }
  /** Adds a hexahedron from eight [x, y, z] corners with a flat colour (top faces a touch lighter). */
  hexa(c: number[][], color: THREE.Color): void {
    for (let f = 0; f < 6; f++) {
      const [i0, i1, i2, i3] = FACES[f];
      const p0 = c[i0], p1 = c[i1], p2 = c[i2];
      _a.set(p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]);
      _b.set(p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]);
      _n.crossVectors(_a, _b).normalize();
      const ax = Math.abs(_n.x), ay = Math.abs(_n.y), az = Math.abs(_n.z);
      const shade = f === 1 ? 1.06 : f === 0 ? 0.8 : 1;
      const base = this.pos.length / 3;
      for (const i of [i0, i1, i2, i3]) {
        const [x, y, z] = c[i];
        this.pos.push(x, y, z);
        this.nor.push(_n.x, _n.y, _n.z);
        const [u, v] = ay >= ax && ay >= az ? [x, z] : ax >= az ? [z, y] : [x, y];
        this.uv.push(u * this.uvScale, v * this.uvScale);
        this.col.push(Math.min(1, color.r * shade), Math.min(1, color.g * shade), Math.min(1, color.b * shade));
      }
      this.idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
    }
  }
  box(b: Box, color: THREE.Color): void {
    const { minX: x0, minY: y0, minZ: z0, maxX: x1, maxY: y1, maxZ: z1 } = b;
    this.hexa([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1], [x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], color);
  }
  geometry(): THREE.BufferGeometry {
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.Float32BufferAttribute(this.pos, 3));
    g.setAttribute('normal', new THREE.Float32BufferAttribute(this.nor, 3));
    g.setAttribute('uv', new THREE.Float32BufferAttribute(this.uv, 2));
    g.setAttribute('color', new THREE.Float32BufferAttribute(this.col, 3));
    g.setIndex(this.idx);
    g.computeBoundingSphere();
    return g;
  }
}

/** All static architecture as one mesh (shared colours per palette entry, subtle per-block variation). */
export function blockGeometry(blocks: Block[]): THREE.BufferGeometry {
  const w = new MeshWriter(1 / 3);
  const palette = BLOCK_PALETTE.map(c => new THREE.Color(c));
  const c = new THREE.Color();
  blocks.forEach((b, i) => {
    c.copy(palette[b.mat] ?? palette[0]).multiplyScalar(0.94 + ((i * 2654435761) % 997) / 997 * 0.1);
    w.box(b, c);
  });
  return w.geometry();
}

/** Structure tint: pieces still going up glow blue, damaged ones darken. */
function structureTint(b: Build, out: THREE.Color): THREE.Color {
  const k = Math.min(1, Math.max(0, b.hp / maxHp(b)));
  if (b.grow > 0) return out.setRGB(1, 1, 1).lerp(_growing, 0.6);
  return out.setRGB(1, 1, 1).lerp(_worn, (1 - k) * 0.6);
}
const _growing = new THREE.Color(0.55, 0.8, 1.25), _worn = new THREE.Color(0.45, 0.42, 0.4), _tint = new THREE.Color();
/** Quads per structure: six faces per box, at most four boxes (a window wall). */
const QUADS_PER_PIECE = 24;
const _c = new Float32Array(24);

/**
 * Player structures of one material as one mesh whose buffers are written in
 * place: capacity grows by doubling (never shrinks), the index pattern is
 * shared, and only the written range is uploaded. A health-only change
 * rewrites just the colours.
 */
export class StructureMesh {
  readonly geometry = new THREE.BufferGeometry();
  private pieces = 0;
  private pos = new Float32Array(0); private nor = new Float32Array(0); private uv = new Float32Array(0); private col = new Float32Array(0);
  private quads = 0;

  /** Rewrites the pieces of `mat`; returns the number of pieces drawn. */
  update(builds: readonly Build[], mat: number, coloursOnly = false): number {
    let n = 0;
    for (const b of builds) if (b.mat === mat) n++;
    if (n > this.pieces) { this.grow(n); coloursOnly = false; }
    let q = 0;
    for (const b of builds) {
      if (b.mat !== mat) continue;
      structureTint(b, _tint);
      if (b.kind === 2) { rampCornersInto(b, _c); q = this.hexa(q, _c, _tint, coloursOnly); }
      else for (const box of b.boxes) {
        const { minX: x0, minY: y0, minZ: z0, maxX: x1, maxY: y1, maxZ: z1 } = box;
        _c[0] = x0; _c[1] = y0; _c[2] = z0; _c[3] = x1; _c[4] = y0; _c[5] = z0; _c[6] = x1; _c[7] = y0; _c[8] = z1; _c[9] = x0; _c[10] = y0; _c[11] = z1;
        _c[12] = x0; _c[13] = y1; _c[14] = z0; _c[15] = x1; _c[16] = y1; _c[17] = z0; _c[18] = x1; _c[19] = y1; _c[20] = z1; _c[21] = x0; _c[22] = y1; _c[23] = z1;
        q = this.hexa(q, _c, _tint, coloursOnly);
      }
    }
    const g = this.geometry;
    const touch = (name: string, size: number) => {
      const a = g.getAttribute(name) as THREE.BufferAttribute;
      a.clearUpdateRanges(); a.addUpdateRange(0, q * 4 * size); a.needsUpdate = true;
    };
    touch('color', 3);
    if (!coloursOnly) { touch('position', 3); touch('normal', 3); touch('uv', 2); }
    if (!coloursOnly || q !== this.quads) g.setDrawRange(0, q * 6);
    this.quads = q;
    if (!coloursOnly) g.computeBoundingSphere();
    return n;
  }

  private grow(pieces: number): void {
    const cap = Math.max(16, this.pieces * 2, pieces), verts = cap * QUADS_PER_PIECE * 4;
    this.pieces = cap;
    this.pos = new Float32Array(verts * 3); this.nor = new Float32Array(verts * 3); this.uv = new Float32Array(verts * 2); this.col = new Float32Array(verts * 3);
    const idx = new Uint32Array(cap * QUADS_PER_PIECE * 6);
    for (let k = 0, v = 0; k < idx.length; k += 6, v += 4) { idx[k] = v; idx[k + 1] = v + 1; idx[k + 2] = v + 2; idx[k + 3] = v; idx[k + 4] = v + 2; idx[k + 5] = v + 3; }
    const g = this.geometry;
    // Frees the old GPU buffers; the geometry re-uploads the new ones on its next draw.
    if (g.getAttribute('position')) g.dispose();
    g.setAttribute('position', new THREE.BufferAttribute(this.pos, 3).setUsage(THREE.DynamicDrawUsage));
    g.setAttribute('normal', new THREE.BufferAttribute(this.nor, 3).setUsage(THREE.DynamicDrawUsage));
    g.setAttribute('uv', new THREE.BufferAttribute(this.uv, 2).setUsage(THREE.DynamicDrawUsage));
    g.setAttribute('color', new THREE.BufferAttribute(this.col, 3).setUsage(THREE.DynamicDrawUsage));
    g.setIndex(new THREE.BufferAttribute(idx, 1));
  }

  /** Writes a hexahedron (corner layout as FACES) from quad `q`; returns the next quad. */
  private hexa(q: number, c: Float32Array, color: THREE.Color, coloursOnly: boolean): number {
    for (let f = 0; f < 6; f++, q++) {
      const face = FACES[f], shade = f === 1 ? 1.06 : f === 0 ? 0.8 : 1;
      const r = Math.min(1, color.r * shade), gg = Math.min(1, color.g * shade), bb = Math.min(1, color.b * shade);
      let v = q * 4;
      for (let k = 0; k < 4; k++) { const o = (v + k) * 3; this.col[o] = r; this.col[o + 1] = gg; this.col[o + 2] = bb; }
      if (coloursOnly) continue;
      const a = face[0] * 3, b = face[1] * 3, d = face[2] * 3;
      const ax = c[b] - c[a], ay = c[b + 1] - c[a + 1], az = c[b + 2] - c[a + 2];
      const bx = c[d] - c[a], by = c[d + 1] - c[a + 1], bz = c[d + 2] - c[a + 2];
      let nx = ay * bz - az * by, ny = az * bx - ax * bz, nz = ax * by - ay * bx;
      const nl = Math.hypot(nx, ny, nz) || 1; nx /= nl; ny /= nl; nz /= nl;
      const anx = Math.abs(nx), any = Math.abs(ny), anz = Math.abs(nz);
      for (let k = 0; k < 4; k++, v++) {
        const i = face[k] * 3, x = c[i], y = c[i + 1], z = c[i + 2];
        this.pos[v * 3] = x; this.pos[v * 3 + 1] = y; this.pos[v * 3 + 2] = z;
        this.nor[v * 3] = nx; this.nor[v * 3 + 1] = ny; this.nor[v * 3 + 2] = nz;
        const u = any >= anx && any >= anz ? x : anx >= anz ? z : x, w = any >= anx && any >= anz ? z : y;
        this.uv[v * 2] = u / TILE; this.uv[v * 2 + 1] = w / TILE;
      }
    }
    return q;
  }

  dispose(): void { this.geometry.dispose(); }
}

/** A ramp's sloped slab as eight corners (x0z0, x1z0, x1z1, x0z1 bottom, then top) into `out`. */
function rampCornersInto(b: Build, out: Float32Array): void {
  const x0 = tileX(b.gx), z0 = tileX(b.gz), y = baseY(b.level), t = 0.22, cx = x0 + TILE / 2, cz = z0 + TILE / 2;
  for (let k = 0; k < 4; k++) {
    // (s up the slope, a across) for the four footprint corners.
    const sv = k === 2 || k === 3 ? TILE : 0, av = k === 1 || k === 2 ? TILE : 0;
    let x: number, z: number;
    switch (b.dir) {
      case 1: x = x0 + sv; z = z0 + av; break;
      case 2: x = x0 + av; z = z0 + TILE - sv; break;
      case 3: x = x0 + TILE - sv; z = z0 + av; break;
      default: x = x0 + av; z = z0 + sv;
    }
    const slot = x < cx ? (z < cz ? 0 : 3) : z < cz ? 1 : 2, top = y + (LEVEL * sv) / TILE;
    out[slot * 3] = x; out[slot * 3 + 1] = top - t; out[slot * 3 + 2] = z;
    out[12 + slot * 3] = x; out[12 + slot * 3 + 1] = top; out[12 + slot * 3 + 2] = z;
  }
}

function canvas(size: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement('canvas');
  c.width = c.height = size;
  return [c, c.getContext('2d')!];
}
/** Generated wood planks / brick courses / riveted metal panels for structures. */
export function materialTexture(mat: number): THREE.CanvasTexture {
  const [c, g] = canvas(128);
  const rng = new Rng(17 + mat);
  if (mat === 0) {
    g.fillStyle = '#c08a52'; g.fillRect(0, 0, 128, 128);
    for (let y = 0; y < 128; y += 16) {
      g.fillStyle = `rgba(${90 + rng.int(40)},${55 + rng.int(25)},${25 + rng.int(15)},0.35)`; g.fillRect(0, y, 128, 16);
      for (let i = 0; i < 18; i++) { g.fillStyle = 'rgba(70,40,15,0.18)'; g.fillRect(rng.int(128), y + 2 + rng.int(12), 10 + rng.int(30), 1); }
      g.fillStyle = 'rgba(50,28,10,0.6)'; g.fillRect(0, y, 128, 2);
      g.fillRect((y * 37) % 128, y, 2, 16);
    }
  } else if (mat === 1) {
    g.fillStyle = '#d9d0c0'; g.fillRect(0, 0, 128, 128);
    for (let row = 0; row < 8; row++) for (let col = -1; col < 4; col++) {
      const x = col * 40 + (row % 2) * 20, y = row * 16;
      g.fillStyle = `rgb(${160 + rng.int(40)},${70 + rng.int(25)},${55 + rng.int(20)})`;
      g.fillRect(x + 2, y + 2, 36, 12);
    }
  } else {
    g.fillStyle = '#9aa5ae'; g.fillRect(0, 0, 128, 128);
    for (let i = 0; i < 600; i++) { const v = rng.int(40); g.fillStyle = `rgba(${v},${v},${v + 10},0.08)`; g.fillRect(rng.int(128), rng.int(128), 2, 2); }
    g.strokeStyle = 'rgba(40,48,56,0.7)'; g.lineWidth = 3; g.strokeRect(2, 2, 124, 124); g.beginPath(); g.moveTo(64, 0); g.lineTo(64, 128); g.stroke();
    g.fillStyle = 'rgba(30,36,42,0.8)';
    for (const [x, y] of [[8, 8], [56, 8], [72, 8], [120, 8], [8, 120], [56, 120], [72, 120], [120, 120]]) { g.beginPath(); g.arc(x, y, 3, 0, Math.PI * 2); g.fill(); }
  }
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  t.wrapS = t.wrapT = THREE.RepeatWrapping;
  t.anisotropy = 4;
  return t;
}

/** Ghost preview geometries, centred: wall (along x), floor, ramp (rising toward +z). */
export function ghostGeometries(): THREE.BufferGeometry[] {
  return [new THREE.BoxGeometry(TILE, LEVEL, 0.3), new THREE.BoxGeometry(TILE, 0.25, TILE), new THREE.BoxGeometry(TILE, 0.25, Math.hypot(TILE, LEVEL))];
}
/** Places a ghost on a slot: position + rotation for kind / dir. */
export function placeGhost(o: THREE.Object3D, kind: number, gx: number, gz: number, level: number, dir: number): void {
  const x0 = tileX(gx), z0 = tileX(gz), y = baseY(level);
  if (kind === 0) {
    if (dir === 0) { o.position.set(x0 + TILE / 2, y + LEVEL / 2, z0 + TILE); o.rotation.set(0, 0, 0); }
    else { o.position.set(x0 + TILE, y + LEVEL / 2, z0 + TILE / 2); o.rotation.set(0, Math.PI / 2, 0); }
  } else if (kind === 1) { o.position.set(x0 + TILE / 2, y - 0.1, z0 + TILE / 2); o.rotation.set(0, 0, 0); }
  else { o.position.set(x0 + TILE / 2, y + LEVEL / 2, z0 + TILE / 2); o.rotation.set(-Math.atan2(LEVEL, TILE), [0, Math.PI / 2, Math.PI, -Math.PI / 2][dir] ?? 0, 0, 'YXZ'); }
}

/** A simple pickaxe: wooden handle and a steel head; forward is +z like the blasters. */
export function pickaxeMesh(handle: THREE.Material, head: THREE.Material, geos: THREE.BufferGeometry[]): THREE.Group {
  const g = new THREE.Group();
  const stick = new THREE.Mesh(geos[0], handle);
  stick.rotation.x = Math.PI / 2;
  const blade = new THREE.Mesh(geos[1], head);
  blade.position.z = 0.32;
  g.add(stick, blade);
  g.traverse(o => { o.castShadow = true; });
  return g;
}
export const pickaxeGeometries = (): THREE.BufferGeometry[] => [new THREE.CylinderGeometry(0.025, 0.03, 0.7, 6), new THREE.BoxGeometry(0.5, 0.07, 0.07)];

/** Launch pad: a squat metal drum (base) and a glowing spring disc (top), both unit-sized and centred on the floor. */
export const padGeometries = (): THREE.BufferGeometry[] => [
  new THREE.CylinderGeometry(1.15, 1.25, 0.28, 20).translate(0, 0.14, 0),
  new THREE.CylinderGeometry(0.95, 0.95, 0.08, 20).translate(0, 0.32, 0),
];
