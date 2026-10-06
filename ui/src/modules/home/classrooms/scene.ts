// Classrooms — the three.js half. Draws a `ClassroomModel` (model.ts) as a
// small low-poly campus: one floor tile + low walls + a back-wall board per
// workspace, a desk + screen + seated capsule figure per session, and the
// headmaster (the user) standing at the door of the current classroom.
//
// Conventions shared with the Design Hall 3D embed (studio3d/embed.ts):
//  • `three` + OrbitControls are lazy-loaded here, on first mount — the Home
//    chunk only carries the Svelte host (bundle-budget's lazy-only rule pins
//    it). Pixel ratio capped at 2; `dispose()` + `forceContextLoss()` on destroy.
//  • Render on demand: a frame is drawn only when something changed (data,
//    theme, camera, hover) or something animates. Nothing animates under
//    `prefers-reduced-motion`. The loop stops while the box is off screen
//    (IntersectionObserver), its Home space is inactive, or the tab is hidden.
//  • Colours are CSS tokens resolved by the host (lib/cssColor.ts) and pushed
//    in with `setColors` — again on every theme change; no literals.
//
// Scale (~20 rooms × ~30 students): every repeated part is ONE InstancedMesh
// (per-instance matrix + colour) over shared geometries/materials, so the
// draw-call count is constant, whatever the number of sessions.

import type * as THREE_NS from 'three';
import type { Rgba } from '../../../lib/cssColor';
import { alongPath, doorPosition, exitPath, type Classroom, type ClassroomModel, type Student } from './model';

type Three = typeof THREE_NS;

/** Tokens the scene reads (the host resolves them). */
export const SCENE_TOKENS = [
  '--surface',
  '--surface-2',
  '--surface-3',
  '--bg',
  '--border-strong',
  '--text',
  '--text-dim',
  '--accent',
  '--warning',
  '--success',
  '--cat-1',
  '--cat-2',
  '--cat-3',
  '--cat-4',
  '--cat-5',
  '--cat-6',
] as const;
export type SceneToken = (typeof SCENE_TOKENS)[number];
export type SceneColors = Record<SceneToken, Rgba | null>;

export interface ScreenPoint {
  /** Client (viewport) coordinates. */
  x: number;
  y: number;
  /** In front of the camera and inside the canvas. */
  visible: boolean;
}

export interface SceneHandle {
  update(model: ClassroomModel): void;
  setColors(colors: SceneColors): void;
  setHighlight(id: string | null): void;
  /** Screen point above a student's head / a room's back-wall board. */
  projectStudent(id: string): ScreenPoint | null;
  projectRoom(id: string): ScreenPoint | null;
  projectHeadmaster(): ScreenPoint | null;
  /** Student id under a client point, or null. */
  pick(clientX: number, clientY: number): string | null;
  /** Stand up, walk out of the door, vanish. Resolves when gone. */
  kickOut(id: string): Promise<void>;
  /** Bring a kicked-out student back (the delete failed). */
  restore(id: string): void;
  resetView(): void;
  /** Draw one frame (e.g. after the host mounted new HTML labels). */
  redraw(): void;
  /** False pauses the loop (inactive Home space / hidden tab). */
  setActive(on: boolean): void;
  /** Called after every rendered frame (the host repositions HTML labels). */
  onFrame(fn: () => void): void;
  destroy(): void;
}

export interface SceneOptions {
  reducedMotion: boolean;
  /** Accessible name for the canvas. */
  label: string;
}

/** Whether this browser can create a WebGL context at all. */
export function webglAvailable(): boolean {
  try {
    const c = document.createElement('canvas');
    return !!(c.getContext('webgl2') ?? c.getContext('webgl'));
  } catch {
    return false;
  }
}

