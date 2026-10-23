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
//  • Only the room you are in is built. Frames are requested only while
//    there is work (scene/loop.ts — a flight, held keys, a live room, a door
//    swing), at ≤ 30 fps, and never while the box is inactive, off screen or
//    the document hidden (lib/three/stage.ts `demandLoop`/`watchVisibility`).
//  • Light: daylight from the window wall (a low sun, soft shadows) plus warm
//    ceiling lamps; the dark theme is evening — moonlight, lamps up, the
//    window views dimmed. Rooms and the corridor have a ceiling.
//  • State at a glance: a glowing pin over every kid whose session is
//    working / needs you / failed / done (one InstancedMesh), a pulsing amber
//    ring under each kid that needs you (another), and the monitor the camera
//    is on repainted at 2× so its live terminal stays crisp up close.
//  • Reduced motion: camera moves cut instead of flying; life.ts keeps
//    everybody seated; pins and rings hold still.
//  • The WebGL lifecycle is the shared lib/three/stage.ts: `pixelRatio` ≤ 2,
//    `releaseRenderer` on destroy, and a lost context is reported so the host
//    remounts (or offers Retry).

//  • The pieces live in scene/: the public surface (api.ts), the stage —
//    renderer, lights, theme (stage.ts) —, kit instancing (kit.ts), the
//    label painters (paint.ts), the cast (cast.ts), the pins / rings
//    (markers.ts) and the camera rig (camera.ts).

import type * as THREE_NS from 'three';
import { disposeActor, assetBase, loadSchoolAssets, type SchoolAssets } from './assets.ts';
import { furnishCorridor, furnishRoom } from './furnish.ts';
import { HEAD_CLIPS, Life } from './life.ts';
import { CORRIDOR_HALF, ROOM_BACK_Z, type Kid, type Room, type School } from './model.ts';
import { paintScreen, screenKey, type ScreenInfo } from './screens.ts';
import { type MountOpts, type SchoolHandle, type SchoolPick, type SchoolView } from './scene/api.ts';
import { createCamera } from './scene/camera.ts';
import { createCast, KID_CLIP_NAMES } from './scene/cast.ts';
import { createKit, hiResCanvas } from './scene/kit.ts';
import { schoolDebug } from './scene/debug.ts';
import { createMarkers } from './scene/markers.ts';
import { paintBoardTitle, paintPlate, paintSign } from './scene/paint.ts';
import { createPicker } from './scene/pick.ts';
import { EYE, FPS_MS, ease, type Character, type Door, type Obj, type Painted, type Three } from './scene/shared.ts';
import { aimRoomLights, applyTheme, createStage } from './scene/stage.ts';
import { demandLoop, onContextLost, releaseRenderer, watchVisibility } from '../../../lib/three/stage.ts';
import { schoolHasWork } from './scene/loop.ts';

