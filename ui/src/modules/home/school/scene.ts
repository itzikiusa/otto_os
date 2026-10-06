// Otto School — the three.js half. Draws a `School` (model.ts) from the
// Blender-built kit (assets.ts): a corridor with one door per workspace, and
// the classroom you walk into — floor, walls, windows, board, desks with PCs
// whose monitors show each session's live terminal (screens.ts), the kids
// (one character per provider) and the robot headmaster, all moved by the
// ambient-life director (life.ts).
//
//  • `three`, its loaders and the .glb files load on first mount only — the
//    Home chunk carries the Svelte host alone (bundle-budget's lazy-only rule).
//  • Static kit pieces are InstancedMeshes (one per kit mesh), so a 40-desk
//    room costs a handful of draw calls; only the pieces the runtime paints or
//    moves (screens, board title, door leaves, door signs) are per-instance.
//  • Only the room you are in is built. The loop runs while the box is on
//    screen and active, at ≤ 30 fps; the corridor renders on demand.
//  • Reduced motion: camera moves cut instead of flying; life.ts keeps
//    everybody seated.
//  • `pixelRatio` ≤ 2; `dispose()` + `forceContextLoss()` on destroy; a lost
//    context is reported so the host can offer Retry.

import type * as THREE_NS from 'three';
import type { GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { assetBase, loadSchoolAssets, type SchoolAssets } from './assets.ts';
import { KIT_NODES, furnishCorridor, furnishRoom, type KitNode, type Placement } from './furnish.ts';
import { HEAD_CLIPS, Life, ONCE_CLIPS, SIT_MS, STAND_MS, NOD_MS, SCOLD_MS, type ActorView } from './life.ts';
import { CORRIDOR_HALF, ROOM_BACK_Z, roomSummary, toWorld, type Kid, type Room, type School } from './model.ts';
import { SCREEN_H, SCREEN_W, paintScreen, screenKey, type ScreenInfo } from './screens.ts';

type Three = typeof THREE_NS;
type Obj = THREE_NS.Object3D;

export type SchoolView = { kind: 'corridor' } | { kind: 'room'; roomId: string } | { kind: 'screen'; roomId: string; kidId: string };
export type SchoolPick = { kind: 'kid'; id: string } | { kind: 'door'; id: string } | { kind: 'head'; id: 'headmaster' };

export interface ScreenPoint {
  x: number;
  y: number;
  visible: boolean;
}

export interface SchoolDebug {
  view: SchoolView;
  frames: number;
  missing: string[];
  kids: { id: string; clip: string; mode: string; visible: boolean; x: number; z: number }[];
  head: { clip: string; mode: string; x: number; z: number; target: string | null } | null;
  camera: { x: number; y: number; z: number };
  screens: Record<string, string>;
}

export interface SchoolHandle {
  update(school: School): void;
  view(): SchoolView;
  onView(fn: (v: SchoolView) => void): void;
  enterRoom(id: string): Promise<void>;
  toCorridor(): Promise<void>;
  lookAtScreen(kidId: string): Promise<void>;
  /** Screen view → back to the room overview. */
  backToRoom(): Promise<void>;
  pick(clientX: number, clientY: number): SchoolPick | null;
  setHover(p: SchoolPick | null): void;
  setSelected(kidId: string | null): void;
  project(p: SchoolPick): ScreenPoint | null;
  /** What a kid's monitor shows (the host feeds the live screen). */
  setScreen(kidId: string, info: ScreenInfo): void;
  /** Kids in the open room whose monitor the camera can see, nearest first. */
  visibleScreens(): string[];
  /** The door the corridor camera faces (Enter walks in). */
  facingDoor(): string | null;
  kickOut(id: string): Promise<void>;
  detention(id: string): Promise<void>;
  restore(id: string): void;
  inspect(id: string): void;
  /** Keyboard (walk / orbit). Returns true when handled. */
  key(e: KeyboardEvent, down: boolean): boolean;
  drag(dx: number, dy: number): void;
  wheel(dy: number): void;
  resetView(): void;
  setActive(on: boolean): void;
  setTheme(dark: boolean): void;
  onFrame(fn: () => void): void;
  onContextLost(fn: () => void): void;
  debug(): SchoolDebug;
  destroy(): void;
}

export interface MountOpts {
  reducedMotion: boolean;
  label: string;
  dark: boolean;
}

/** WebGL available at all (cheap probe, once). */
let glProbe: boolean | null = null;
export function webglAvailable(): boolean {
  if (glProbe !== null) return glProbe;
  try {
    const c = document.createElement('canvas');
    const gl = c.getContext('webgl2') ?? c.getContext('webgl');
    glProbe = !!gl;
    (gl as WebGLRenderingContext | null)?.getExtension('WEBGL_lose_context')?.loseContext();
  } catch {
    glProbe = false;
  }
  return glProbe;
}

/** Nodes the runtime paints or moves (never instanced). */
const LIVE_CHILDREN = new Set(['Screen', 'BoardTitle', 'NamePlate', 'DoorSign', 'DoorLeaf']);
const EYE = 1.62;
const FPS_MS = 1000 / 30;
const ease = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

interface MeshDesc {
  geometry: THREE_NS.BufferGeometry;
  material: THREE_NS.Material | THREE_NS.Material[];
  rel: THREE_NS.Matrix4;
}

interface Painted {
  mesh: THREE_NS.Mesh;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  tex: THREE_NS.CanvasTexture;
  key: string;
}

interface Character {
  id: string;
  obj: Obj;
  mixer: THREE_NS.AnimationMixer;
  actions: Map<string, THREE_NS.AnimationAction>;
  clip: string;
  hit: THREE_NS.Mesh;
  heading: number;
}

interface Door {
  roomId: string;
  pivots: { obj: Obj; base: THREE_NS.Matrix4 }[];
  angle: number;
  target: number;
  hit: THREE_NS.Mesh | null;
  plate: Painted | null;
  sign: Painted | null;
}

export async function mountSchool(host: HTMLElement, opts: MountOpts): Promise<SchoolHandle> {
  const [T, { GLTFLoader }, SkeletonUtils, { RoomEnvironment }, { HDRLoader }] = await Promise.all([
    import('three'),
    import('three/examples/jsm/loaders/GLTFLoader.js'),
    import('three/examples/jsm/utils/SkeletonUtils.js'),
    import('three/examples/jsm/environments/RoomEnvironment.js'),
    import('three/examples/jsm/loaders/HDRLoader.js'),
  ]);
  const [assets, hdr] = await Promise.all([
    loadSchoolAssets(T, GLTFLoader),
    // Optional CC0 HDRI (Poly Haven) for the light; the procedural room
    // environment stands in when the kit ships none.
    new HDRLoader().loadAsync(`${assetBase()}env.hdr`).catch(() => null),
  ]);
  return build(T, assets, SkeletonUtils.clone, RoomEnvironment, hdr, host, opts);
}

function build(
  T: Three,
  assets: SchoolAssets,
  cloneSkinned: (o: Obj) => Obj,
  RoomEnvironment: new () => THREE_NS.Scene,
  hdr: THREE_NS.DataTexture | null,
  host: HTMLElement,
  opts: MountOpts,
): SchoolHandle {
  const reduced = opts.reducedMotion;
  const renderer = new T.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' });
  renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
  renderer.outputColorSpace = T.SRGBColorSpace;
  renderer.toneMapping = T.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.05;
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = T.PCFShadowMap;
  // A software rasteriser (SwiftShader / llvmpipe — CI, VMs, no GPU) can't
  // afford soft shadows at 2× pixels: drop both so the room stays live.
  try {
    const gl = renderer.getContext();
    const info = gl.getExtension('WEBGL_debug_renderer_info');
    const name = String(info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER));
    if (/swiftshader|llvmpipe|softpipe|software/i.test(name)) {
      renderer.shadowMap.enabled = false;
      renderer.setPixelRatio(1);
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
  const sun = new T.DirectionalLight(0xfff1dc, 2.4);
  sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048);
  sun.shadow.bias = -0.0004;
  sun.shadow.normalBias = 0.02;
  const fill = new T.DirectionalLight(0xcfe3ff, 0.5);
  scene.add(hemi, sun, sun.target, fill, fill.target);

  const corridorGroup = new T.Group();
  const roomGroup = new T.Group();
  scene.add(corridorGroup, roomGroup);

  // ── Kit instancing ────────────────────────────────────────────────────────
  const descCache = new Map<string, { meshes: MeshDesc[]; live: { name: string; obj: Obj; rel: THREE_NS.Matrix4 }[] }>();
  const warned = new Set<string>();
  function kitDesc(node: string) {
    const hit = descCache.get(node);
    if (hit) return hit;
    const root = assets.kit.get(node);
    if (!root) {
      if (!warned.has(node)) {
        warned.add(node);
        if (KIT_NODES.includes(node as KitNode)) assets.missing.push(node);
      }
      return null;
    }
    root.updateMatrixWorld(true);
    const inv = new T.Matrix4().copy(root.matrixWorld).invert();
    const meshes: MeshDesc[] = [];
    const live: { name: string; obj: Obj; rel: THREE_NS.Matrix4 }[] = [];
    for (const child of root.children) {
      if (LIVE_CHILDREN.has(child.name)) {
        live.push({ name: child.name, obj: child, rel: new T.Matrix4().multiplyMatrices(inv, child.matrixWorld) });
        continue;
      }
      child.traverse((o) => {
        const m = o as THREE_NS.Mesh;
        if (!m.isMesh || (m as unknown as THREE_NS.SkinnedMesh).isSkinnedMesh) return;
        meshes.push({ geometry: m.geometry, material: m.material, rel: new T.Matrix4().multiplyMatrices(inv, m.matrixWorld) });
      });
    }
    // The root itself may be the mesh.
    const rm = root as THREE_NS.Mesh;
    if (rm.isMesh) meshes.push({ geometry: rm.geometry, material: rm.material, rel: new T.Matrix4() });
    const d = { meshes, live };
    descCache.set(node, d);
    return d;
  }

  const tmpM = new T.Matrix4();
  const tmpQ = new T.Quaternion();
  const tmpV = new T.Vector3();
  const one = new T.Vector3(1, 1, 1);
  const up = new T.Vector3(0, 1, 0);
  function placeMatrix(p: Placement, out = new T.Matrix4()): THREE_NS.Matrix4 {
    tmpQ.setFromAxisAngle(up, p.rotY);
    return out.compose(tmpV.set(p.x, p.y, p.z), tmpQ, one);
  }

  /** Instanced statics for `placements` into `group`. Returns, per tag, the
   *  (instancedMesh, index, rel) triples so the caller can move one instance
   *  (a chair sliding back). */
  function instance(group: THREE_NS.Group, placements: Placement[], shadows: boolean) {
    const byNode = new Map<string, Placement[]>();
    for (const p of placements) {
      const l = byNode.get(p.node);
      if (l) l.push(p);
      else byNode.set(p.node, [p]);
    }
    const handles = new Map<string, { mesh: THREE_NS.InstancedMesh; index: number; rel: THREE_NS.Matrix4; base: THREE_NS.Matrix4 }[]>();
    for (const [node, list] of byNode) {
      const d = kitDesc(node);
      if (!d) continue;
      for (const md of d.meshes) {
        const im = new T.InstancedMesh(md.geometry, md.material, list.length);
        im.castShadow = shadows && node !== 'Floor_Tile' && node !== 'Corridor_Floor';
        im.receiveShadow = shadows;
        im.frustumCulled = false;
        list.forEach((p, i) => {
          const base = placeMatrix(p);
          im.setMatrixAt(i, tmpM.multiplyMatrices(base, md.rel));
          if (p.tag && node === 'Chair') {
            const h = handles.get(p.tag) ?? [];
            h.push({ mesh: im, index: i, rel: md.rel, base });
            handles.set(p.tag, h);
          }
        });
        im.instanceMatrix.needsUpdate = true;
        im.userData.owned = true;
        group.add(im);
      }
    }
    return handles;
  }

  function paintable(src: Obj, w: number, h: number): Painted | null {
    const m = src as THREE_NS.Mesh;
    if (!m.isMesh) return null;
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    const ctx = c.getContext('2d');
    if (!ctx) return null;
    const tex = new T.CanvasTexture(c);
    tex.colorSpace = T.SRGBColorSpace;
    tex.anisotropy = 4;
    const mat = new T.MeshBasicMaterial({ map: tex, toneMapped: false });
    const mesh = new T.Mesh(m.geometry, mat);
    mesh.userData.owned = true;
    return { mesh, canvas: c, ctx, tex, key: '' };
  }

  /** The live children of one placement (screen, board title, door bits). */
  function liveChildren(group: THREE_NS.Group, p: Placement) {
    const d = kitDesc(p.node);
    const out: Record<string, { painted?: Painted; pivot?: { obj: Obj; base: THREE_NS.Matrix4 } }> = {};
    if (!d) return out;
    const base = placeMatrix(p);
    for (const l of d.live) {
      const world = new T.Matrix4().multiplyMatrices(base, l.rel);
      if (l.name === 'DoorLeaf') {
        const obj = l.obj.clone(true);
        obj.matrixAutoUpdate = false;
        obj.matrix.copy(world);
        obj.traverse((o) => {
          o.castShadow = true;
          o.receiveShadow = true;
        });
        group.add(obj);
        out.DoorLeaf = { pivot: { obj, base: world } };
        continue;
      }
      const size = l.name === 'Screen' ? [SCREEN_W, SCREEN_H] : l.name === 'BoardTitle' ? [768, 150] : l.name === 'NamePlate' ? [640, 160] : [320, 224];
      const painted = paintable(l.obj, size[0], size[1]);
      if (!painted) continue;
      painted.mesh.matrixAutoUpdate = false;
      painted.mesh.matrix.copy(world);
      painted.mesh.matrixWorldNeedsUpdate = true;
      group.add(painted.mesh);
      out[l.name] = { painted };
    }
    return out;
  }

  function clearGroup(g: THREE_NS.Group): void {
    for (const o of [...g.children]) {
      o.traverse((x) => {
        if (!x.userData.owned) return;
        if ((x as THREE_NS.InstancedMesh).isInstancedMesh) (x as THREE_NS.InstancedMesh).dispose();
        const m = x as THREE_NS.Mesh;
        const mat = m.material as THREE_NS.MeshBasicMaterial | undefined;
        if (mat && !Array.isArray(mat) && mat.map instanceof T.CanvasTexture) {
          mat.map.dispose();
          mat.dispose();
        }
      });
      g.remove(o);
    }
  }

  // ── State ────────────────────────────────────────────────────────────────
  let school: School | null = null;
  let view: SchoolView = { kind: 'corridor' };
  const viewFns: ((v: SchoolView) => void)[] = [];
  const life = new Life({ reducedMotion: reduced });
  let openRoom: Room | null = null;
  let roomKey = '';
  let corridorKey = '';
  const doors = new Map<string, Door>();
  /** The open room's own door leaf (swings with its corridor door). */
  let roomDoorPivots: { obj: Obj; base: THREE_NS.Matrix4 }[] = [];
  const screens = new Map<string, Painted>(); // desk tag → monitor
  const screenInfo = new Map<string, ScreenInfo>(); // kid id → last info
  let chairs = new Map<string, { mesh: THREE_NS.InstancedMesh; index: number; rel: THREE_NS.Matrix4; base: THREE_NS.Matrix4 }[]>();
  const chairBack = new Map<string, number>();
  let boardTitle: Painted | null = null;
  const characters = new Map<string, Character>();
  let head: Character | null = null;
  const kicks = new Map<string, { resolve: () => void; until: number }>();
  let hover: SchoolPick | null = null;
  let selected: string | null = null;
  let dirty = true;
  let frames = 0;

  function setView(v: SchoolView): void {
    view = v;
    canvas.dataset.view = v.kind;
    canvas.dataset.room = v.kind === 'corridor' ? '' : v.roomId;
    for (const f of viewFns) f(v);
  }

  const ringGeo = new T.RingGeometry(0.34, 0.44, 40);
  ringGeo.rotateX(-Math.PI / 2);
  const ringMat = new T.MeshBasicMaterial({ color: 0x4f8cff, transparent: true, opacity: 0.9, depthWrite: false });
  const hoverMat = new T.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.55, depthWrite: false });
  const selRing = new T.Mesh(ringGeo, ringMat);
  const hoverRing = new T.Mesh(ringGeo, hoverMat);
  selRing.visible = hoverRing.visible = false;
  selRing.renderOrder = hoverRing.renderOrder = 2;
  scene.add(selRing, hoverRing);
  const hitGeo = new T.BoxGeometry(1, 1, 1);
  const hitMat = new T.MeshBasicMaterial({ visible: false });

  // ── Corridor ─────────────────────────────────────────────────────────────
  function buildCorridor(s: School): void {
    clearGroup(corridorGroup);
    doors.clear();
    const placements = furnishCorridor(s.rooms, s.corridorLength);
    instance(corridorGroup, placements.filter((p) => p.node !== 'Corridor_Door'), true);
    // Doors: static frame instanced, the leaf / name plate / sign live.
    const doorPl = placements.filter((p) => p.node === 'Corridor_Door');
    instance(corridorGroup, doorPl, true);
    for (const p of doorPl) {
      const roomId = p.tag!.slice('door:'.length);
      const live = liveChildren(corridorGroup, p);
      const hit = new T.Mesh(hitGeo, hitMat);
      hit.scale.set(1.4, 2.4, 0.5);
      hit.position.set(p.x, 1.2, p.z + 0.1);
      hit.userData.pick = { kind: 'door', id: roomId } satisfies SchoolPick;
      corridorGroup.add(hit);
      doors.set(roomId, { roomId, pivots: live.DoorLeaf?.pivot ? [live.DoorLeaf.pivot] : [], angle: 0, target: 0, hit, plate: live.NamePlate?.painted ?? null, sign: live.DoorSign?.painted ?? null });
    }
    paintDoors(s);
  }

  function paintDoors(s: School): void {
    for (const r of s.rooms) {
      const d = doors.get(r.id);
      if (!d) continue;
      if (d.plate) {
        const key = r.name;
        if (d.plate.key !== key) {
          d.plate.key = key;
          const { ctx, canvas: c } = d.plate;
          ctx.fillStyle = '#f6f1e3';
          ctx.fillRect(0, 0, c.width, c.height);
          ctx.strokeStyle = '#8a6a3c';
          ctx.lineWidth = 10;
          ctx.strokeRect(5, 5, c.width - 10, c.height - 10);
          ctx.fillStyle = '#2b2118';
          ctx.textAlign = 'center';
          ctx.textBaseline = 'middle';
          let size = 64;
          ctx.font = `700 ${size}px ui-rounded, -apple-system, system-ui, sans-serif`;
          while (size > 26 && ctx.measureText(r.name).width > c.width - 50) {
            size -= 4;
            ctx.font = `700 ${size}px ui-rounded, -apple-system, system-ui, sans-serif`;
          }
          ctx.fillText(r.name, c.width / 2, c.height / 2 + 2);
          d.plate.tex.needsUpdate = true;
        }
      }
      if (d.sign) {
        const c = r.counts;
        const key = JSON.stringify(c);
        if (d.sign.key !== key) {
          d.sign.key = key;
          const { ctx, canvas: cv } = d.sign;
          ctx.fillStyle = '#1f2a3a';
          ctx.fillRect(0, 0, cv.width, cv.height);
          ctx.textAlign = 'left';
          ctx.textBaseline = 'middle';
          const rows: [string, string][] = [];
          if (c.needsYou) rows.push(['#f5b83d', `✋ ${c.needsYou} ${c.needsYou === 1 ? 'needs' : 'need'} you`]);
          if (c.working) rows.push(['#4ccf72', `● ${c.working} working`]);
          if (c.idle) rows.push(['#9fb4cc', `○ ${c.idle} idle`]);
          if (!rows.length) rows.push(['#9fb4cc', c.total ? `${c.total} asleep` : 'empty']);
          ctx.font = '600 34px -apple-system, system-ui, sans-serif';
          rows.slice(0, 4).forEach(([col, txt], i) => {
            ctx.fillStyle = col;
            ctx.fillText(txt, 20, 38 + i * 50);
          });
          d.sign.tex.needsUpdate = true;
        }
      }
    }
    dirty = true;
  }

  // ── Room ─────────────────────────────────────────────────────────────────
  const dimsKey = (r: Room) => `${r.id}|${r.width}|${r.depth}|${r.cols}|${r.rows}|${r.backRows}|${r.cx}`;

  function buildRoomStatics(r: Room): void {
    // Characters live in roomGroup too — detach them across the rebuild.
    const keep = [...characters.values()].map((c) => c.obj).concat(head ? [head.obj] : []);
    for (const o of keep) roomGroup.remove(o);
    clearGroup(roomGroup);
    screens.clear();
    chairBack.clear();
    roomDoorPivots = [];
    const placements = furnishRoom(r);
    chairs = instance(roomGroup, placements, true);
    boardTitle = null;
    for (const p of placements) {
      if (p.node === 'Workstation' && p.tag) {
        const live = liveChildren(roomGroup, p);
        const scr = live.Screen?.painted;
        if (scr) {
          scr.mesh.userData.desk = p.tag;
          screens.set(p.tag, scr);
        }
      } else if (p.node === 'Board') {
        boardTitle = liveChildren(roomGroup, p).BoardTitle?.painted ?? null;
      } else if (p.node === 'Wall_Door') {
        const live = liveChildren(roomGroup, p);
        if (live.DoorLeaf?.pivot) roomDoorPivots.push(live.DoorLeaf.pivot);
        const hit = new T.Mesh(hitGeo, hitMat);
        hit.scale.set(1.4, 2.4, 0.5);
        hit.position.set(p.x, 1.2, p.z - 0.1);
        hit.userData.pick = { kind: 'door', id: r.id } satisfies SchoolPick;
        roomGroup.add(hit);
      }
    }
    for (const o of keep) roomGroup.add(o);
    paintBoard(r);
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
    sun.position.set(r.cx - r.width / 2 - 8, 12, r.cz - 3);
    sun.target.position.set(r.cx, 0, r.cz);
    fill.position.set(r.cx + 6, 8, r.cz + 8);
    fill.target.position.set(r.cx, 0, r.cz);
    for (const s of screens.values()) s.key = '';
    for (const [id, info] of screenInfo) applyScreen(id, info);
    paintEmptyDesks(r);
    dirty = true;
  }

  function paintBoard(r: Room): void {
    const b = boardTitle;
    if (!b || b.key === `${r.name}|${roomSummary(r)}`) return;
    b.key = `${r.name}|${roomSummary(r)}`;
    const { ctx, canvas: c } = b;
    ctx.fillStyle = '#2f5d46';
    ctx.fillRect(0, 0, c.width, c.height);
    ctx.fillStyle = 'rgba(255,255,255,0.92)';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    let size = 70;
    ctx.font = `600 ${size}px "Chalkboard SE", "Comic Sans MS", ui-rounded, system-ui, sans-serif`;
    while (size > 28 && ctx.measureText(r.name).width > c.width - 40) {
      size -= 4;
      ctx.font = `600 ${size}px "Chalkboard SE", "Comic Sans MS", ui-rounded, system-ui, sans-serif`;
    }
    ctx.fillText(r.name, c.width / 2, c.height * 0.42);
    ctx.font = '30px "Chalkboard SE", "Comic Sans MS", ui-rounded, system-ui, sans-serif';
    ctx.fillStyle = 'rgba(255,255,255,0.7)';
    ctx.fillText(roomSummary(r), c.width / 2, c.height * 0.82);
    b.tex.needsUpdate = true;
  }

  function deskOf(kidId: string): string | null {
    const k = openRoom?.kids.find((x) => x.id === kidId);
    return k ? `${k.row}:${k.col}` : null;
  }

  function applyScreen(kidId: string, info: ScreenInfo): void {
    const tag = deskOf(kidId);
    const scr = tag ? screens.get(tag) : null;
    if (!scr) return;
    const full = { ...info, selected: selected === kidId };
    const key = screenKey(full);
    if (scr.key === key) return;
    scr.key = key;
    paintScreen(scr.ctx, full);
    scr.tex.needsUpdate = true;
    dirty = true;
  }

  function paintEmptyDesks(r: Room): void {
    const used = new Set(r.kids.map((k) => `${k.row}:${k.col}`));
    for (const [tag, scr] of screens) {
      if (used.has(tag) || scr.key === 'empty') continue;
      scr.key = 'empty';
      paintScreen(scr.ctx, { title: '', provider: '', character: 'custom', pose: 'empty', feed: null });
      scr.tex.needsUpdate = true;
    }
  }

  // ── Characters ───────────────────────────────────────────────────────────
  function makeCharacter(id: string, g: GLTF, clips: readonly string[]): Character {
    const obj = cloneSkinned(g.scene);
    obj.traverse((o) => {
      if ((o as THREE_NS.Mesh).isMesh) {
        o.castShadow = true;
        o.receiveShadow = true;
        o.frustumCulled = false;
      }
    });
    const mixer = new T.AnimationMixer(obj);
    const actions = new Map<string, THREE_NS.AnimationAction>();
    for (const c of g.animations) {
      if (!clips.includes(c.name)) continue;
      const a = mixer.clipAction(c);
      if (ONCE_CLIPS.has(c.name)) {
        a.setLoop(T.LoopOnce, 1);
        a.clampWhenFinished = true;
      }
      actions.set(c.name, a);
    }
    const hit = new T.Mesh(hitGeo, hitMat);
    hit.userData.pick = id === 'headmaster' ? ({ kind: 'head', id: 'headmaster' } satisfies SchoolPick) : ({ kind: 'kid', id } satisfies SchoolPick);
    if (id === 'headmaster') {
      hit.scale.set(0.8, 1.9, 0.8);
      hit.position.y = 0.95;
    } else {
      hit.scale.set(0.6, 1.2, 0.6);
      hit.position.y = 0.6;
    }
    obj.add(hit);
    roomGroup.add(obj);
    return { id, obj, mixer, actions, clip: '', hit, heading: Math.PI };
  }

  function play(c: Character, clip: string, phaseMs?: number): void {
    if (c.clip === clip) return;
    const next = c.actions.get(clip) ?? c.actions.get('Idle_Stand') ?? c.actions.get('Idle') ?? null;
    const prev = c.actions.get(c.clip);
    c.clip = clip;
    if (!next) return;
    next.reset();
    if (phaseMs && ONCE_CLIPS.has(clip)) next.timeScale = next.getClip().duration / (phaseMs / 1000);
    else next.timeScale = 1;
    next.enabled = true;
    next.setEffectiveWeight(1);
    next.play();
    if (prev && prev !== next) prev.crossFadeTo(next, reduced ? 0 : 0.28, false);
  }

  const KID_CLIP_NAMES = ['Sit_Type', 'Sit_Idle', 'Sit_RaiseHand', 'Sit_Slump', 'Stand_Up', 'Sit_Down', 'Walk', 'Idle_Stand', 'Talk', 'Look_Around', 'Wave'];

  function syncCharacters(r: Room): void {
    const want = new Map<string, Kid>([...r.kids, ...r.bench].map((k) => [k.id, k]));
    for (const [id, c] of characters) {
      if (want.has(id) || kicks.has(id)) continue;
      roomGroup.remove(c.obj);
      c.mixer.stopAllAction();
      characters.delete(id);
    }
    for (const [id, k] of want) {
      if (characters.has(id)) continue;
      const g = assets.kids.get(k.character)!;
      characters.set(id, makeCharacter(id, g, KID_CLIP_NAMES));
    }
    if (!head) head = makeCharacter('headmaster', assets.head, HEAD_CLIPS);
  }

  function dropCharacters(): void {
    for (const c of characters.values()) {
      c.mixer.stopAllAction();
      roomGroup.remove(c.obj);
    }
    characters.clear();
    if (head) {
      head.mixer.stopAllAction();
      roomGroup.remove(head.obj);
      head = null;
    }
    for (const k of kicks.values()) k.resolve();
    kicks.clear();
  }

  function placeCharacter(c: Character, v: ActorView, r: Room, dt: number): void {
    c.obj.visible = v.visible;
    const w = toWorld(r, v);
    c.obj.position.set(w.x, 0, w.z);
    // Turn smoothly (shortest way round).
    let d = v.heading - c.heading;
    d = Math.atan2(Math.sin(d), Math.cos(d));
    c.heading += reduced ? d : d * Math.min(1, dt / 120);
    c.obj.rotation.y = c.heading;
    const phase = v.clip === 'Stand_Up' ? STAND_MS : v.clip === 'Sit_Down' ? SIT_MS : v.clip === 'Nod' ? NOD_MS : v.clip === 'Scold' ? SCOLD_MS : undefined;
    play(c, v.clip, phase);
  }

  function updateChairs(r: Room, dt: number): void {
    for (const k of r.kids) {
      const tag = `${k.row}:${k.col}`;
      const v = life.kidView(k.id);
      const want = v && !v.seated && v.visible ? 0.38 : 0;
      const cur = chairBack.get(tag) ?? 0;
      if (Math.abs(cur - want) < 1e-3) continue;
      const next = reduced ? want : cur + (want - cur) * Math.min(1, dt / 160);
      chairBack.set(tag, next);
      for (const h of chairs.get(tag) ?? []) {
        // A WORLD +z shift: kids face −Z, so +z is behind the kid.
        const m = new T.Matrix4().makeTranslation(0, 0, next).multiply(h.base).multiply(h.rel);
        h.mesh.setMatrixAt(h.index, m);
        h.mesh.instanceMatrix.needsUpdate = true;
      }
    }
  }

  // ── Camera ───────────────────────────────────────────────────────────────
  const cam = {
    pos: new T.Vector3(0.7, EYE, CORRIDOR_HALF - 0.6),
    target: new T.Vector3(6, 1.4, -1.6),
    yaw: Math.PI / 2 + 0.62,
    pitch: -0.08,
    dist: 6,
    tween: null as null | { from: [THREE_NS.Vector3, THREE_NS.Vector3]; to: [THREE_NS.Vector3, THREE_NS.Vector3]; t: number; ms: number; done: () => void },
  };
  const keys = new Set<string>();

  function lookDir(yaw: number, pitch: number, out = new T.Vector3()): THREE_NS.Vector3 {
    return out.set(Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), Math.cos(yaw) * Math.cos(pitch));
  }

  function fly(pos: THREE_NS.Vector3, target: THREE_NS.Vector3, ms: number): Promise<void> {
    if (cam.tween) cam.tween.done();
    if (reduced || ms <= 0) {
      cam.pos.copy(pos);
      cam.target.copy(target);
      dirty = true;
      return Promise.resolve();
    }
    return new Promise((resolve) => {
      cam.tween = { from: [cam.pos.clone(), cam.target.clone()], to: [pos.clone(), target.clone()], t: 0, ms, done: resolve };
    });
  }

  /** Standing by the lockers, looking along the corridor at the doors. */
  function corridorPose(x: number): [THREE_NS.Vector3, THREE_NS.Vector3] {
    const pos = new T.Vector3(x, EYE, CORRIDOR_HALF - 0.6);
    return [pos, pos.clone().add(lookDir(Math.PI / 2 + 0.62, -0.08))];
  }

  function roomPose(r: Room): [THREE_NS.Vector3, THREE_NS.Vector3] {
    const pos = new T.Vector3(r.cx + r.width / 2 - 0.9, 2.55, r.cz + r.depth / 2 - 0.9);
    const target = new T.Vector3(r.cx - r.width * 0.12, 0.75, r.cz - r.depth * 0.12);
    return [pos, target];
  }

  function syncOrbitFromCam(): void {
    const off = new T.Vector3().subVectors(cam.pos, cam.target);
    cam.dist = Math.max(0.5, off.length());
    cam.yaw = Math.atan2(off.x, off.z);
    cam.pitch = Math.asin(Math.max(-1, Math.min(1, off.y / cam.dist)));
  }

  function syncWalkFromCam(): void {
    const d = new T.Vector3().subVectors(cam.target, cam.pos).normalize();
    cam.yaw = Math.atan2(d.x, d.z);
    cam.pitch = Math.asin(Math.max(-1, Math.min(1, d.y)));
  }

  function clampRoom(p: THREE_NS.Vector3, r: Room, pad = 0.3): void {
    p.x = Math.min(r.cx + r.width / 2 - pad, Math.max(r.cx - r.width / 2 + pad, p.x));
    p.z = Math.min(r.cz + r.depth / 2 - pad, Math.max(r.cz - r.depth / 2 + pad, p.z));
  }

  function applyOrbit(): void {
    const r = openRoom;
    if (!r) return;
    clampRoom(cam.target, r, 0.8);
    cam.target.y = Math.min(1.6, Math.max(0.3, cam.target.y));
    const p = cam.target.clone().add(lookDir(cam.yaw, cam.pitch).multiplyScalar(cam.dist));
    clampRoom(p, r);
    p.y = Math.min(2.85, Math.max(0.45, p.y));
    cam.pos.copy(p);
  }

  function applyWalk(): void {
    const len = school?.corridorLength ?? 18;
    cam.pos.x = Math.min(len - 0.5, Math.max(0.5, cam.pos.x));
    cam.pos.z = Math.min(CORRIDOR_HALF - 0.5, Math.max(-CORRIDOR_HALF + 0.5, cam.pos.z));
    cam.pos.y = EYE;
    cam.target.copy(cam.pos).add(lookDir(cam.yaw, cam.pitch));
  }

  function stepInput(dt: number): void {
    if (cam.tween || keys.size === 0 || view.kind === 'screen') return;
    const s = dt / 1000;
    const fwd = (keys.has('w') || keys.has('arrowup') ? 1 : 0) - (keys.has('s') || keys.has('arrowdown') ? 1 : 0);
    const side = (keys.has('d') ? 1 : 0) - (keys.has('a') ? 1 : 0);
    const turn = (keys.has('arrowright') ? 1 : 0) - (keys.has('arrowleft') ? 1 : 0);
    const zoom = (keys.has('-') ? 1 : 0) - (keys.has('=') || keys.has('+') ? 1 : 0);
    if (view.kind === 'corridor') {
      cam.yaw -= turn * 1.8 * s;
      const f = new T.Vector3(Math.sin(cam.yaw), 0, Math.cos(cam.yaw));
      const rgt = new T.Vector3(-f.z, 0, f.x);
      cam.pos.addScaledVector(f, fwd * 3.2 * s).addScaledVector(rgt, side * 2.6 * s);
      applyWalk();
    } else {
      cam.yaw += turn * 1.4 * s;
      const f = new T.Vector3(-Math.sin(cam.yaw), 0, -Math.cos(cam.yaw));
      const rgt = new T.Vector3(-f.z, 0, f.x);
      cam.target.addScaledVector(f, fwd * 2.6 * s).addScaledVector(rgt, side * 2.6 * s);
      cam.dist = Math.min(14, Math.max(1.2, cam.dist * (1 + zoom * 1.2 * s)));
      applyOrbit();
    }
    dirty = true;
  }

  // ── Doors ────────────────────────────────────────────────────────────────
  function openDoor(roomId: string, open: boolean): void {
    const d = doors.get(roomId);
    if (!d) return;
    d.target = open ? -Math.PI * 0.55 : 0;
    if (reduced) d.angle = d.target;
    dirty = true;
  }

  function stepDoors(dt: number): void {
    for (const d of doors.values()) {
      const pivots = openRoom?.id === d.roomId ? [...d.pivots, ...roomDoorPivots] : d.pivots;
      if (Math.abs(d.angle - d.target) < 1e-3 && pivots.every((p) => p.obj.userData.angle === d.angle)) continue;
      d.angle += (d.target - d.angle) * Math.min(1, dt / 140);
      if (Math.abs(d.angle - d.target) < 0.002) d.angle = d.target;
      for (const p of pivots) {
        p.obj.matrix.copy(p.base).multiply(new T.Matrix4().makeRotationY(d.angle));
        p.obj.matrixWorldNeedsUpdate = true;
        p.obj.userData.angle = d.angle;
      }
      dirty = true;
    }
  }

  // ── Transitions ──────────────────────────────────────────────────────────
  let busy: Promise<void> = Promise.resolve();
  const queue = (f: () => Promise<void>): Promise<void> => (busy = busy.then(f, f));

  function openRoomNow(r: Room): void {
    openRoom = r;
    roomKey = dimsKey(r);
    dropCharacters();
    buildRoomStatics(r);
    life.setRoom(r);
    syncCharacters(r);
    roomGroup.visible = true;
  }

  function enterRoom(id: string): Promise<void> {
    return queue(async () => {
      const r = school?.rooms.find((x) => x.id === id);
      if (!r) return;
      if (view.kind !== 'corridor') {
        if (openRoom?.id === id) {
          if (view.kind === 'screen') await backToRoomNow();
          return;
        }
        await toCorridorNow();
      }
      openRoomNow(r);
      openDoor(r.id, true);
      await fly(new T.Vector3(r.doorX, EYE, 0.6), new T.Vector3(r.doorX, 1.35, -3), 650);
      const [p, t] = roomPose(r);
      await fly(p, t, 950);
      openDoor(r.id, false);
      syncOrbitFromCam();
      setView({ kind: 'room', roomId: r.id });
    });
  }

  async function toCorridorNow(): Promise<void> {
    const r = openRoom;
    if (!r) {
      setView({ kind: 'corridor' });
      return;
    }
    openDoor(r.id, true);
    await fly(new T.Vector3(r.doorX, EYE, ROOM_BACK_Z - 0.9), new T.Vector3(r.doorX, 1.4, 3), 750);
    const [p, t] = corridorPose(r.doorX);
    await fly(p, t, 700);
    openDoor(r.id, false);
    dropCharacters();
    clearGroup(roomGroup);
    screens.clear();
    roomDoorPivots = [];
    openRoom = null;
    roomKey = '';
    life.setRoom(null);
    syncWalkFromCam();
    setView({ kind: 'corridor' });
  }

  async function backToRoomNow(): Promise<void> {
    const r = openRoom;
    if (!r) return;
    const [p, t] = roomPose(r);
    const prev = view.kind === 'screen' ? view.kidId : null;
    await fly(p, t, 700);
    syncOrbitFromCam();
    setView({ kind: 'room', roomId: r.id });
    if (prev) {
      const info = screenInfo.get(prev);
      if (info) applyScreen(prev, info);
    }
  }

  function lookAtScreen(kidId: string): Promise<void> {
    return queue(async () => {
      const r = openRoom;
      const tag = deskOf(kidId);
      const scr = tag ? screens.get(tag) : null;
      if (!r || !scr) return;
      scr.mesh.updateMatrixWorld(true);
      if (!scr.mesh.geometry.boundingBox) scr.mesh.geometry.computeBoundingBox();
      const c = scr.mesh.geometry.boundingBox!.getCenter(new T.Vector3()).applyMatrix4(scr.mesh.matrixWorld);
      // Monitors face +Z (toward their kid): look over the kid's shoulder.
      await fly(new T.Vector3(c.x + 0.18, c.y + 0.3, c.z + 0.95), c, 750);
      setView({ kind: 'screen', roomId: r.id, kidId });
    });
  }

  // ── Picking / projection ─────────────────────────────────────────────────
  const ray = new T.Raycaster();
  const ndc = new T.Vector2();
  function pick(cx: number, cy: number): SchoolPick | null {
    const rect = canvas.getBoundingClientRect();
    if (!rect.width || !rect.height) return null;
    ndc.set(((cx - rect.left) / rect.width) * 2 - 1, -((cy - rect.top) / rect.height) * 2 + 1);
    ray.setFromCamera(ndc, camera);
    const targets: Obj[] = [];
    if (view.kind === 'corridor') {
      for (const d of doors.values()) if (d.hit) targets.push(d.hit);
    } else {
      for (const c of characters.values()) if (c.obj.visible) targets.push(c.hit);
      if (head) targets.push(head.hit);
      for (const s of screens.values()) targets.push(s.mesh);
      roomGroup.traverse((o) => {
        if ((o.userData.pick as SchoolPick | undefined)?.kind === 'door') targets.push(o);
      });
    }
    const hit = ray.intersectObjects(targets, false)[0];
    if (!hit) return null;
    const desk = hit.object.userData.desk as string | undefined;
    if (desk) {
      const k = openRoom?.kids.find((x) => `${x.row}:${x.col}` === desk);
      return k ? { kind: 'kid', id: k.id } : null;
    }
    return (hit.object.userData.pick as SchoolPick | undefined) ?? null;
  }

  function project(p: SchoolPick): ScreenPoint | null {
    let w: THREE_NS.Vector3 | null = null;
    if (p.kind === 'kid') {
      const c = characters.get(p.id);
      if (c && c.obj.visible) w = c.obj.position.clone().setY(1.3);
    } else if (p.kind === 'head') {
      if (head) w = head.obj.position.clone().setY(1.95);
    } else {
      const r = school?.rooms.find((x) => x.id === p.id);
      if (r) w = new T.Vector3(r.doorX, 2.75, -CORRIDOR_HALF + 0.05);
    }
    if (!w) return null;
    const rect = canvas.getBoundingClientRect();
    const v = w.project(camera);
    const x = rect.left + ((v.x + 1) / 2) * rect.width;
    const y = rect.top + ((1 - v.y) / 2) * rect.height;
    const visible = v.z < 1 && v.z > -1 && x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
    return { x, y, visible };
  }

  function visibleScreens(): string[] {
    const r = openRoom;
    if (!r) return [];
    const frustum = new T.Frustum().setFromProjectionMatrix(new T.Matrix4().multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse));
    const out: { id: string; d: number }[] = [];
    for (const k of r.kids) {
      const w = toWorld(r, k.seat);
      const pt = new T.Vector3(w.x, 1, w.z - 0.55);
      if (view.kind === 'screen' && view.kidId === k.id) out.push({ id: k.id, d: -1 });
      else if (frustum.containsPoint(pt)) out.push({ id: k.id, d: pt.distanceTo(cam.pos) });
    }
    return out.sort((a, b) => a.d - b.d).map((x) => x.id);
  }

  function facingDoor(): string | null {
    if (view.kind !== 'corridor' || !school) return null;
    const f = lookDir(cam.yaw, 0);
    let best: { id: string; s: number } | null = null;
    for (const r of school.rooms) {
      const to = new T.Vector3(r.doorX - cam.pos.x, 0, -CORRIDOR_HALF - cam.pos.z);
      const d = to.length();
      const s = to.normalize().dot(f);
      if (d < 6 && s > 0.3 && (!best || s - d * 0.05 > best.s)) best = { id: r.id, s: s - d * 0.05 };
    }
    return best?.id ?? null;
  }

  // ── Loop ─────────────────────────────────────────────────────────────────
  let active = true;
  let raf = 0;
  let last = performance.now();
  let acc = 0;
  /** Uncapped time since the last drawn frame (camera flights run on it). */
  let accRaw = 0;
  const frameFns: (() => void)[] = [];
  const lostFns: (() => void)[] = [];

  function frame(now: number): void {
    raf = 0;
    if (!active) return;
    const raw = Math.max(0, now - last);
    const dt = Math.min(1000, raw);
    last = now;
    acc += dt;
    accRaw += raw;
    if (acc < FPS_MS) {
      raf = requestAnimationFrame(frame);
      return;
    }
    // Life steps are capped at 1 s (a long stall never teleports anyone, a
    // slow GPU still walks in real time); camera flights use real time.
    const step = acc;
    const elapsed = accRaw;
    acc = 0;
    accRaw = 0;
    // Camera.
    if (cam.tween) {
      const tw = cam.tween;
      tw.t += elapsed;
      const k = ease(Math.min(1, tw.t / tw.ms));
      cam.pos.lerpVectors(tw.from[0], tw.to[0], k);
      cam.target.lerpVectors(tw.from[1], tw.to[1], k);
      if (tw.t >= tw.ms) {
        cam.tween = null;
        tw.done();
      }
      dirty = true;
    } else stepInput(step);
    stepDoors(step);
    // Life.
    const r = openRoom;
    if (r) {
      life.step(step);
      for (const c of characters.values()) {
        const v = life.kidView(c.id);
        if (v) placeCharacter(c, v, r, step);
        c.mixer.update(step / 1000);
      }
      const hv = life.headView();
      if (head && hv) {
        placeCharacter(head, hv, r, step);
        head.mixer.update(step / 1000);
      }
      updateChairs(r, step);
      for (const [id, k] of kicks) {
        if (life.isGone(id) || now > k.until) {
          kicks.delete(id);
          const c = characters.get(id);
          if (c) c.obj.visible = false;
          k.resolve();
        }
      }
      placeRing(selRing, selected);
      placeRing(hoverRing, hover?.kind === 'kid' && hover.id !== selected ? hover.id : null);
      dirty = true;
    }
    if (dirty) {
      camera.position.copy(cam.pos);
      camera.lookAt(cam.target);
      renderer.render(scene, camera);
      frames++;
      dirty = false;
      for (const f of frameFns) f();
    }
    raf = requestAnimationFrame(frame);
  }

  function placeRing(ring: THREE_NS.Mesh, id: string | null): void {
    const c = id ? characters.get(id) : null;
    ring.visible = !!c && c.obj.visible;
    if (c) ring.position.set(c.obj.position.x, 0.02, c.obj.position.z);
  }

  function kick(): void {
    if (raf || !active) return;
    last = performance.now();
    raf = requestAnimationFrame(frame);
  }

  // ── Size / context / theme ───────────────────────────────────────────────
  const ro = new ResizeObserver(() => {
    const w = Math.max(1, host.clientWidth);
    const h = Math.max(1, host.clientHeight);
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.fov = w / h < 1 ? 68 : 55;
    camera.updateProjectionMatrix();
    dirty = true;
    kick();
  });
  ro.observe(host);
  canvas.addEventListener('webglcontextlost', (e) => {
    e.preventDefault();
    for (const f of lostFns) f();
  });

  function setTheme(dark: boolean): void {
    scene.background = new T.Color(dark ? 0x1b2433 : 0xcfe6fb);
    hemi.intensity = dark ? 0.7 : 1.1;
    sun.intensity = dark ? 1.1 : 2.4;
    sun.color.set(dark ? 0xb8c8ff : 0xfff1dc);
    renderer.toneMappingExposure = dark ? 0.95 : 1.05;
    dirty = true;
    kick();
  }
  setTheme(opts.dark);
  setView({ kind: 'corridor' });
  applyWalk();
  kick();

  return {
    update(s) {
      school = s;
      const ck = s.rooms.map((r) => `${r.id}@${r.doorX}`).join(',') + `|${s.corridorLength}`;
      if (ck !== corridorKey) {
        corridorKey = ck;
        buildCorridor(s);
        // The open room's door pivots live in the room group: re-attach.
        if (openRoom) roomKey = '';
      } else paintDoors(s);
      const r = openRoom ? s.rooms.find((x) => x.id === openRoom!.id) : null;
      if (openRoom && !r) {
        // The workspace went away under us: back to the corridor.
        void queue(toCorridorNow);
      } else if (r) {
        if (dimsKey(r) !== roomKey) {
          roomKey = dimsKey(r);
          openRoom = r;
          buildRoomStatics(r);
        }
        openRoom = r;
        life.sync(r);
        syncCharacters(r);
        paintBoard(r);
        paintEmptyDesks(r);
        for (const [id, info] of screenInfo) applyScreen(id, info);
      }
      dirty = true;
      kick();
    },
    view: () => view,
    onView(fn) {
      viewFns.push(fn);
    },
    enterRoom,
    toCorridor: () => queue(toCorridorNow),
    lookAtScreen,
    backToRoom: () => queue(backToRoomNow),
    pick,
    setHover(p) {
      hover = p;
      dirty = true;
      kick();
    },
    setSelected(id) {
      const prev = selected;
      selected = id;
      for (const k of [prev, id]) {
        const info = k ? screenInfo.get(k) : null;
        if (k && info) applyScreen(k, info);
      }
      dirty = true;
      kick();
    },
    project,
    setScreen(id, info) {
      screenInfo.set(id, info);
      applyScreen(id, info);
      kick();
    },
    visibleScreens,
    facingDoor,
    kickOut(id) {
      if (!openRoom || !characters.has(id)) return Promise.resolve();
      const ms = life.kick(id);
      if (ms <= 0) return Promise.resolve();
      return new Promise<void>((resolve) => {
        kicks.set(id, { resolve, until: performance.now() + ms + 2500 });
        kick();
      });
    },
    detention(id) {
      if (!openRoom || !characters.has(id)) return Promise.resolve();
      const ms = life.detention(id);
      kick();
      return new Promise<void>((resolve) => setTimeout(resolve, reduced ? 0 : Math.min(ms, 9000)));
    },
    restore(id) {
      kicks.get(id)?.resolve();
      kicks.delete(id);
      life.restore(id);
      const c = characters.get(id);
      if (c) c.obj.visible = true;
      kick();
    },
    inspect(id) {
      life.inspect(id);
      kick();
    },
    key(e, down) {
      const k = e.key.toLowerCase();
      if (!['w', 'a', 's', 'd', 'arrowup', 'arrowdown', 'arrowleft', 'arrowright', '+', '=', '-'].includes(k)) return false;
      if (e.metaKey || e.ctrlKey || e.altKey) return false;
      if (down) keys.add(k);
      else keys.delete(k);
      kick();
      return true;
    },
    drag(dx, dy) {
      if (cam.tween || view.kind === 'screen') return;
      if (view.kind === 'corridor') {
        cam.yaw -= dx * 0.0042;
        cam.pitch = Math.max(-0.6, Math.min(0.6, cam.pitch - dy * 0.0035));
        applyWalk();
      } else {
        cam.yaw -= dx * 0.005;
        cam.pitch = Math.max(0.05, Math.min(1.25, cam.pitch + dy * 0.004));
        applyOrbit();
      }
      dirty = true;
      kick();
    },
    wheel(dy) {
      if (cam.tween || view.kind === 'screen') return;
      if (view.kind === 'corridor') {
        cam.pos.addScaledVector(new T.Vector3(Math.sin(cam.yaw), 0, Math.cos(cam.yaw)), -dy * 0.006);
        applyWalk();
      } else {
        cam.dist = Math.min(14, Math.max(1.2, cam.dist * (1 + dy * 0.0012)));
        applyOrbit();
      }
      dirty = true;
      kick();
    },
    resetView() {
      if (view.kind === 'corridor') {
        const [p, t] = corridorPose(0.7);
        void fly(p, t, 600).then(syncWalkFromCam);
      } else if (openRoom) void queue(backToRoomNow);
      kick();
    },
    setActive(on) {
      active = on;
      if (on) {
        dirty = true;
        kick();
      } else {
        keys.clear();
        if (raf) cancelAnimationFrame(raf);
        raf = 0;
      }
    },
    setTheme,
    onFrame(fn) {
      frameFns.push(fn);
    },
    onContextLost(fn) {
      lostFns.push(fn);
    },
    debug() {
      const hv = life.headView();
      const out: Record<string, string> = {};
      for (const [tag, s] of screens) out[tag] = s.key.split('\u0001').slice(4).join('\n');
      return {
        view,
        frames,
        missing: [...assets.missing],
        kids: life.kidViews().map((v) => ({ id: v.id, clip: v.clip, mode: v.mode, visible: v.visible, x: v.x, z: v.z })),
        head: hv ? { clip: hv.clip, mode: hv.mode, x: hv.x, z: hv.z, target: life.headTarget() } : null,
        camera: { x: cam.pos.x, y: cam.pos.y, z: cam.pos.z },
        screens: out,
      };
    },
    destroy() {
      active = false;
      if (raf) cancelAnimationFrame(raf);
      ro.disconnect();
      dropCharacters();
      clearGroup(roomGroup);
      clearGroup(corridorGroup);
      ringGeo.dispose();
      ringMat.dispose();
      hoverMat.dispose();
      hitGeo.dispose();
      hitMat.dispose();
      envRT.dispose();
      pmrem.dispose();
      assets.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      canvas.remove();
    },
  };
}