// Figure dimensions (seated).
const BODY_R = 0.2;
const BODY_LEN = 0.34;
const BODY_Y = 0.66;
const HEAD_R = 0.16;
const HEAD_Y = 1.16;
const DESK_W = 0.95;
const DESK_D = 0.5;
const DESK_H = 0.62;
const DESK_DZ = 0.46; // desk sits in front (+z) of the student
const WALL_H = 0.45;
const WALL_T = 0.08;
const KICK_MS = 1400;

interface Slot {
  student: Student;
  room: Classroom;
  x: number;
  z: number;
  phase: number;
}

interface Kick {
  start: number;
  path: { x: number; z: number }[];
  resolve: () => void;
}

export async function mountClassroomScene(host: HTMLElement, opts: SceneOptions): Promise<SceneHandle> {
  const [T, orbitMod] = (await Promise.all([import('three'), import('three/examples/jsm/controls/OrbitControls.js')])) as [
    Three,
    typeof import('three/examples/jsm/controls/OrbitControls.js'),
  ];
  const reduced = opts.reducedMotion;

  const renderer = new T.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.outputColorSpace = T.SRGBColorSpace;
  renderer.setClearColor(0x000000, 0);
  const canvas = renderer.domElement;
  // Touch: a one-finger VERTICAL swipe still scrolls the Home page (a widget
  // must not trap the page on a phone); horizontal drags orbit, pinch zooms.
  const coarse = !!window.matchMedia?.('(pointer: coarse)').matches;
  Object.assign(canvas.style, { display: 'block', width: '100%', height: '100%', touchAction: coarse ? 'pan-y' : 'none' });
  canvas.setAttribute('role', 'img');
  canvas.setAttribute('aria-label', opts.label);
  host.appendChild(canvas);

  const scene = new T.Scene();
  const camera = new T.PerspectiveCamera(38, 1, 0.1, 600);
  scene.add(new T.HemisphereLight(0xffffff, 0x808080, 1.6));
  const sun = new T.DirectionalLight(0xffffff, 1.4);
  sun.position.set(-8, 18, 12);
  scene.add(sun);

  // ── Shared geometry + materials ──────────────────────────────────────────
  const unitBox = new T.BoxGeometry(1, 1, 1);
  const bodyGeo = new T.CapsuleGeometry(BODY_R, BODY_LEN, 4, 10);
  const headGeo = new T.SphereGeometry(HEAD_R, 16, 12);
  const armGeo = new T.CapsuleGeometry(0.05, 0.36, 3, 6);
  const markerGeo = new T.SphereGeometry(0.09, 12, 8);
  const solid = new T.MeshStandardMaterial({ roughness: 0.85, metalness: 0 });
  const ghost = new T.MeshStandardMaterial({ roughness: 0.85, metalness: 0, transparent: true, opacity: 0.28, depthWrite: false });
  const glow = new T.MeshBasicMaterial();
  const geometries = [unitBox, bodyGeo, headGeo, armGeo, markerGeo];
  const materials: THREE_NS.Material[] = [solid, ghost, glow];

  // ── Instanced parts ──────────────────────────────────────────────────────
  type PartKey = 'floor' | 'wall' | 'board' | 'stripe' | 'desk' | 'screen' | 'body' | 'head' | 'bodyGhost' | 'headGhost' | 'arm' | 'marker';
  const PART: Record<PartKey, { geo: THREE_NS.BufferGeometry; mat: THREE_NS.Material; pick?: boolean }> = {
    floor: { geo: unitBox, mat: solid },
    wall: { geo: unitBox, mat: solid },
    board: { geo: unitBox, mat: solid },
    stripe: { geo: unitBox, mat: solid },
    desk: { geo: unitBox, mat: solid, pick: true },
    screen: { geo: unitBox, mat: glow },
    body: { geo: bodyGeo, mat: solid, pick: true },
    head: { geo: headGeo, mat: solid, pick: true },
    bodyGhost: { geo: bodyGeo, mat: ghost, pick: true },
    headGhost: { geo: headGeo, mat: ghost, pick: true },
    arm: { geo: armGeo, mat: solid },
    marker: { geo: markerGeo, mat: glow },
  };
  const meshes = new Map<PartKey, THREE_NS.InstancedMesh>();
  /** Instance index → student id, per pickable part. */
  const owners = new Map<PartKey, string[]>();
  function part(key: PartKey, count: number): THREE_NS.InstancedMesh {
    let m = meshes.get(key);
    if (!m || m.instanceMatrix.count < count) {
      if (m) {
        scene.remove(m);
        m.dispose();
      }
      const cap = Math.max(8, 2 ** Math.ceil(Math.log2(Math.max(1, count))));
      m = new T.InstancedMesh(PART[key].geo, PART[key].mat, cap);
      m.instanceMatrix.setUsage(T.DynamicDrawUsage);
      m.frustumCulled = false;
      scene.add(m);
      meshes.set(key, m);
    }
    m.count = count;
    return m;
  }

  // Headmaster: one small group (accent robe, head, mortarboard).
  const headmaster = new T.Group();
  const hmMat = new T.MeshStandardMaterial({ roughness: 0.7 });
  const hmHeadMat = new T.MeshStandardMaterial({ roughness: 0.8 });
  const hmCapMat = new T.MeshStandardMaterial({ roughness: 0.6 });
  materials.push(hmMat, hmHeadMat, hmCapMat);
  const hmBody = new T.Mesh(bodyGeo, hmMat);
  hmBody.scale.set(1.25, 1.6, 1.25);
  hmBody.position.y = 0.72;
  const hmHead = new T.Mesh(headGeo, hmHeadMat);
  hmHead.scale.setScalar(1.25);
  hmHead.position.y = 1.5;
  const hmCap = new T.Mesh(unitBox, hmCapMat);
  hmCap.scale.set(0.5, 0.04, 0.5);
  hmCap.position.y = 1.72;
  hmCap.rotation.y = Math.PI / 4;
  const hmCapBase = new T.Mesh(unitBox, hmCapMat);
  hmCapBase.scale.set(0.26, 0.1, 0.26);
  hmCapBase.position.y = 1.66;
  headmaster.add(hmBody, hmHead, hmCap, hmCapBase);
  scene.add(headmaster);

  // ── State ────────────────────────────────────────────────────────────────
  let model: ClassroomModel | null = null;
  let colors: SceneColors | null = null;
  let highlight: string | null = null;
  const slots = new Map<string, Slot>();
  const kicks = new Map<string, Kick>();
  /** Kicked out and gone (until the next model drops them, or restore). */
  const gone = new Set<string>();
  let destroyed = false;
  let active = true;
  let onScreen = true;
  let pageVisible = document.visibilityState !== 'hidden';
  let needsFrame = true;
  let interactUntil = 0;
  let raf = 0;
  let lastAmbient = 0;
  let frameFn: (() => void) | null = null;
  const tmp = new T.Object3D();
  const col = new T.Color();

  // ── Colours ──────────────────────────────────────────────────────────────
  const fallback = { r: 0.5, g: 0.5, b: 0.5, a: 1 };
  /** Token → THREE.Color, alpha flattened onto the box surface. */
  function tc(name: SceneToken, out = new T.Color()): THREE_NS.Color {
    const c = colors?.[name] ?? fallback;
    const bg = colors?.['--surface'] ?? fallback;
    const a = c.a;
    return out.setRGB(c.r * a + bg.r * (1 - a), c.g * a + bg.g * (1 - a), c.b * a + bg.b * (1 - a), T.SRGBColorSpace);
  }
  function mix(a: THREE_NS.Color, b: THREE_NS.Color, k: number): THREE_NS.Color {
    return a.clone().lerp(b, k);
  }

  // ── Build ────────────────────────────────────────────────────────────────
  function setInst(m: THREE_NS.InstancedMesh, i: number, x: number, y: number, z: number, sx: number, sy: number, sz: number, ry = 0): void {
    tmp.position.set(x, y, z);
    tmp.rotation.set(0, ry, 0);
    tmp.scale.set(sx, sy, sz);
    tmp.updateMatrix();
    m.setMatrixAt(i, tmp.matrix);
  }

  function build(): void {
    if (!model) return;
    slots.clear();
    const rooms = model.rooms;
    // Rooms: floor, 4 walls (front split around the door → 5 pieces), board, back-row stripe.
    const floor = part('floor', rooms.length);
    const wall = part('wall', rooms.length * 5);
    const board = part('board', rooms.length);
    const withBack = rooms.filter((r) => r.backRow.length > 0);
    const stripe = part('stripe', withBack.length);
    // The stage behind the campus is --surface-2: floors are a step darker/
    // lighter (--surface-3) so every room reads as a tile on it.
    const floorC = tc('--surface-3');
    const boardC = mix(tc('--text-dim'), tc('--surface'), 0.35);
    const accent = tc('--accent');
    const wallC = tc('--border-strong');
    const text = tc('--text-dim');
    rooms.forEach((r, i) => {
      setInst(floor, i, r.x, -0.03, r.z, r.width, 0.06, r.depth);
      floor.setColorAt(i, r.current ? mix(floorC, accent, 0.16) : floorC);
      const hw = r.width / 2;
      const hd = r.depth / 2;
      const wc = r.current ? mix(wallC, accent, 0.55) : wallC;
      let w = i * 5;
      setInst(wall, w++, r.x, WALL_H / 2, r.z - hd, r.width, WALL_H, WALL_T); // back
      setInst(wall, w++, r.x - hw, WALL_H / 2, r.z, WALL_T, WALL_H, r.depth); // start side
      setInst(wall, w++, r.x + hw, WALL_H / 2, r.z, WALL_T, WALL_H, r.depth); // end side
      // Front wall with a 1.2-unit door gap near the end side.
      const door = doorPosition(r);
      const leftLen = door.x - 0.6 - (r.x - hw);
      const rightLen = r.x + hw - (door.x + 0.6);
      setInst(wall, w++, r.x - hw + leftLen / 2, WALL_H / 2, r.z + hd, Math.max(0.01, leftLen), WALL_H, WALL_T);
      setInst(wall, w++, r.x + hw - rightLen / 2, WALL_H / 2, r.z + hd, Math.max(0.01, rightLen), WALL_H, WALL_T);
      for (let k = i * 5; k < i * 5 + 5; k++) wall.setColorAt(k, wc);
      // The board on the back wall (the room name floats above it as HTML).
      setInst(board, i, r.x, 0.95, r.z - hd + 0.06, Math.min(r.width - 1, 3.2), 0.7, 0.04);
      board.setColorAt(i, r.current ? mix(boardC, accent, 0.3) : boardC);
    });
    withBack.forEach((r, i) => {
      const z = r.z + r.backRow[0].seat.z + SEAT_GAP_HALF;
      setInst(stripe, i, r.x, 0.005, z, r.width - 0.4, 0.01, 0.06);
      stripe.setColorAt(i, text);
    });

    // Students.
    const all: { s: Student; r: Classroom }[] = [];
    for (const r of rooms) for (const s of [...r.students, ...r.backRow]) all.push({ s, r });
    const desk = part('desk', all.length);
    const screen = part('screen', all.length);
    const solidN = all.filter((a) => a.s.visual !== 'away').length;
    part('body', solidN);
    part('head', solidN);
    part('bodyGhost', all.length - solidN);
    part('headGhost', all.length - solidN);
    const hands = all.filter((a) => a.s.visual === 'needs-you');
    part('arm', hands.length);
    part('marker', hands.length);
    const deskOwners: string[] = [];
    const bodyOwners: string[] = [];
    const ghostOwners: string[] = [];
    all.forEach(({ s, r }, i) => {
      const x = r.x + s.seat.x;
      const z = r.z + s.seat.z;
      slots.set(s.id, { student: s, room: r, x, z, phase: (i * 0.618) % 1 });
      setInst(desk, i, x, DESK_H / 2, z + DESK_DZ, DESK_W, DESK_H, DESK_D);
      deskOwners.push(s.id);
      setInst(screen, i, x, DESK_H + 0.17, z + DESK_DZ + 0.1, 0.42, 0.26, 0.03);
    });
    owners.set('desk', deskOwners);
    let bi = 0;
    let gi = 0;
    for (const { s } of all) {
      if (s.visual === 'away') ghostOwners[gi++] = s.id;
      else bodyOwners[bi++] = s.id;
    }
    owners.set('body', bodyOwners);
    owners.set('head', bodyOwners);
    owners.set('bodyGhost', ghostOwners);
    owners.set('headGhost', ghostOwners);
    owners.set('arm', hands.map((h) => h.s.id));
    owners.set('marker', hands.map((h) => h.s.id));
    paintStudents();
    pose(performance.now());
    placeHeadmaster();
    for (const m of meshes.values()) {
      m.instanceMatrix.needsUpdate = true;
      if (m.instanceColor) m.instanceColor.needsUpdate = true;
      m.computeBoundingSphere();
    }
    needsFrame = true;
  }

  /** Per-student colours (theme, highlight, status). */
  function paintStudents(): void {
    if (!model) return;
    const desk = meshes.get('desk');
    const screen = meshes.get('screen');
    if (!desk || !screen) return;
    const deskC = mix(tc('--text-dim'), tc('--surface-3'), 0.55);
    const deskBack = mix(tc('--text-dim'), tc('--surface-3'), 0.75);
    const accent = tc('--accent');
    const on = tc('--success');
    const off = mix(tc('--surface-3'), tc('--text-dim'), 0.35);
    const headC = mix(tc('--text'), tc('--surface'), 0.3);
    const warn = tc('--warning');
    const deskOwners = owners.get('desk') ?? [];
    deskOwners.forEach((id, i) => {
      const s = slots.get(id)!.student;
      const hl = id === highlight;
      desk.setColorAt(i, hl ? mix(deskC, accent, 0.45) : s.background ? deskBack : deskC);
      const lit = !gone.has(id) && !kicks.has(id) && (s.visual === 'working' || s.visual === 'needs-you');
      screen.setColorAt(i, lit ? (s.visual === 'needs-you' ? warn : on) : s.visual === 'idle' ? mix(off, on, 0.25) : off);
    });
    for (const [bk, hk] of [
      ['body', 'head'],
      ['bodyGhost', 'headGhost'],
    ] as const) {
      const b = meshes.get(bk);
      const h = meshes.get(hk);
      (owners.get(bk) ?? []).forEach((id, i) => {
        const s = slots.get(id)!.student;
        let c = tc(s.colorToken as SceneToken, col.clone());
        if (s.visual === 'idle' || s.visual === 'stale') c = mix(c, tc('--surface'), 0.35);
        if (id === highlight) c = mix(c, tc('--text'), 0.25);
        b?.setColorAt(i, c);
        h?.setColorAt(i, id === highlight ? mix(headC, accent, 0.4) : headC);
      });
    }
    const arm = meshes.get('arm');
    const marker = meshes.get('marker');
    (owners.get('arm') ?? []).forEach((id, i) => {
      const s = slots.get(id)!.student;
      arm?.setColorAt(i, tc(s.colorToken as SceneToken, col.clone()));
      marker?.setColorAt(i, warn);
    });
    for (const m of meshes.values()) if (m.instanceColor) m.instanceColor.needsUpdate = true;
    hmMat.color.copy(tc('--accent'));
    hmHeadMat.color.copy(headC);
    hmCapMat.color.copy(tc('--text'));
    needsFrame = true;
  }

  function placeHeadmaster(): void {
    const room = model?.rooms.find((r) => r.current) ?? model?.rooms[0];
    headmaster.visible = !!room;
    if (!room) return;
    const door = doorPosition(room);
    headmaster.position.set(door.x - 0.2, 0, door.z - 0.8);
    headmaster.rotation.y = Math.PI; // faces the class
  }

  /** Figure transforms for `now` (bobbing, raised hands, kick-outs). */
  function pose(now: number): boolean {
    let moving = false;
    const t = now / 1000;
    for (const [bk, hk] of [
      ['body', 'head'],
      ['bodyGhost', 'headGhost'],
    ] as const) {
      const b = meshes.get(bk);
      const h = meshes.get(hk);
      if (!b || !h) continue;
      (owners.get(bk) ?? []).forEach((id, i) => {
        const sl = slots.get(id)!;
        let x = sl.x;
        let z = sl.z;
        let lift = 0;
        let bob = 0;
        let nod = 0;
        let scale = 1;
        let face = 0;
        const k = kicks.get(id);
        if (gone.has(id)) {
          scale = 0;
        } else if (k) {
          const p = Math.min(1, (now - k.start) / KICK_MS);
          const pos = alongPath(k.path, p);
          x = pos.x;
          z = pos.z;
          lift = Math.min(1, p * 5) * 0.3; // stands up first
          bob = Math.abs(Math.sin(p * 28)) * 0.05; // steps
          scale = p > 0.82 ? Math.max(0, 1 - (p - 0.82) / 0.18) : 1;
          face = Math.atan2(k.path[1].x - sl.x, k.path[1].z - sl.z);
          moving = true;
          if (p >= 1) {
            kicks.delete(id);
            gone.add(id);
            k.resolve();
            paintStudents();
          }
        } else if (!reduced && sl.student.visual === 'working') {
          bob = Math.max(0, Math.sin((t + sl.phase) * 7)) * 0.035;
          nod = Math.sin((t + sl.phase) * 3.5) * 0.04;
          moving = true;
        }
        const back = sl.student.background ? 0.88 : 1;
        tmp.position.set(x, BODY_Y + lift + bob, z);
        tmp.rotation.set(0, face, 0);
        tmp.scale.setScalar(scale * back);
        tmp.updateMatrix();
        b.setMatrixAt(i, tmp.matrix);
        tmp.position.set(x, (HEAD_Y - BODY_Y) * back + BODY_Y + lift + bob * 1.4, z + nod);
        tmp.updateMatrix();
        h.setMatrixAt(i, tmp.matrix);
      });
      b.instanceMatrix.needsUpdate = true;
      h.instanceMatrix.needsUpdate = true;
    }
    const arm = meshes.get('arm');
    const marker = meshes.get('marker');
    if (arm && marker) {
      (owners.get('arm') ?? []).forEach((id, i) => {
        const sl = slots.get(id)!;
        const hidden = gone.has(id) || kicks.has(id);
        const pulse = reduced ? 1 : 1 + Math.sin(t * 5 + sl.phase * 6) * 0.22;
        if (!reduced) moving = true;
        setInst(arm, i, sl.x + 0.24, BODY_Y + 0.62, sl.z, hidden ? 0 : 1, hidden ? 0 : 1, hidden ? 0 : 1);
        const s = hidden ? 0 : pulse;
        setInst(marker, i, sl.x + 0.26, BODY_Y + 1.02, sl.z, s, s, s);
      });
      arm.instanceMatrix.needsUpdate = true;
      marker.instanceMatrix.needsUpdate = true;
    }
    return moving;
  }

  // ── Camera ───────────────────────────────────────────────────────────────
  const controls = new orbitMod.OrbitControls(camera, canvas);
  controls.enableDamping = !reduced;
  controls.dampingFactor = 0.12;
  controls.screenSpacePanning = false;
  controls.minPolarAngle = 0.25;
  controls.maxPolarAngle = 1.2;
  controls.minDistance = 4;
  controls.maxDistance = 120;
  controls.addEventListener('change', () => {
    clampTarget();
    needsFrame = true;
    wake();
  });
  controls.addEventListener('start', () => {
    interactUntil = Infinity;
    wake();
  });
  controls.addEventListener('end', () => {
    interactUntil = performance.now() + 900;
  });

  function fitDistance(): number {
    const w = Math.max(8, model?.width ?? 10);
    const d = Math.max(8, model?.depth ?? 10);
    const aspect = Math.max(0.4, camera.aspect || 1);
    const vfov = (camera.fov * Math.PI) / 180;
    const byW = w / 2 / Math.tan(Math.atan(Math.tan(vfov / 2) * aspect));
    const byD = d / 2 / Math.tan(vfov / 2);
    return Math.max(byW, byD) * 1.15 + 3;
  }
  function clampTarget(): void {
    const hw = (model?.width ?? 10) / 2;
    const hd = (model?.depth ?? 10) / 2;
    const t = controls.target;
    const cx = Math.min(hw, Math.max(-hw, t.x));
    const cz = Math.min(hd, Math.max(-hd, t.z));
    if (cx !== t.x || cz !== t.z || t.y !== 0) {
      camera.position.x += cx - t.x;
      camera.position.z += cz - t.z;
      t.set(cx, 0, cz);
    }
  }
  let viewed = false;
  function resetView(): void {
    const dist = fitDistance();
    controls.maxDistance = Math.max(20, dist * 2);
    const polar = 0.82; // ≈ 47° from vertical: an isometric-ish overview
    controls.target.set(0, 0, 0);
    camera.position.set(0, Math.cos(polar) * dist, Math.sin(polar) * dist);
    camera.lookAt(0, 0, 0);
    controls.update();
    needsFrame = true;
    wake();
  }

  // ── Projection + picking ─────────────────────────────────────────────────
  const v = new T.Vector3();
  function project(x: number, y: number, z: number): ScreenPoint {
    const rect = canvas.getBoundingClientRect();
    v.set(x, y, z).project(camera);
    const sx = rect.left + ((v.x + 1) / 2) * rect.width;
    const sy = rect.top + ((1 - v.y) / 2) * rect.height;
    const visible = v.z < 1 && sx >= rect.left && sx <= rect.right && sy >= rect.top && sy <= rect.bottom;
    return { x: sx, y: sy, visible };
  }
  const ray = new T.Raycaster();
  const ndc = new T.Vector2();
  function pick(clientX: number, clientY: number): string | null {
    const rect = canvas.getBoundingClientRect();
    ndc.set(((clientX - rect.left) / rect.width) * 2 - 1, -((clientY - rect.top) / rect.height) * 2 + 1);
    ray.setFromCamera(ndc, camera);
    const targets = [...meshes.entries()].filter(([k, m]) => PART[k].pick && m.count > 0).map(([, m]) => m);
    const hit = ray.intersectObjects(targets, false).find((h) => h.instanceId !== undefined);
    if (!hit) return null;
    const key = [...meshes.entries()].find(([, m]) => m === hit.object)?.[0];
    const id = key ? owners.get(key)?.[hit.instanceId!] : undefined;
    return id && !gone.has(id) ? id : null;
  }

  // ── Loop ─────────────────────────────────────────────────────────────────
  function running(): boolean {
    return !destroyed && active && onScreen && pageVisible;
  }
  function wake(): void {
    if (!raf && running()) raf = requestAnimationFrame(frame);
  }
  function frame(now: number): void {
    raf = 0;
    if (!running()) return;
    const interacting = now < interactUntil;
    if (controls.enableDamping || interacting) controls.update();
    // Ambient motion (typing, raised-hand pulse) at ~30 fps; kick-outs and
    // camera moves at full rate.
    const ambientDue = now - lastAmbient > 33 || kicks.size > 0;
    let moving = false;
    if (ambientDue) {
      moving = pose(now);
      if (moving) {
        lastAmbient = now;
        needsFrame = true;
      }
    } else {
      moving = true;
    }
    if (needsFrame) {
      needsFrame = false;
      renderer.render(scene, camera);
      frameFn?.();
    }
    // `controls.update()` may already have re-armed the loop (its change event).
    if (!raf && (moving || interacting || needsFrame)) raf = requestAnimationFrame(frame);
  }

  const resize = (): void => {
    const w = Math.max(1, host.clientWidth);
    const h = Math.max(1, host.clientHeight);
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    if (!viewed && model) {
      viewed = true;
      resetView();
    }
    needsFrame = true;
    wake();
  };
  const ro = new ResizeObserver(resize);
  ro.observe(host);
  const io = new IntersectionObserver((es) => {
    onScreen = es.some((e) => e.isIntersecting);
    if (onScreen) {
      needsFrame = true;
      wake();
    }
  });
  io.observe(host);
  const onVis = (): void => {
    pageVisible = document.visibilityState !== 'hidden';
    if (pageVisible) {
      needsFrame = true;
      wake();
    }
  };
  document.addEventListener('visibilitychange', onVis);
  resize();

  function headTop(id: string): { x: number; y: number; z: number } | null {
    const sl = slots.get(id);
    if (!sl || gone.has(id)) return null;
    return { x: sl.x, y: HEAD_Y + HEAD_R + 0.15, z: sl.z };
  }

  return {
    update(m) {
      const first = !model;
      model = m;
      // A student the new model no longer holds is truly gone.
      const ids = new Set(m.students.map((s) => s.id));
      for (const id of [...gone]) if (!ids.has(id)) gone.delete(id);
      build();
      if (first || !viewed) {
        viewed = true;
        resetView();
      }
      wake();
    },
    setColors(c) {
      colors = c;
      if (model) {
        build();
        wake();
      }
    },
    setHighlight(id) {
      if (id === highlight) return;
      highlight = id;
      paintStudents();
      wake();
    },
    projectStudent(id) {
      const p = headTop(id);
      return p ? project(p.x, p.y, p.z) : null;
    },
    projectRoom(id) {
      const r = model?.rooms.find((x) => x.id === id);
      return r ? project(r.x, 1.5, r.z - r.depth / 2) : null;
    },
    projectHeadmaster() {
      return headmaster.visible ? project(headmaster.position.x, 2.05, headmaster.position.z) : null;
    },
    pick,
    kickOut(id) {
      const sl = slots.get(id);
      if (!sl || gone.has(id)) return Promise.resolve();
      if (reduced) {
        gone.add(id);
        paintStudents();
        pose(performance.now());
        needsFrame = true;
        wake();
        return Promise.resolve();
      }
      return new Promise<void>((resolve) => {
        kicks.set(id, { start: performance.now(), path: exitPath({ x: sl.x, z: sl.z }, doorPosition(sl.room)), resolve });
        paintStudents();
        wake();
      });
    },
    restore(id) {
      const k = kicks.get(id);
      kicks.delete(id);
      k?.resolve();
      gone.delete(id);
      paintStudents();
      pose(performance.now());
      needsFrame = true;
      wake();
    },
    resetView,
    redraw() {
      needsFrame = true;
      wake();
    },
    setActive(on) {
      active = on;
      if (on) {
        needsFrame = true;
        wake();
      }
    },
    onFrame(fn) {
      frameFn = fn;
    },
    destroy() {
      destroyed = true;
      cancelAnimationFrame(raf);
      for (const k of kicks.values()) k.resolve();
      kicks.clear();
      ro.disconnect();
      io.disconnect();
      document.removeEventListener('visibilitychange', onVis);
      controls.dispose();
      for (const m of meshes.values()) m.dispose();
      meshes.clear();
      for (const g of geometries) g.dispose();
      for (const m of materials) m.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      canvas.remove();
    },
  };
}

/** Half the desk pitch: the back-row stripe sits between the last front row
 *  and the first back row. */
const SEAT_GAP_HALF = 0.8;