export * from './scene/api.ts';

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
  const stage = createStage(T, assets, RoomEnvironment, hdr, host, opts);
  const { renderer, canvas, scene, pmrem, envRT, camera, lamps, corridorLamps, ceilGeo, ceilMat, ceiling } = stage;

  const corridorGroup = new T.Group();
  const roomGroup = new T.Group();
  scene.add(corridorGroup, roomGroup);

  // ── Kit instancing ────────────────────────────────────────────────────────
  const { tmpM, tmpR, instance, liveChildren, clearGroup } = createKit(T, assets);

  // The one 2× monitor canvas (the focused kid's screen borrows it).
  const hi = hiResCanvas(T);

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

  const hitGeo = new T.BoxGeometry(1, 1, 1);
  const hitMat = new T.MeshBasicMaterial({ visible: false });
  const { selRing, hoverRing, ringGeo, ringMat, hoverMat, pinGeo, pinMat, pins, callGeo, callMat, calls, updatePins, placeRing } = createMarkers(
    T,
    scene,
    assets,
    characters,
    life,
    reduced,
  );

  // ── Corridor ─────────────────────────────────────────────────────────────
  function buildCorridor(s: School): void {
    clearGroup(corridorGroup);
    doors.clear();
    const placements = furnishCorridor(s.rooms, s.corridorLength);
    instance(corridorGroup, placements.filter((p) => p.node !== 'Corridor_Door'), true);
    ceiling(corridorGroup, s.corridorLength / 2, 0, s.corridorLength, CORRIDOR_HALF * 2);
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
      if (d.plate) paintPlate(d.plate, r);
      if (d.sign) paintSign(d.sign, r);
    }
    dirty = true;
  }

  // ── Room ─────────────────────────────────────────────────────────────────
  const dimsKey = (r: Room) => `${r.id}|${r.width}|${r.depth}|${r.cols}|${r.rows}|${r.backRows}|${r.cx}`;

  function buildRoomStatics(r: Room): void {
    // Characters live in roomGroup too — detach them across the rebuild.
    const keep = [...characters.values()].map((c) => c.obj).concat(head ? [head.obj] : []);
    for (const o of keep) roomGroup.remove(o);
    // Hand the 2× canvas back first, so each monitor disposes its own texture.
    setHiRes(null);
    clearGroup(roomGroup);
    screens.clear();
    chairBack.clear();
    roomDoorPivots = [];
    const placements = furnishRoom(r);
    chairs = instance(roomGroup, placements, true);
    ceiling(roomGroup, r.cx, r.cz, r.width, r.depth);
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
    aimRoomLights(stage, r);
    for (const s of screens.values()) s.key = '';
    for (const [id, info] of screenInfo) applyScreen(id, info);
    paintEmptyDesks(r);
    dirty = true;
  }

  function paintBoard(r: Room): void {
    paintBoardTitle(boardTitle, r);
  }

  function deskOf(kidId: string): string | null {
    const k = openRoom?.kids.find((x) => x.id === kidId);
    return k ? `${k.row}:${k.col}` : null;
  }

  /** The kid whose monitor the camera is on (painted at 2×). */
  function focusKid(): string | null {
    return view.kind === 'screen' ? view.kidId : selected;
  }

  /** Point desk `tag`'s monitor at the shared 2× canvas (null = give it
   *  back to its own canvas and repaint that). */
  function setHiRes(tag: string | null): void {
    if (hi.tag === tag) return;
    const prev = hi.tag ? screens.get(hi.tag) : null;
    hi.tag = tag;
    if (prev) {
      (prev.mesh.material as THREE_NS.MeshBasicMaterial).map = prev.tex;
      prev.key = '';
    }
    const next = tag ? screens.get(tag) : null;
    if (next) {
      (next.mesh.material as THREE_NS.MeshBasicMaterial).map = hi.tex;
      next.key = '';
    }
    dirty = true;
  }

  function applyScreen(kidId: string, info: ScreenInfo): void {
    const tag = deskOf(kidId);
    const scr = tag ? screens.get(tag) : null;
    if (!scr) return;
    const focused = focusKid() === kidId;
    if (focused) setHiRes(tag);
    else if (hi.tag === tag) setHiRes(null);
    const full = { ...info, selected: selected === kidId };
    const key = screenKey(full) + (focused ? '\u0002' : '');
    if (scr.key === key) return;
    scr.key = key;
    if (focused) {
      paintScreen(hi.ctx, full, 2);
      hi.tex.needsUpdate = true;
    } else {
      paintScreen(scr.ctx, full);
      scr.tex.needsUpdate = true;
    }
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
  const { makeCharacter, placeCharacter } = createCast(T, cloneSkinned, hitGeo, hitMat, roomGroup, reduced);

  function syncCharacters(r: Room): void {
    const want = new Map<string, Kid>([...r.kids, ...r.bench].map((k) => [k.id, k]));
    for (const [id, c] of characters) {
      if (want.has(id) || kicks.has(id)) continue;
      disposeActor(c);
      characters.delete(id);
    }
    for (const [id, k] of want) {
      if (characters.has(id)) continue;
      const g = assets.kids.get(k.character)!;
      characters.set(id, makeCharacter(id, g, KID_CLIP_NAMES));
    }
    if (!head) {
      head = makeCharacter('headmaster', assets.head, HEAD_CLIPS);
      // A touch smaller than the kit's: he patrols between the desks without
      // hiding a whole row behind him.
      head.obj.scale.setScalar(0.85);
    }
  }

  function dropCharacters(): void {
    for (const c of characters.values()) {
      disposeActor(c);
    }
    characters.clear();
    if (head) {
      disposeActor(head);
      head = null;
    }
    for (const k of kicks.values()) k.resolve();
    kicks.clear();
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
        tmpM.makeTranslation(0, 0, next).multiply(h.base).multiply(h.rel);
        h.mesh.setMatrixAt(h.index, tmpM);
        h.mesh.instanceMatrix.needsUpdate = true;
      }
    }
  }

  // ── Camera ───────────────────────────────────────────────────────────────
  const rig = createCamera(T, reduced, {
    room: () => openRoom,
    corridorLength: () => school?.corridorLength,
    view: () => view,
    touch: () => {
      dirty = true;
      kick();
    },
  });
  const { cam, keys, lookDir, fly, corridorPose, roomPose, syncOrbitFromCam, syncWalkFromCam, applyWalk, stepInput } = rig;

  // ── Doors ────────────────────────────────────────────────────────────────
  function openDoor(roomId: string, open: boolean): void {
    const d = doors.get(roomId);
    if (!d) return;
    d.target = open ? -Math.PI * 0.55 : 0;
    if (reduced) d.angle = d.target;
    dirty = true;
    kick();
  }

  /** Swings the doors toward their targets; true while any leaf moved. */
  function stepDoors(dt: number): boolean {
    let moved = false;
    for (const d of doors.values()) {
      const pivots = openRoom?.id === d.roomId ? [...d.pivots, ...roomDoorPivots] : d.pivots;
      if (Math.abs(d.angle - d.target) < 1e-3 && pivots.every((p) => p.obj.userData.angle === d.angle)) continue;
      d.angle += (d.target - d.angle) * Math.min(1, dt / 140);
      if (Math.abs(d.angle - d.target) < 0.002) d.angle = d.target;
      tmpR.makeRotationY(d.angle);
      for (const p of pivots) {
        p.obj.matrix.copy(p.base).multiply(tmpR);
        p.obj.matrixWorldNeedsUpdate = true;
        p.obj.userData.angle = d.angle;
      }
      dirty = true;
      moved = true;
    }
    return moved;
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
    setHiRes(null);
    clearGroup(roomGroup);
    screens.clear();
    pins.count = 0;
    calls.count = 0;
    corridorLamps();
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
      // The monitor we lean over switches to the crisp 2× canvas.
      const info = screenInfo.get(kidId);
      if (info) applyScreen(kidId, info);
    });
  }

  // ── Picking / projection ─────────────────────────────────────────────────
  const { pick, project, visibleScreens, facingDoor } = createPicker(
    T,
    { canvas, camera, roomGroup, doors, characters, screens, rig: { cam, lookDir } },
    { view: () => view, school: () => school, room: () => openRoom, head: () => head },
  );

  // ── Loop ─────────────────────────────────────────────────────────────────
  let active = true;
  /** In (or near) the viewport and the document is visible. */
  let visible = true;
  let last = performance.now();
  let acc = 0;
  /** Uncapped time since the last drawn frame (camera flights run on it). */
  let accRaw = 0;
  const frameFns: (() => void)[] = [];
  const lostFns: (() => void)[] = [];

  /** One frame; true while there is more work (the loop re-arms only then). */
  function frame(now: number): boolean {
    const raw = Math.max(0, now - last);
    const dt = Math.min(1000, raw);
    last = now;
    acc += dt;
    accRaw += raw;
    // Below the 30 fps step: wait for the next display frame.
    if (acc < FPS_MS) return true;
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
    const doorsMoving = stepDoors(step);
    // Life.
    const r = openRoom;
    if (r) {
      if (!reduced) {
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
        dirty = true;
      } else if (dirty) {
        for (const c of characters.values()) {
          const v = life.kidView(c.id);
          if (v) placeCharacter(c, v, r, 0);
          c.mixer.update(0);
        }
        const hv = life.headView();
        if (head && hv) {
          placeCharacter(head, hv, r, 0);
          head.mixer.update(0);
        }
      }
      for (const [id, k] of kicks) {
        if (life.isGone(id) || now > k.until) {
          kicks.delete(id);
          const c = characters.get(id);
          if (c) c.obj.visible = false;
          k.resolve();
          dirty = true;
        }
      }
      placeRing(selRing, selected);
      placeRing(hoverRing, hover?.kind === 'kid' && hover.id !== selected ? hover.id : null);
      // Leaning over one monitor, a bead the size of a fist would block it.
      pins.visible = calls.visible = view.kind !== 'screen';
      if (!reduced || dirty) updatePins(r, now);
    }
    if (dirty) {
      camera.position.copy(cam.pos);
      camera.lookAt(cam.target);
      renderer.render(scene, camera);
      frames++;
      dirty = false;
      for (const f of frameFns) f();
    }
    return schoolHasWork({ tween: !!cam.tween, keys: keys.size, roomOpen: !!openRoom, reduced, kicks: kicks.size, doorsMoving });
  }
  const loop = demandLoop(frame, { canRun: () => active && visible });

  function kick(): void {
    // Restarting from idle: time the first step from now, not from the
    // last frame minutes ago (life steps would jump).
    if (!loop.pending) last = performance.now();
    loop.kick();
  }

  // Native WebKit deliberately disables background throttling for terminals.
  // The School must pause explicitly (hidden document, or the stage scrolled
  // off screen); losing focus alone is not a reason to stop a still-visible
  // window. Preserve the room and camera while hidden.
  const unwatch = watchVisibility(host, (v) => {
    visible = v;
    if (!v) {
      keys.clear();
      loop.stop();
    } else {
      dirty = true;
      kick();
    }
  }, '120px 0px');

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
  onContextLost(canvas, () => {
    for (const f of lostFns) f();
  });

  /** Light theme = a sunny afternoon; dark theme = evening (stage.ts). */
  function setTheme(dark: boolean): void {
    applyTheme(T, stage, dark);
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
      if (!id && view.kind !== 'screen') setHiRes(null);
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
      if (rig.drag(dx, dy)) kick();
    },
    wheel(dy) {
      if (rig.wheel(dy)) kick();
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
        loop.stop();
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
      return schoolDebug({ view, frames, assets, life, screens, openRoom, cam, hiTag: hi.tag, lamps: lamps.length });
    },
    destroy() {
      unwatch();
      active = false;
      loop.stop();
      ro.disconnect();
      dropCharacters();
      setHiRes(null);
      clearGroup(roomGroup);
      clearGroup(corridorGroup);
      ringGeo.dispose();
      ringMat.dispose();
      pinGeo.dispose();
      pinMat.dispose();
      pins.dispose();
      callGeo.dispose();
      callMat.dispose();
      calls.dispose();
      ceilGeo.dispose();
      ceilMat.dispose();
      hi.tex.dispose();
      for (const l of lamps) l.dispose();
      hoverMat.dispose();
      hitGeo.dispose();
      hitMat.dispose();
      envRT.dispose();
      pmrem.dispose();
      assets.dispose();
      releaseRenderer(renderer);
    },
  };
}
