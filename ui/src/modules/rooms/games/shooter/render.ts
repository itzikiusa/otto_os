/**
 * Storm Royale renderer: island terrain + sea / lake, instanced Kenney props
 * (felled trees hidden until they regrow), the merged walk-in architecture,
 * player structures (one persistent mesh per material, rewritten in place)
 * with a locally predicted build-preview ghost, chests, supply drops, launch
 * pads, floor loot (nearest items only), animated mini-character fighters
 * holding Blaster Kit weapons / heals / the pickaxe (crawling when knocked;
 * teammates ringed and labelled in their team colour), the drop balloon,
 * gliders, tracers, rockets, explosions, debris and the storm wall.
 * Third-person camera with shoulder offset, aim zoom and collision.
 */
import * as THREE from 'three';
import { AssetLibrary, PropBatch } from '../common/assets';
import { GameView, owned, type Theme } from '../common/view';
import { Particles } from '../common/particles';
import { approach, clamp, lerp, smoothstep } from '../common/math';
import { groundDetailTexture, labelTexture } from '../common/textures';
import { chestY, GRID, HALF, SEA_SURFACE, mapFor, terrainHeight, type ShooterMap } from './map';
import { groundAt, raycast, syncFelled, worldFor, type World } from './collide';
import { BUS_HEIGHT, EYE, heldItem, MAX_PADS, PICKAXE_SLOT, rayFighter, type Drop, type Fighter, type Loot, type Pad, type ShooterEvent, type ShooterState } from './sim';
import { BUILD_COST, CONSUMABLES, isConsumable, MATERIAL_COLOURS, RARITY_COLOURS, WEAPONS } from './weapons';
import { canPlace, packBuild, planBuild, type BuildKind } from './builds';
import { FIGHTER_IDS, teamColour, type FighterId } from './catalog';
import { blockGeometry, ghostGeometries, materialTexture, padGeometries, pickaxeGeometries, pickaxeMesh, placeGhost, StructureMesh } from './meshes';

const THEMES: Record<ShooterMap['theme'], Theme & { low: number; mid: number; high: number; rock: number; sand: number }> = {
  isle: { skyTop: 0x3d84d6, skyHorizon: 0xcbe7ff, fog: 0xcfe6fb, fogNear: 140, fogFar: 620, sun: 0xfff2da, sunIntensity: 2.7, ambientSky: 0xd4e8ff, ambientGround: 0x5e7a3c, ambient: 1.1, low: 0x7cc35a, mid: 0x5da746, high: 0x8fbf63, rock: 0x8d8f8c, sand: 0xe8d39a },
  mesa: { skyTop: 0x3c7ccb, skyHorizon: 0xf4d6ab, fog: 0xf0d3a8, fogNear: 140, fogFar: 620, sun: 0xffe3bb, sunIntensity: 2.9, ambientSky: 0xffe6c6, ambientGround: 0x9a5b36, ambient: 1.05, low: 0xd99a5c, mid: 0xc97a45, high: 0xe3b276, rock: 0xa65a34, sand: 0xe7c08a },
};
const CHAR_SCALE = 2.7;
const UPPER = new Set(['arm-left', 'arm-right']);
const LOOT_RANGE = 70;
const _v = new THREE.Vector3();
const _q = new THREE.Quaternion();
const _m4 = new THREE.Matrix4();
const ZERO = new THREE.Matrix4().makeScale(0, 0, 0);
const UP = new THREE.Vector3(0, 1, 0);
const PROBE_HEIGHTS = [1.65, 1.1];
const MAT_KEYS = ['surv-box', 'surv-rock-a', 'surv-metal-panel-screws'];

interface FighterVisual {
  fighter: FighterId; root: THREE.Group; model: THREE.Object3D; mixer: THREE.AnimationMixer; hand: THREE.Object3D | null;
  lower: Map<string, THREE.AnimationAction>; upper: Map<string, THREE.AnimationAction>; die: THREE.AnimationAction;
  lowerName: string; upperName: string; dead: boolean; gun: THREE.Object3D | null; gunKey: string; glider: THREE.Mesh; label: THREE.Sprite | null;
}
interface Tracer { line: THREE.Line; life: number }
/** A seat as the renderer sees it: model, label, and team (for teammate markers). */
export interface SeatView { fighter: FighterId; human: boolean; name: string; team: number }
/**
 * The local survivor's build intent, applied to the preview ghost before the
 * host has echoed it (guests would otherwise see the ghost trail the camera by
 * a round trip). Position is the freshest known one.
 */
export interface LocalBuild { building: boolean; piece: number; mat: number; yaw: number; pitch: number; x: number; y: number; z: number }

export interface CameraState { yaw: number; pitch: number; aiming: boolean; zoom: number }

