/**
 * Procedural three.js geometry for Storm Royale: the walk-in architecture
 * (one merged, vertex-coloured mesh), player structures (one merged mesh per
 * material, rebuilt when they change), generated wood / brick / metal
 * textures, build-preview ghosts and the pickaxe. Everything here is made at
 * runtime; no image or model files.
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

/** The sloped slab of a ramp as eight corners. */
function rampCorners(b: Build): number[][] {
  const x0 = tileX(b.gx), z0 = tileX(b.gz), y = baseY(b.level), t = 0.22;
  // Local frame: s runs up the slope, across is the tile width; map to world per direction.
  const at = (s: number, a: number, h: number): number[] => {
    const yy = y + (LEVEL * s) / TILE + h;
    switch (b.dir) {
      case 1: return [x0 + s, yy, z0 + a];
      case 2: return [x0 + a, yy, z0 + TILE - s];
      case 3: return [x0 + TILE - s, yy, z0 + a];
      default: return [x0 + a, yy, z0 + s];
    }
  };
  const raw = [at(0, 0, -t), at(0, TILE, -t), at(TILE, TILE, -t), at(TILE, 0, -t), at(0, 0, 0), at(0, TILE, 0), at(TILE, TILE, 0), at(TILE, 0, 0)];
  // Reorder into the x0z0, x1z0, x1z1, x0z1 corner convention so faces wind outward.
  const order = (pts: number[][]) => {
    const cx = pts.reduce((a, p) => a + p[0], 0) / 4, cz = pts.reduce((a, p) => a + p[2], 0) / 4;
    const key = (p: number[]) => (p[0] < cx ? (p[2] < cz ? 0 : 3) : p[2] < cz ? 1 : 2);
    const out: number[][] = [];
    for (const p of pts) out[key(p)] = p;
    return out;
  };
  return [...order(raw.slice(0, 4)), ...order(raw.slice(4))];
}

/** Player structures of one material as a single mesh; pieces still going up glow blue, damaged ones darken. */
export function structureGeometry(builds: readonly Build[], mat: number): THREE.BufferGeometry | null {
  const w = new MeshWriter(1 / TILE);
  const base = new THREE.Color(1, 1, 1), growing = new THREE.Color(0.55, 0.8, 1.25), worn = new THREE.Color(0.45, 0.42, 0.4), c = new THREE.Color();
  for (const b of builds) {
    if (b.mat !== mat) continue;
    const k = Math.min(1, Math.max(0, b.hp / maxHp(b)));
    if (b.grow > 0) c.copy(base).lerp(growing, 0.6);
    else c.copy(base).lerp(worn, (1 - k) * 0.6);
    if (b.kind === 2) w.hexa(rampCorners(b), c);
    else for (const box of b.boxes) w.box(box, c);
  }
  return w.pos.length ? w.geometry() : null;
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