export class ShooterRenderer {
  readonly view: GameView;
  private lib: AssetLibrary;
  private map: ShooterMap;
  private world: World;
  private fighters: FighterVisual[] = [];
  private loot = new Map<number, THREE.Object3D>();
  private lootCheck = 0;
  private chests: THREE.Object3D[] = [];
  private rockets = new Map<number, THREE.Object3D>();
  private tracers: Tracer[] = [];
  private tracerMat = new THREE.LineBasicMaterial({ color: 0xfff1a8, transparent: true, opacity: 0.9, blending: THREE.AdditiveBlending, depthWrite: false });
  private sparks = new Particles(2000, true);
  private smoke = new Particles(700, false);
  private storm: THREE.Mesh;
  private stormMat: THREE.ShaderMaterial;
  private balloon: THREE.Group;
  private beamGeo = new THREE.CylinderGeometry(0.08, 0.3, 3, 8, 1, true);
  private beamMats = RARITY_COLOURS.map(c => new THREE.MeshBasicMaterial({ color: c, transparent: true, opacity: 0.35, blending: THREE.AdditiveBlending, depthWrite: false }));
  private gliderGeo = new THREE.SphereGeometry(1.6, 16, 8, 0, Math.PI * 2, 0, Math.PI / 2.2);
  private gliderMat = new THREE.MeshLambertMaterial({ color: 0xff5a5a, side: THREE.DoubleSide });
  /** Structures: one persistent mesh per material, rewritten in place (throttled) when the set or their health changes. */
  private structMats = [0, 1, 2].map(() => new THREE.MeshLambertMaterial({ vertexColors: true }));
  private structs = [0, 1, 2].map(() => new StructureMesh());
  private structMeshes: THREE.Mesh[] = [];
  private structRev = -1;
  private structList: unknown = null;
  /** Shape key of the meshed structures (layout, kind, material, edits; not health), overall and per material. */
  private structShape = -1;
  private matShape = new Int32Array([-1, -1, -1]);
  private matCount = new Int32Array(3);
  private matKey = new Int32Array(3);
  private structAge = 0;
  private structCount = 0;
  /** Felled trees: the instanced meshes + instance drawing each trunk, and the matrices hidden away. */
  private trees: { meshes: THREE.InstancedMesh[]; index: number }[] = [];
  private felledFrom: readonly number[] | null = null;
  private felledShown = new Uint8Array(0);
  private treeSaved = new Map<number, THREE.Matrix4[]>();
  /** Launch pads (instanced) and supply drops. */
  private padGeos = padGeometries();
  private padBase = new THREE.MeshLambertMaterial({ color: 0x3b4656 });
  private padTop = new THREE.MeshBasicMaterial({ color: 0x62e0ff });
  private pads: THREE.InstancedMesh[] = [];
  private padKey = NaN;
  private drops = new Map<number, THREE.Group>();
  private dropBalloon = new THREE.SphereGeometry(2.6, 16, 10);
  private dropBalloonMat = new THREE.MeshLambertMaterial({ color: 0x4aa8ff });
  /** Teammate ring under the feet. */
  private ringGeo = new THREE.RingGeometry(0.62, 0.86, 28).rotateX(-Math.PI / 2);
  private ringMats = new Map<number, THREE.MeshBasicMaterial>();
  private ghosts: THREE.Mesh[] = [];
  private ghostOk = new THREE.MeshBasicMaterial({ color: 0x62b4ff, transparent: true, opacity: 0.35, depthWrite: false });
  private ghostBad = new THREE.MeshBasicMaterial({ color: 0xff5a5a, transparent: true, opacity: 0.3, depthWrite: false });
  private pickGeos = pickaxeGeometries();
  private pickHandle = new THREE.MeshLambertMaterial({ color: 0x8a5a2b });
  private pickHead = new THREE.MeshLambertMaterial({ color: 0xb8c4cc });
  private ground = (x: number, z: number, feet: number) => groundAt(this.world, x, z, feet);
  private time = 0;
  private camPos = new THREE.Vector3();
  private self: number;
  private seats: SeatView[];
  /** Team modes: teammates get rings and labels; enemies get neither. */
  private teams: boolean;
  private alive = new Set<number>();
  aimPoint = new THREE.Vector3();
  /** Whether the current build preview can be placed (for the HUD). */
  ghostValid = false;

  constructor(host: HTMLElement, mapId: string, seats: SeatView[], self: number, signal: AbortSignal) {
    this.map = mapFor(mapId);
    this.world = worldFor(this.map);
    this.view = new GameView(host, THEMES[this.map.theme]);
    this.view.camera.far = 1400;
    this.lib = new AssetLibrary(signal);
    this.seats = seats;
    this.self = self;
    this.teams = seats.some((s, i) => i !== self && s.team === seats[self]?.team);
    this.stormMat = new THREE.ShaderMaterial({
      transparent: true, side: THREE.DoubleSide, depthWrite: false,
      uniforms: { time: { value: 0 } },
      vertexShader: 'varying vec2 vUv; varying vec3 vPos; void main(){ vUv = uv; vPos = position; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }',
      fragmentShader: `uniform float time; varying vec2 vUv; varying vec3 vPos;
        void main(){ float band = sin(vUv.x * 160.0 + time * 1.5) * 0.5 + 0.5; float swirl = sin(vUv.y * 24.0 - time * 2.0 + vUv.x * 40.0) * 0.5 + 0.5;
          float a = 0.32 + band * 0.12 + swirl * 0.1; a *= smoothstep(0.0, 0.05, vUv.y) * (1.0 - smoothstep(0.85, 1.0, vUv.y));
          gl_FragColor = vec4(mix(vec3(0.42, 0.18, 0.78), vec3(0.75, 0.45, 1.0), swirl), a); }`,
    });
    this.storm = new THREE.Mesh(new THREE.CylinderGeometry(1, 1, 260, 128, 1, true), this.stormMat);
    this.storm.frustumCulled = false;
    this.storm.renderOrder = 4;
    this.balloon = new THREE.Group();
  }

  async load(): Promise<void> {
    // With guests aboard every survivor is loaded: guests may switch during the drop.
    const guests = this.seats.some((s, i) => s.human && i !== this.self);
    await Promise.all([this.lib.loadKit('shooter-kit.glb'), this.lib.loadCharacters(guests ? FIGHTER_IDS : [...new Set(this.seats.map(s => s.fighter))])]);
    this.buildWorld();
    this.seats.forEach((seat, i) => this.fighters.push(this.buildFighter(seat, i)));
    this.view.scene.add(this.sparks.points, this.smoke.points, this.storm);
    this.structMats.forEach((m, i) => { m.map = this.lib.own(materialTexture(i)); });
    this.structMeshes = this.structs.map((st, i) => {
      const mesh = new THREE.Mesh(st.geometry, this.structMats[i]);
      mesh.castShadow = true; mesh.receiveShadow = true; mesh.visible = false;
      this.view.scene.add(mesh);
      return mesh;
    });
    this.pads = [this.padBase, this.padTop].map((m, i) => {
      const mesh = new THREE.InstancedMesh(this.padGeos[i], m, MAX_PADS);
      mesh.count = 0; mesh.frustumCulled = false; mesh.receiveShadow = true;
      this.view.scene.add(mesh);
      return mesh;
    });
    this.ghosts = ghostGeometries().map(g => {
      const mesh = new THREE.Mesh(this.lib.own(g), this.ghostOk);
      mesh.visible = false; mesh.renderOrder = 5;
      this.view.scene.add(mesh);
      return mesh;
    });
    for (const r of [this.tracerMat, this.beamGeo, ...this.beamMats, this.gliderGeo, this.gliderMat, this.stormMat, this.storm.geometry, ...this.structMats, ...this.structs, this.ghostOk, this.ghostBad, ...this.pickGeos, this.pickHandle, this.pickHead,
      ...this.padGeos, this.padBase, this.padTop, this.dropBalloon, this.dropBalloonMat, this.ringGeo]) this.lib.own(r);
  }

  private buildWorld(): void {
    const map = this.map, theme = THEMES[map.theme], scene = this.view.scene;
    const n = map.size;
    const pos = new Float32Array(n * n * 3), col = new Float32Array(n * n * 3), uv = new Float32Array(n * n * 2);
    const c = new THREE.Color(), low = new THREE.Color(theme.low), mid = new THREE.Color(theme.mid), high = new THREE.Color(theme.high), rock = new THREE.Color(theme.rock), sand = new THREE.Color(theme.sand);
    const lake = map.waters[0];
    for (let iz = 0; iz < n; iz++) for (let ix = 0; ix < n; ix++) {
      const k = iz * n + ix, x = -HALF + ix * GRID, z = -HALF + iz * GRID, h = map.heights[k];
      pos[k * 3] = x; pos[k * 3 + 1] = h; pos[k * 3 + 2] = z;
      uv[k * 2] = x / 5; uv[k * 2 + 1] = z / 5;
      const hx = map.heights[iz * n + Math.min(n - 1, ix + 1)] - map.heights[iz * n + Math.max(0, ix - 1)];
      const hz = map.heights[Math.min(n - 1, iz + 1) * n + ix] - map.heights[Math.max(0, iz - 1) * n + ix];
      const slope = Math.hypot(hx, hz) / (GRID * 2);
      c.copy(low).lerp(mid, smoothstep(2, 9, h)).lerp(high, smoothstep(9, 16, h) * 0.6);
      // Meadow patches and worn dirt for variety.
      const patch = Math.sin(x * 0.083 + Math.cos(z * 0.05) * 2) * Math.cos(z * 0.071 + Math.sin(x * 0.04) * 2);
      c.lerp(map.theme === 'isle' ? mid : high, smoothstep(0.2, 0.8, patch) * 0.5);
      c.multiplyScalar(0.92 + 0.08 * Math.sin(x * 0.31) * Math.sin(z * 0.27));
      if (map.theme === 'isle') c.lerp(sand, 1 - smoothstep(0.6, 2.2, h));
      else {
        c.lerp(sand, smoothstep(0.3, 0.6, Math.sin(x * 0.05) * Math.cos(z * 0.04) * 0.5 + 0.5) * 0.35);
        // Wet sand and reeds-green around the oasis.
        if (lake) c.lerp(sand, 1 - smoothstep(lake.y + 0.2, lake.y + 1.6, h)).lerp(mid, smoothstep(lake.r + 10, lake.r, Math.hypot(x - lake.x, z - lake.z)) * 0.25);
      }
      c.lerp(rock, smoothstep(0.45, 0.9, slope));
      col[k * 3] = c.r; col[k * 3 + 1] = c.g; col[k * 3 + 2] = c.b;
    }
    const index: number[] = [];
    for (let iz = 0; iz < n - 1; iz++) for (let ix = 0; ix < n - 1; ix++) { const a = iz * n + ix, b = a + 1, cc = a + n, d = cc + 1; index.push(a, cc, b, b, cc, d); }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(pos, 3));
    geo.setAttribute('color', new THREE.BufferAttribute(col, 3));
    geo.setAttribute('uv', new THREE.BufferAttribute(uv, 2));
    geo.setIndex(index); geo.computeVertexNormals();
    const detail = this.lib.own(groundDetailTexture());
    const terrain = owned(new THREE.Mesh(geo, new THREE.MeshLambertMaterial({ vertexColors: true, map: detail })));
    terrain.receiveShadow = true;
    scene.add(terrain);
    if (map.theme === 'isle') {
      const sea = owned(new THREE.Mesh(new THREE.PlaneGeometry(3000, 3000), new THREE.MeshLambertMaterial({ color: 0x2e8fd0, transparent: true, opacity: 0.88 })));
      sea.rotation.x = -Math.PI / 2; sea.position.y = SEA_SURFACE;
      scene.add(sea);
    } else {
      // Desert floor beyond the playable cliffs, and the oasis water.
      const floor = owned(new THREE.Mesh(new THREE.PlaneGeometry(3000, 3000), new THREE.MeshLambertMaterial({ color: theme.mid })));
      floor.rotation.x = -Math.PI / 2; floor.position.y = -1;
      scene.add(floor);
      for (const w of map.waters) {
        const water = owned(new THREE.Mesh(new THREE.CircleGeometry(w.r, 64), new THREE.MeshLambertMaterial({ color: 0x2f9ad6, transparent: true, opacity: 0.86 })));
        water.rotation.x = -Math.PI / 2; water.position.set(w.x, w.y, w.z);
        scene.add(water);
      }
    }
    // Walk-in architecture: one merged mesh.
    if (map.blocks.length) {
      const arch = owned(new THREE.Mesh(blockGeometry(map.blocks), new THREE.MeshLambertMaterial({ vertexColors: true, map: detail })));
      arch.castShadow = true; arch.receiveShadow = true;
      scene.add(arch);
    }
    const batch = new PropBatch(this.lib);
    // Instance index of every prop within its model's batch (trees hide theirs once felled).
    const perKey = new Map<string, number>(), instance = new Int32Array(map.props.length);
    map.props.forEach((p, i) => { const n = perKey.get(p.key) ?? 0; instance[i] = n; perKey.set(p.key, n + 1); batch.add(p.key, p.x, p.y - 0.05, p.z, p.yaw, p.scale); });
    const group = new THREE.Group();
    batch.build(group, true);
    scene.add(group);
    const byKey = new Map<string, THREE.InstancedMesh[]>();
    for (const o of group.children) if (o instanceof THREE.InstancedMesh) { const l = byKey.get(o.name) ?? []; l.push(o); byKey.set(o.name, l); }
    this.trees = map.trunkProps.map(prop => ({ meshes: byKey.get(map.props[prop].key) ?? [], index: instance[prop] }));
    this.felledShown = new Uint8Array(map.cylinders.length);
    for (const ch of map.chests) {
      const o = this.lib.instance('gun-crate-wide');
      o.scale.setScalar(2.4);
      o.position.set(ch.x, chestY(map, ch), ch.z);
      o.rotation.y = ch.yaw;
      scene.add(o);
      this.chests.push(o);
    }
    // Drop balloon: Kenney delivery van slung under a striped envelope.
    const van = this.lib.instance('car-delivery');
    van.scale.setScalar(3.4);
    const envelope = owned(new THREE.Mesh(new THREE.SphereGeometry(8, 24, 16), new THREE.MeshLambertMaterial({ color: 0x4c7bff })));
    envelope.position.y = 17; envelope.scale.set(1, 1.15, 1);
    const band = owned(new THREE.Mesh(new THREE.TorusGeometry(8.1, 0.7, 8, 32), new THREE.MeshLambertMaterial({ color: 0xffd23f })));
    band.position.y = 17; band.rotation.x = Math.PI / 2;
    this.balloon.add(van, envelope, band);
    scene.add(this.balloon);
  }

  private ringMat(team: number): THREE.MeshBasicMaterial {
    let m = this.ringMats.get(team);
    if (!m) { m = this.lib.own(new THREE.MeshBasicMaterial({ color: teamColour(team), transparent: true, opacity: 0.85, depthWrite: false })); this.ringMats.set(team, m); }
    return m;
  }

  private buildFighter(seat: SeatView, index: number): FighterVisual {
    const root = new THREE.Group();
    const model = this.lib.character(seat.fighter);
    model.scale.setScalar(CHAR_SCALE);
    root.add(model);
    const mixer = new THREE.AnimationMixer(model);
    const split = (name: string, upper: boolean) => {
      const clip = this.lib.clip(name);
      const tracks = clip.tracks.filter(t => UPPER.has(t.name.split('.')[0]) === upper);
      return mixer.clipAction(new THREE.AnimationClip(`${name}-${upper ? 'u' : 'l'}`, clip.duration, tracks));
    };
    const lower = new Map<string, THREE.AnimationAction>(), upper = new Map<string, THREE.AnimationAction>();
    for (const n of ['idle', 'walk', 'sprint', 'jump', 'fall']) lower.set(n, split(n, false));
    for (const n of ['idle', 'walk', 'sprint', 'fall', 'holding-both', 'holding-both-shoot', 'holding-right', 'holding-right-shoot']) upper.set(n, split(n, true));
    const die = mixer.clipAction(this.lib.clip('die'));
    die.setLoop(THREE.LoopOnce, 1); die.clampWhenFinished = true;
    lower.get('idle')!.play(); upper.get('idle')!.play();
    let hand: THREE.Object3D | null = null;
    model.traverse(o => { if (o.name === 'arm-right') hand = o; });
    const glider = new THREE.Mesh(this.gliderGeo, this.gliderMat);
    glider.position.y = 3.6; glider.visible = false;
    root.add(glider);
    let label: THREE.Sprite | null = null;
    // Teammates (team modes) are labelled in the team colour and ringed; in solo, other people are labelled.
    const mate = this.teams && index !== this.self && seat.team === this.seats[this.self]?.team;
    if ((seat.human && index !== this.self && !this.teams) || mate) {
      const tint = mate ? `#${teamColour(seat.team).toString(16).padStart(6, '0')}` : '#ffffff';
      label = new THREE.Sprite(this.lib.own(new THREE.SpriteMaterial({ map: this.lib.own(labelTexture(seat.name, tint)), depthTest: false, transparent: true })));
      label.scale.set(3.2, 0.8, 1); label.position.y = 2.5; label.renderOrder = 10;
      root.add(label);
    }
    if (mate || (this.teams && index === this.self)) {
      const ring = new THREE.Mesh(this.ringGeo, this.ringMat(seat.team));
      ring.position.y = 0.06; ring.renderOrder = 3;
      root.add(ring);
    }
    this.view.scene.add(root);
    return { fighter: seat.fighter, root, model, mixer, hand, lower, upper, die, lowerName: 'idle', upperName: 'idle', dead: false, gun: null, gunKey: '', glider, label };
  }

  effect(e: ShooterEvent, state: ShooterState): void {
    switch (e.type) {
      case 'shot': {
        if (e.value === 5) break; // rockets draw their own trail
        const geo = new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(e.x, e.y, e.z), new THREE.Vector3(e.x2, e.y2, e.z2)]);
        const line = new THREE.Line(geo, this.tracerMat);
        this.view.scene.add(line);
        this.tracers.push({ line, life: 0.07 });
        this.sparks.spawn(e.x, e.y, e.z, 0, 0, 0, 0xffe08a, 0.7, 0.05);
        this.sparks.burst(e.x2, e.y2, e.z2, 4, 0xffd27a, 3, 0.25, 0.2);
        break;
      }
      case 'hit': this.sparks.burst(e.x, e.y, e.z, e.flag & 1 ? 10 : 7, e.flag & 1 ? 0x6fc6ff : 0xff5a4a, 4, 0.35, 0.35); break;
      case 'explode':
        this.sparks.burst(e.x, e.y, e.z, 90, 0xffa040, 18, 1.4, 0.7);
        for (let i = 0; i < 26; i++) this.smoke.spawn(e.x, e.y + 0.5, e.z, (Math.random() - 0.5) * 8, 2 + Math.random() * 4, (Math.random() - 0.5) * 8, 0x4d4d4d, 4, 1.6, 0, 1.6);
        break;
      case 'chest': this.sparks.burst(e.x, e.y + 0.8, e.z, 40, 0xffd34d, 7, 0.6, 0.8); break;
      case 'land': for (let i = 0; i < 12; i++) this.smoke.spawn(e.x, e.y + 0.2, e.z, (Math.random() - 0.5) * 6, 0.8, (Math.random() - 0.5) * 6, 0xcbb894, 1.6, 0.8, 0, 2); break;
      case 'kill': this.sparks.burst(e.x, e.y + 1, e.z, 50, 0xd9a8ff, 6, 0.6, 1.0, 2); break;
      case 'build': this.sparks.burst(e.x, e.y + 0.4, e.z, 16, 0x8fd0ff, 3, 0.4, 0.5); break;
      case 'edit': this.sparks.burst(e.x, e.y, e.z, 14, 0xffffff, 3, 0.3, 0.4); break;
      case 'break':
        this.sparks.burst(e.x, e.y, e.z, 40, MATERIAL_COLOURS[e.flag] ?? 0xc8955a, 7, 0.6, 0.9, 2);
        for (let i = 0; i < 10; i++) this.smoke.spawn(e.x, e.y, e.z, (Math.random() - 0.5) * 5, Math.random() * 2, (Math.random() - 0.5) * 5, 0xb7ab98, 2.2, 1.1, 0, 1.4);
        break;
      case 'harvest': this.sparks.burst(e.x, e.y, e.z, 8, MATERIAL_COLOURS[e.value] ?? 0xc8955a, 3, 0.3, 0.5, 2); break;
      case 'use': this.sparks.burst(e.x, e.y + (e.flag === 1 ? 0.4 : 1.2), e.z, 24, e.value === 2 ? 0x62e0ff : e.value === 1 ? 0x62b4ff : 0x6fe07a, 2.5, 0.4, 0.8); break;
      case 'knock': this.sparks.burst(e.x, e.y + 0.6, e.z, 36, 0xff5a4a, 5, 0.5, 0.8, 2); break;
      case 'revive': this.sparks.burst(e.x, e.y + 0.8, e.z, 40, 0x6fe07a, 4, 0.6, 1, -1); break;
      case 'launch':
        this.sparks.burst(e.x, e.y + 0.4, e.z, 30, 0x62e0ff, 8, 0.4, 0.6);
        for (let i = 0; i < 8; i++) this.smoke.spawn(e.x, e.y + 0.2, e.z, (Math.random() - 0.5) * 4, 1.5, (Math.random() - 0.5) * 4, 0xd8e6ee, 1.4, 0.7, 0, 1.6);
        break;
      case 'fell':
        this.sparks.burst(e.x, e.y + 1.5, e.z, 50, 0x6b8f3a, 6, 0.8, 1.2, 2);
        for (let i = 0; i < 10; i++) this.smoke.spawn(e.x, e.y + 1, e.z, (Math.random() - 0.5) * 4, Math.random() * 2, (Math.random() - 0.5) * 4, 0xa58a62, 1.8, 1, 0, 1.4);
        break;
      case 'supply':
        if (e.flag === 1) for (let i = 0; i < 16; i++) this.smoke.spawn(e.x, e.y + 0.3, e.z, (Math.random() - 0.5) * 7, 0.6, (Math.random() - 0.5) * 7, 0xcbb894, 2, 0.9, 0, 2);
        if (e.flag === 2) this.sparks.burst(e.x, e.y + 1, e.z, 60, 0x62b4ff, 7, 0.7, 0.9);
        break;
      default: break;
    }
    void state;
  }

  /** Renders a frame; returns the world point under the crosshair. `local` drives the build ghost. */
  frame(state: ShooterState, selfIndex: number, cam: CameraState, dt: number, spectate: number, local?: LocalBuild): THREE.Vector3 {
    this.time += dt;
    syncFelled(this.world, state.felled);
    this.syncTrees(state.felled);
    this.syncPads(state.pads);
    this.syncDrops(state.drops);
    this.stormMat.uniforms.time.value = this.time;
    const s = state.storm;
    // The wall appears once the first circle is announced.
    this.storm.visible = s.stage >= 0 || s.timer < 20;
    this.storm.position.set(s.cx, 80, s.cz);
    this.storm.scale.set(Math.max(0.5, s.r), 1, Math.max(0.5, s.r));
    // Balloon follows the flight path while anyone is aboard.
    const t = clamp(state.bus.t / state.bus.duration, 0, 1);
    this.balloon.visible = state.phase === 'bus' || state.players.some(p => p.mode === 'bus');
    this.balloon.position.set(lerp(state.bus.ax, state.bus.bx, t), BUS_HEIGHT - 9, lerp(state.bus.az, state.bus.bz, t));
    this.balloon.rotation.y = Math.atan2(state.bus.bx - state.bus.ax, state.bus.bz - state.bus.az);
    state.players.forEach((p, i) => {
      const v = this.fighters[i];
      if (!v) return;
      if (v.fighter !== p.fighter) {
        v.mixer.stopAllAction(); v.root.removeFromParent();
        this.seats[i] = { ...this.seats[i], fighter: p.fighter };
        this.fighters[i] = this.buildFighter(this.seats[i], i);
      }
      this.updateFighter(this.fighters[i], p, dt);
    });
    this.padTop.color.setHSL(0.53, 0.9, 0.55 + Math.sin(this.time * 5) * 0.12);
    state.chests.forEach((open, i) => { const o = this.chests[i]; if (o) { o.scale.y = open ? 1.2 : 2.4; if (!open && Math.random() < 0.05) this.sparks.spawn(o.position.x + (Math.random() - 0.5) * 2, o.position.y + 1, o.position.z + (Math.random() - 0.5) * 2, 0, 1.2, 0, 0xffd34d, 0.35, 0.8); } });
    this.lootCheck -= dt;
    if (this.lootCheck <= 0) { this.lootCheck = 0.2; this.syncLoot(state.loot); }
    for (const [, o] of this.loot) { o.rotation.y += dt * 1.2; }
    this.syncRockets(state);
    this.syncStructures(state, dt);
    this.updateGhost(state, state.players[selfIndex], local);
    for (let i = this.tracers.length - 1; i >= 0; i--) {
      const tr = this.tracers[i];
      tr.life -= dt;
      if (tr.life <= 0) { tr.line.removeFromParent(); tr.line.geometry.dispose(); this.tracers.splice(i, 1); }
    }
    this.sparks.update(dt); this.smoke.update(dt);
    const focus = state.players[spectate] ?? state.players[selfIndex];
    const aim = this.updateCamera(state, focus, cam, dt, spectate === selfIndex);
    this.view.focusShadow(focus.x, focus.y, focus.z);
    this.view.render();
    return aim;
  }

  /**
   * Rewrites the structure meshes when the list or its revision changed: new,
   * removed or edited pieces show at once (only the materials whose layout
   * changed are rewritten); health-only changes (damage, finished growing)
   * are batched to four times a second and only touch the colours.
   */
  private syncStructures(state: ShooterState, dt: number): void {
    this.structAge += dt;
    if (state.buildRev === this.structRev && state.builds === this.structList) return;
    let shape = state.builds.length;
    const ms = this.matCount, keys = this.matKey;
    ms[0] = ms[1] = ms[2] = 0; keys[0] = 3; keys[1] = 5; keys[2] = 7;
    for (const b of state.builds) {
      const code = packBuild(b);
      shape = (Math.imul(shape, 31) + code) | 0;
      keys[b.mat] = (Math.imul(keys[b.mat], 31) + code) | 0;
      ms[b.mat]++;
    }
    if (this.structAge < 0.25 && shape === this.structShape) return;
    const coloursOnly = shape === this.structShape;
    this.structRev = state.buildRev; this.structList = state.builds; this.structShape = shape; this.structAge = 0; this.structCount = state.builds.length;
    for (let m = 0; m < 3; m++) {
      const mesh = this.structMeshes[m];
      if (!mesh) continue;
      mesh.visible = ms[m] > 0;
      if (!ms[m]) { this.matShape[m] = keys[m]; continue; }
      if (coloursOnly || keys[m] === this.matShape[m]) this.structs[m].update(state.builds, m, true);
      else this.structs[m].update(state.builds, m, false);
      this.matShape[m] = keys[m];
    }
  }

  /** Hides felled trees (zero-scale instance) and brings regrown ones back; only on change. */
  private syncTrees(list: readonly number[]): void {
    if (list === this.felledFrom) return;
    this.felledFrom = list;
    const want = this.world.felled;
    for (let i = 0; i < this.trees.length; i++) {
      if (want[i] === this.felledShown[i]) continue;
      this.felledShown[i] = want[i];
      const t = this.trees[i];
      if (want[i]) {
        const saved = t.meshes.map(m => { const mx = new THREE.Matrix4(); m.getMatrixAt(t.index, mx); m.setMatrixAt(t.index, ZERO); m.instanceMatrix.needsUpdate = true; return mx; });
        this.treeSaved.set(i, saved);
      } else {
        const saved = this.treeSaved.get(i);
        if (saved) t.meshes.forEach((m, k) => { m.setMatrixAt(t.index, saved[k]); m.instanceMatrix.needsUpdate = true; });
        this.treeSaved.delete(i);
      }
    }
  }

  /** Launch pads: instance matrices rewritten only when the set moves. */
  private syncPads(pads: readonly Pad[]): void {
    let key = pads.length;
    for (const p of pads) key = key * 1.0001 + p.x * 3.1 + p.z * 7.3 + p.y;
    if (key === this.padKey) return;
    this.padKey = key;
    const n = Math.min(pads.length, MAX_PADS);
    for (const mesh of this.pads) {
      for (let i = 0; i < n; i++) { _m4.makeTranslation(pads[i].x, pads[i].y, pads[i].z); mesh.setMatrixAt(i, _m4); }
      mesh.count = n; mesh.instanceMatrix.needsUpdate = true;
    }
  }

  /** Supply drops: a big crate under a balloon while it falls, with a beam so it can be found. */
  private syncDrops(drops: readonly Drop[]): void {
    const seen = this.alive;
    seen.clear();
    for (const d of drops) {
      seen.add(d.id);
      let g = this.drops.get(d.id);
      if (!g) {
        g = new THREE.Group();
        const crate = this.lib.instance('gun-crate-wide');
        crate.scale.setScalar(3.4);
        const balloon = new THREE.Mesh(this.dropBalloon, this.dropBalloonMat);
        balloon.position.y = 6; balloon.name = 'balloon';
        const beam = new THREE.Mesh(this.beamGeo, this.beamMats[4]);
        beam.scale.set(3, 30, 3); beam.position.y = 45;
        g.add(crate, balloon, beam);
        this.view.scene.add(g);
        this.drops.set(d.id, g);
      }
      g.position.set(d.x, d.y, d.z);
      g.rotation.y = d.landed ? 0 : this.time * 0.6;
      const balloon = g.getObjectByName('balloon');
      if (balloon) balloon.visible = !d.landed;
      if (Math.random() < 0.08) this.sparks.spawn(d.x + (Math.random() - 0.5) * 3, d.y + 1.6, d.z + (Math.random() - 0.5) * 3, 0, 1.4, 0, 0x62b4ff, 0.4, 0.9);
    }
    for (const [id, g] of this.drops) if (!seen.has(id)) { g.removeFromParent(); this.drops.delete(id); }
  }

  /** Blue (placeable) or red ghost for the local survivor's build mode (local intent first, see LocalBuild). */
  private updateGhost(state: ShooterState, p: Fighter | undefined, local?: LocalBuild): void {
    for (const g of this.ghosts) g.visible = false;
    this.ghostValid = false;
    if (!p || p.mode !== 'ground' || p.swimming || !(local ? local.building : p.building)) return;
    const kind = (local ? local.piece : p.piece) as BuildKind, mat = local ? local.mat : p.mat;
    const slot = local ? planBuild(kind, local.x, local.y, local.z, local.yaw, local.pitch, state.builds) : planBuild(kind, p.x, p.y, p.z, p.yaw, p.pitch, state.builds);
    const g = this.ghosts[slot.kind];
    this.ghostValid = p.mats[mat] >= BUILD_COST && canPlace(slot, state.builds, this.ground);
    g.material = this.ghostValid ? this.ghostOk : this.ghostBad;
    placeGhost(g, slot.kind, slot.gx, slot.gz, slot.level, slot.dir);
    g.visible = true;
  }

  private play(map: Map<string, THREE.AnimationAction>, current: string, next: string, fade = 0.15): string {
    if (current === next) return current;
    const from = map.get(current), to = map.get(next);
    if (!to) return current;
    to.reset().setEffectiveWeight(1).fadeIn(fade).play();
    from?.fadeOut(fade);
    return next;
  }

  private updateFighter(v: FighterVisual, p: Fighter, dt: number): void {
    const show = p.mode !== 'bus';
    v.root.visible = show;
    if (!show) return;
    v.root.position.set(p.x, p.y, p.z);
    v.root.rotation.y = p.yaw;
    if (p.mode === 'dead') {
      if (!v.dead) { v.dead = true; v.lower.forEach(a => a.fadeOut(0.1)); v.upper.forEach(a => a.fadeOut(0.1)); v.die.reset().play(); }
      v.mixer.update(dt);
      v.glider.visible = false;
      if (v.gun) v.gun.visible = false;
      return;
    }
    if (v.dead) { v.dead = false; v.die.stop(); v.lowerName = ''; v.upperName = ''; }
    // Knocked: pitched forward on hands and knees, crawling; the label reddens.
    const down = p.mode === 'down';
    v.model.rotation.x = down ? 1.15 : 0;
    v.model.position.y = down ? 0.45 : 0;
    if (v.label) (v.label.material as THREE.SpriteMaterial).color.setHex(down ? 0xff7a6a : 0xffffff);
    const speed = Math.hypot(p.vx, p.vz);
    const air = p.mode === 'sky' || p.mode === 'glide' || (!p.grounded && !p.swimming && !down);
    const lower = air ? (p.vy > 1 ? 'jump' : 'fall') : speed > 6.2 ? 'sprint' : speed > 0.6 ? 'walk' : 'idle';
    v.lowerName = this.play(v.lower, v.lowerName, lower);
    v.lower.get('walk')!.timeScale = clamp(speed / 4, 0.6, 1.6);
    const held = heldItem(p);
    const pick = p.current === PICKAXE_SLOT && !p.building;
    const oneHand = held?.w === 'pistol' || pick || (!!held && isConsumable(held.w));
    const armed = (!!held || pick) && !p.building && !p.swimming && !down;
    const firing = p.shootT > 0 || p.swingT > 0 || p.useT > 0;
    const upper = !armed || p.mode !== 'ground' ? (air ? 'fall' : lower === 'jump' ? 'fall' : lower) : p.sprinting ? 'sprint' : firing ? (oneHand ? 'holding-right-shoot' : 'holding-both-shoot') : oneHand ? 'holding-right' : 'holding-both';
    v.upperName = this.play(v.upper, v.upperName, upper, 0.08);
    v.mixer.update(dt);
    v.glider.visible = p.mode === 'glide';
    // Item in the right hand: a blaster, a heal / shield, or the pickaxe.
    const key = !armed || p.mode !== 'ground' ? '' : pick ? 'pickaxe' : held && isConsumable(held.w) ? (CONSUMABLES[held.w].heal ? 'plat-heart' : 'plat-jewel') : held ? WEAPONS[held.w as keyof typeof WEAPONS].model : '';
    if (key !== v.gunKey) {
      v.gun?.removeFromParent();
      v.gun = null; v.gunKey = key;
      if (key) {
        // Held at chest height in front of the right shoulder, barrel forward
        // (+Z); the holding pose raises the arms to meet it.
        const gun = key === 'pickaxe' ? pickaxeMesh(this.pickHandle, this.pickHead, this.pickGeos) : this.lib.instance(key);
        const small = held?.w === 'pistol';
        if (key === 'pickaxe') { gun.scale.setScalar(0.42); gun.position.set(-0.12, 0.32, 0.16); }
        else if (key.startsWith('plat-')) { gun.scale.setScalar(0.22); gun.position.set(-0.14, 0.3, 0.18); }
        else { gun.scale.setScalar(small ? 0.3 : 0.34); gun.position.set(-0.12, 0.3, small ? 0.19 : 0.16); }
        v.model.add(gun);
        v.gun = gun;
      }
    }
    if (v.gun) {
      v.gun.visible = true;
      // Pickaxe chop: tip the head forward through the swing.
      if (v.gunKey === 'pickaxe') v.gun.rotation.x = p.swingT > 0 ? Math.sin((1 - p.swingT / 0.35) * Math.PI) * 1.4 : 0;
    }
  }

  private syncLoot(items: Loot[]): void {
    const cam = this.view.camera.position;
    const alive = this.alive;
    alive.clear();
    for (const l of items) {
      if (Math.abs(l.x - cam.x) > LOOT_RANGE || Math.abs(l.z - cam.z) > LOOT_RANGE) continue;
      alive.add(l.id);
      if (this.loot.has(l.id)) continue;
      const g = new THREE.Group();
      let key: string, scale: number, lift = 0.5, beam = 0;
      if (l.kind === 'weapon' && l.weapon && l.weapon in WEAPONS) { key = WEAPONS[l.weapon as keyof typeof WEAPONS].model; scale = 1.5; lift = 0.7; beam = l.rarity; }
      else if (l.kind === 'item' && isConsumable(l.weapon)) { key = CONSUMABLES[l.weapon].heal ? 'plat-heart' : 'plat-jewel'; scale = 1.7; beam = l.rarity; }
      else if (l.kind === 'mats') { key = MAT_KEYS[l.mat ?? 0]; scale = l.mat === 2 ? 2.4 : 2.2; lift = 0.1; }
      else if (l.kind === 'ammo') { key = l.ammo === 'shells' || l.ammo === 'rockets' ? 'gun-clip-large' : 'gun-clip-small'; scale = 3.2; }
      else if (l.kind === 'heal') { key = 'plat-heart'; scale = 2.2; beam = 1; }
      else { key = 'plat-jewel'; scale = 2.2; beam = 2; }
      const m = this.lib.instance(key);
      m.scale.setScalar(scale); m.position.y = lift;
      if (l.kind === 'weapon') m.rotation.z = Math.PI / 2.4;
      g.add(m);
      const ray = new THREE.Mesh(this.beamGeo, this.beamMats[beam]);
      ray.position.y = 1.5;
      g.add(ray);
      g.position.set(l.x, l.y, l.z);
      this.view.scene.add(g);
      this.loot.set(l.id, g);
    }
    for (const [id, o] of this.loot) if (!alive.has(id)) { o.removeFromParent(); this.loot.delete(id); }
  }

  private syncRockets(state: ShooterState): void {
    const seen = this.alive;
    seen.clear();
    for (const r of state.rockets) {
      seen.add(r.id);
      let o = this.rockets.get(r.id);
      if (!o) { o = this.lib.instance('gun-bullet-foam-tip-thick'); o.scale.setScalar(14); this.view.scene.add(o); this.rockets.set(r.id, o); }
      o.position.set(r.x, r.y, r.z);
      _v.set(r.vx, r.vy, r.vz).normalize();
      _q.setFromUnitVectors(UP, _v);
      o.quaternion.copy(_q);
      this.smoke.spawn(r.x, r.y, r.z, (Math.random() - 0.5), 0.6, (Math.random() - 0.5), 0x9a9a9a, 1.2, 0.8, 0, 1);
      this.sparks.spawn(r.x, r.y, r.z, 0, 0, 0, 0xffa040, 0.6, 0.1);
    }
    for (const [id, o] of this.rockets) if (!seen.has(id)) { o.removeFromParent(); this.rockets.delete(id); }
  }

  private updateCamera(state: ShooterState, p: Fighter, cam: CameraState, dt: number, own: boolean): THREE.Vector3 {
    const c = this.view.camera;
    // From the balloon the view always looks down at the island.
    const pitch = p.mode === 'bus' ? Math.min(cam.pitch, -0.55) : cam.pitch;
    const cy = Math.cos(pitch), fx = Math.sin(cam.yaw) * cy, fy = Math.sin(pitch), fz = Math.cos(cam.yaw) * cy;
    const rx = -Math.cos(cam.yaw), rz = Math.sin(cam.yaw);
    let back: number, side: number, up: number;
    let px = p.x, pz = p.z, baseY = p.y;
    if (p.mode === 'bus') {
      // Ride behind the balloon basket, looking down at the island.
      back = 26; side = 0; up = 38;
      px = this.balloon.position.x; pz = this.balloon.position.z; baseY = this.balloon.position.y;
    } else if (p.mode === 'sky' || p.mode === 'glide') { back = 10; side = 0; up = 3.5; }
    else if (p.mode === 'dead' || !own) { back = 6.5; side = 0.6; up = 2.2; }
    else if (p.mode === 'down') { back = 4.6; side = 0.8; up = 1.4; }
    else if (cam.aiming) { back = 2.8; side = 1.15; up = 1.95; }
    // Building: pull back a little so the ghost piece is in view.
    else if (p.building) { back = 6.6; side = 1.3; up = 2.6; }
    // Over the right shoulder: the survivor sits in the lower-left third and
    // the crosshair stays clear.
    else { back = 5.6; side = 1.55; up = 2.3; }
    const py = baseY + up;
    const pivotX = px + rx * side, pivotZ = pz + rz * side;
    // Pull the camera in front of walls between the pivot and the desired spot.
    const hit = raycast(this.world, pivotX, py, pivotZ, -fx, -fy, -fz, back + 0.3, false, state.builds);
    const dist = Math.max(0.6, Math.min(back, hit.t - 0.3));
    _v.set(pivotX - fx * dist, py - fy * dist, pivotZ - fz * dist);
    _v.y = Math.max(_v.y, terrainHeight(this.map, _v.x, _v.z) + 0.4);
    // Keep the survivor in view: cast from the head and the chest to the
    // camera; a prop in between (car, barrel, crate stack) pulls the camera in
    // front of it, or lifts it over the prop when that would be too close.
    if (p.mode === 'ground' || p.mode === 'dead' || p.mode === 'down') {
      for (let attempt = 0; attempt < 3; attempt++) {
        let nearest = Infinity, ox = 0, oy = 0, oz = 0, ex = 0, ey = 0, ez = 0, el = 0;
        for (const h of PROBE_HEIGHTS) {
          const sx = p.x, sy = p.y + h, sz = p.z;
          const dx = _v.x - sx, dy = _v.y - sy, dz = _v.z - sz, dl = Math.hypot(dx, dy, dz);
          if (dl < 0.5) continue;
          const block = raycast(this.world, sx, sy, sz, dx / dl, dy / dl, dz / dl, dl, false, state.builds);
          if (block.kind !== 'none' && block.t < dl && block.t < nearest) { nearest = block.t; ox = sx; oy = sy; oz = sz; ex = dx; ey = dy; ez = dz; el = dl; }
        }
        if (nearest === Infinity) break;
        if (nearest - 0.35 >= 2.2 || attempt === 2) {
          const k = Math.max(1.2, nearest - 0.35) / el;
          _v.set(ox + ex * k, oy + ey * k, oz + ez * k);
          break;
        }
        _v.y += 1.6;
      }
    }
    this.camPos.lerp(_v, this.camPos.lengthSq() === 0 ? 1 : 1 - Math.exp(-28 * dt));
    c.position.copy(this.camPos);
    c.lookAt(this.camPos.x + fx, this.camPos.y + fy, this.camPos.z + fz);
    c.fov = approach(c.fov, cam.zoom, 14, dt);
    c.updateProjectionMatrix();
    // Crosshair target: first prop/structure/terrain/fighter along the view ray.
    const max = 320;
    const wall = raycast(this.world, c.position.x, c.position.y, c.position.z, fx, fy, fz, max, true, state.builds);
    let tEnd = wall.t;
    for (const q of state.players) {
      if (q.id === p.id || q.mode === 'dead' || q.mode === 'bus') continue;
      const h = rayFighter(c.position.x, c.position.y, c.position.z, fx, fy, fz, q, tEnd);
      if (h.t >= 0 && h.t < tEnd) tEnd = h.t;
    }
    // Never aim at something behind the shooter's own shoulder.
    tEnd = Math.max(tEnd, dist + 1);
    this.aimPoint.set(c.position.x + fx * tEnd, c.position.y + fy * tEnd, c.position.z + fz * tEnd);
    return this.aimPoint;
  }

  /** Projects a world point to CSS pixels in the stage (null when behind the camera). */
  project(x: number, y: number, z: number): { x: number; y: number } | null {
    _v.set(x, y, z).project(this.view.camera);
    if (_v.z > 1) return null;
    const el = this.view.canvas;
    return { x: (_v.x * 0.5 + 0.5) * el.clientWidth, y: (-_v.y * 0.5 + 0.5) * el.clientHeight };
  }

  eye(p: Fighter): [number, number, number] { return [p.x - Math.cos(p.yaw) * 0.3, p.y + EYE, p.z + Math.sin(p.yaw) * 0.3]; }

  stats() { return { ...this.view.stats(), camera: { x: this.camPos.x, y: this.camPos.y, z: this.camPos.z }, loot: this.loot.size, structures: this.structCount, pads: this.pads[0]?.count ?? 0, drops: this.drops.size }; }

  dispose(): void {
    for (const tr of this.tracers) tr.line.geometry.dispose();
    this.sparks.dispose(); this.smoke.dispose();
    for (const f of this.fighters) f.mixer.stopAllAction();
    this.view.dispose();
    this.lib.dispose();
  }
}
