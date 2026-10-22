/**
 * Storm Royale renderer. The world (render/world.ts): textured PBR terrain,
 * sea / lakes, the walk-in architecture merged per surface family, Dead
 * City's streets, kit props batched per chunk (culled, decor LOD, charred
 * wrecks, felled trees hidden until they regrow) and burning fires. Weather
 * and time of day (render/atmosphere.ts): sky, fog, sun and soft shadows,
 * rain, ash, lightning, night flashlights. Fighters are articulated soldiers
 * drawn as one instanced crowd (render/soldiers.ts, poses from pose.ts):
 * run / crouch / slide / aim / recoil / knocked / dying, their weapon in hand
 * (render/guns.ts), teammates ringed and labelled in their team colour.
 * Combat effects (render/fx.ts): flying tracers, muzzle flashes and lights,
 * bullet holes, blood (optional), explosions with fireball, shockwave,
 * debris, scorch and camera shake. Plus player structures (one persistent
 * mesh per material, cracking as they take damage) with a locally predicted
 * build ghost, chests, supply drops, launch pads, floor loot, rockets, the
 * drop balloon, gliders and the storm / toxic-cloud wall. Third-person
 * camera with shoulder offset and collision; aiming (or the first-person
 * option) blends into first person with the weapon viewmodel and ADS.
 */
import * as THREE from 'three';
import { AssetLibrary } from '../common/assets';
import { GameView, type Theme } from '../common/view';
import { Particles } from '../common/particles';
import { approach, clamp, lerp, smoothstep, wrapAngle } from '../common/math';
import { crackMaskTexture, labelTexture } from '../common/textures';
import { chestY, mapFor, terrainHeight, type ShooterMap } from './map';
import { groundAt, raycast, syncFelled, worldFor, type World } from './collide';
import { BUS_HEIGHT, EYE, heldItem, MAX_FIGHTERS, MAX_PADS, PICKAXE_SLOT, rayFighter, type Drop, type Fighter, type Loot, type Pad, type ShooterEvent, type ShooterState } from './sim';
import { BUILD_COST, CONSUMABLES, isConsumable, isWeapon, MATERIAL_COLOURS, RARITY_COLOURS, WEAPONS } from './weapons';
import { canPlace, packBuild, planBuild, type BuildKind } from './builds';
import { FIGHTER_IDS, teamColour, type FighterId } from './catalog';
import { ghostGeometries, materialTexture, padGeometries, placeGhost, StructureMesh } from './meshes';
import { modeLayer } from './modes/layer';
import { architectureMeshes, Fires, PropChunks, roadMeshes, terrainMesh, waterMeshes, type GroundColours } from './render/world';
import { SOLDIER_SKINS, SoldierCrowd } from './render/soldiers';
import { emptyPose, GRIP_NONE, GRIP_ONE, GRIP_PICK, GRIP_TWO, solvePose, STRIDE, type PoseInput } from './render/pose';
import { gunFor, GUNS, type GunSpec } from './render/guns';
import { Fx } from './render/fx';
import { Atmosphere } from './render/atmosphere';
import { WEATHER, weatherFor, weatherTheme, type WeatherId } from './render/weather';
import { Viewmodel } from './render/viewmodel';

const THEMES: Record<ShooterMap['theme'], Theme & GroundColours & { floor: number }> = {
  isle: { skyTop: 0x3d84d6, skyHorizon: 0xcbe7ff, fog: 0xcfe6fb, fogNear: 140, fogFar: 620, sun: 0xfff2da, sunIntensity: 2.7, ambientSky: 0xd4e8ff, ambientGround: 0x5e7a3c, ambient: 1.1, low: 0x7cc35a, mid: 0x5da746, high: 0x8fbf63, rock: 0x8d8f8c, sand: 0xe8d39a, floor: 0x2e8fd0 },
  mesa: { skyTop: 0x3c7ccb, skyHorizon: 0xf4d6ab, fog: 0xf0d3a8, fogNear: 140, fogFar: 620, sun: 0xffe3bb, sunIntensity: 2.9, ambientSky: 0xffe6c6, ambientGround: 0x9a5b36, ambient: 1.05, low: 0xd99a5c, mid: 0xc97a45, high: 0xe3b276, rock: 0xa65a34, sand: 0xe7c08a, floor: 0xc97a45 },
  city: { skyTop: 0x5d6a78, skyHorizon: 0xc9b9a2, fog: 0xb5a690, fogNear: 60, fogFar: 420, sun: 0xffe2c0, sunIntensity: 2.3, ambientSky: 0xd7d0c6, ambientGround: 0x4a4038, ambient: 1.0, low: 0x6a645b, mid: 0x746c5e, high: 0x7d7262, rock: 0x77716a, sand: 0x857a68, floor: 0x5a544c },
};
const LOOT_RANGE = 70;
const _v = new THREE.Vector3(), _w = new THREE.Vector3(), _d = new THREE.Vector3(), _eye = new THREE.Vector3();
const _q = new THREE.Quaternion();
const _m4 = new THREE.Matrix4(), _pm = new THREE.Matrix4();
const _frustum = new THREE.Frustum();
const ZERO = new THREE.Matrix4().makeScale(0, 0, 0);
const UP = new THREE.Vector3(0, 1, 0);
const PROBE_HEIGHTS = [1.65, 1.1];
const MAT_KEYS = ['surv-box', 'surv-rock-a', 'surv-metal-panel-screws'];
/** Shot event `value`: the weapon by WEAPON order (rockets draw their own trail). */
const SHOT_GUNS = ['pistol', 'smg', 'rifle', 'shotgun', 'sniper', 'rocket'];
/** Viewmodel kick per weapon (0..1). */
const KICK: Record<string, number> = { pistol: 0.45, smg: 0.2, rifle: 0.35, shotgun: 1, sniper: 1, rocket: 0.8 };
/** Stance fields newer sims carry, read defensively (an older snapshot renders standing). */
type Stance = { ads?: number; crouch?: boolean; slideT?: number };

interface FighterVisual {
  root: THREE.Group; glider: THREE.Mesh; label: THREE.Sprite | null;
  /** Animation state: stride phase, death progress, eased stance blends, recoil, last muzzle position and when it was drawn. */
  phase: number; dead: number; crouch: number; slide: number; aim: number; kick: number;
  muzzle: THREE.Vector3; seen: number;
}
/** A seat as the renderer sees it: model, label, and team (for teammate markers). */
export interface SeatView { fighter: FighterId; human: boolean; name: string; team: number }
/**
 * The local survivor's build intent, applied to the preview ghost before the
 * host has echoed it (guests would otherwise see the ghost trail the camera by
 * a round trip). Position is the freshest known one.
 */
export interface LocalBuild { building: boolean; piece: number; mat: number; yaw: number; pitch: number; x: number; y: number; z: number }

/** `firstPerson`: stay in first person (the viewmodel) whenever on foot, not only while aiming. */
export interface CameraState { yaw: number; pitch: number; aiming: boolean; zoom: number; firstPerson?: boolean }
/** Weather / time of day and gore; unset picks the map's default (or a saved preference). */
export interface RenderOptions { weather?: WeatherId; blood?: boolean }

function stored(key: string): string | null { try { return localStorage.getItem(key); } catch { return null; } }

export class ShooterRenderer {
  readonly view: GameView;
  private lib: AssetLibrary;
  private map: ShooterMap;
  private world: World;
  private theme: (typeof THEMES)[ShooterMap['theme']];
  private fighters: FighterVisual[] = [];
  private crowd = new SoldierCrowd(MAX_FIGHTERS);
  private pose = emptyPose();
  private poseIn: PoseInput = { speed: 0, phase: 0, air: false, sprint: false, crouch: 0, slide: 0, aim: 0, kick: 0, swing: 0, grip: GRIP_NONE, pitch: 0, down: false, dead: 0, infected: false, time: 0 };
  private loot = new Map<number, THREE.Object3D>();
  private lootCheck = 0;
  private chests: THREE.Object3D[] = [];
  private rockets = new Map<number, THREE.Object3D>();
  private fx = new Fx();
  private atmosphere: Atmosphere;
  private viewmodel = new Viewmodel();
  private fires: Fires;
  private props: PropChunks | null = null;
  private sparks = new Particles(2000, true);
  private smoke = new Particles(900, false);
  private storm: THREE.Mesh;
  private stormMat: THREE.ShaderMaterial;
  private balloon: THREE.Group;
  private beamGeo = new THREE.CylinderGeometry(0.08, 0.3, 3, 8, 1, true);
  private beamMats = RARITY_COLOURS.map(c => new THREE.MeshBasicMaterial({ color: c, transparent: true, opacity: 0.35, blending: THREE.AdditiveBlending, depthWrite: false }));
  private gliderGeo = new THREE.SphereGeometry(1.6, 16, 8, 0, Math.PI * 2, 0, Math.PI / 2.2);
  private gliderMat = new THREE.MeshStandardMaterial({ color: 0xff5a5a, side: THREE.DoubleSide, roughness: 0.7 });
  /** Structures: one persistent mesh per material, rewritten in place (throttled) when the set or their health changes; damage shows cracks. */
  private crack = crackMaskTexture();
  private structMats = [0, 1, 2].map(() => this.crackable());
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
  /** Felled trees: which instanced meshes + instance draw each trunk, and the matrices hidden away. */
  private felledFrom: readonly number[] | null = null;
  private felledShown = new Uint8Array(0);
  private treeSaved = new Map<number, THREE.Matrix4[]>();
  /** Launch pads (instanced) and supply drops. */
  private padGeos = padGeometries();
  private padBase = new THREE.MeshStandardMaterial({ color: 0x3b4656, roughness: 0.5, metalness: 0.5 });
  private padTop = new THREE.MeshBasicMaterial({ color: 0x62e0ff });
  private pads: THREE.InstancedMesh[] = [];
  private padKey = NaN;
  private drops = new Map<number, THREE.Group>();
  private dropBalloon = new THREE.SphereGeometry(2.6, 16, 10);
  private dropBalloonMat = new THREE.MeshStandardMaterial({ color: 0x4aa8ff, roughness: 0.6 });
  /** Teammate ring under the feet. */
  private ringGeo = new THREE.RingGeometry(0.62, 0.86, 28).rotateX(-Math.PI / 2);
  private ringMats = new Map<number, THREE.MeshBasicMaterial>();
  private ghosts: THREE.Mesh[] = [];
  private ghostOk = new THREE.MeshBasicMaterial({ color: 0x62b4ff, transparent: true, opacity: 0.35, depthWrite: false });
  private ghostBad = new THREE.MeshBasicMaterial({ color: 0xff5a5a, transparent: true, opacity: 0.3, depthWrite: false });
  private ground = (x: number, z: number, feet: number) => groundAt(this.world, x, z, feet);
  private groundTop = (x: number, z: number) => terrainHeight(this.map, x, z);
  private time = 0;
  private camPos = new THREE.Vector3();
  private camLook = new THREE.Vector3(0, 0, 1);
  /** First-person blend (0 third person, 1 eye), screen shake, last view angles (viewmodel sway). */
  private fp = 0;
  private shake = 0;
  private lastYaw = 0; private lastPitch = 0;
  private self: number;
  private seats: SeatView[];
  /** Team modes: teammates get rings and labels; enemies get neither. */
  private teams: boolean;
  private alive = new Set<number>();
  private blood: boolean;
  /** Saved preference: stay in first person on foot (`localStorage['otto.games.fpv'] = '1'`). */
  private fpPref: boolean;
  aimPoint = new THREE.Vector3();
  /** Whether the current build preview can be placed (for the HUD). */
  ghostValid = false;

  constructor(host: HTMLElement, mapId: string, seats: SeatView[], self: number, signal: AbortSignal, options: RenderOptions = {}) {
    this.map = mapFor(mapId);
    this.world = worldFor(this.map);
    this.theme = THEMES[this.map.theme];
    const weather = weatherFor(this.map.id, options.weather ?? stored('otto.games.weather'));
    this.blood = options.blood ?? stored('otto.games.blood') !== '0';
    this.fpPref = stored('otto.games.fpv') === '1';
    this.view = new GameView(host, weatherTheme(this.theme, weather), 2048, { softShadows: true, shadowExtent: 62 });
    this.view.camera.far = 1400;
    this.view.overlay = { scene: this.viewmodel.scene, camera: this.viewmodel.camera };
    this.atmosphere = new Atmosphere(this.view, this.theme, weather);
    this.viewmodel.setLighting(weatherTheme(this.theme, weather));
    this.fires = new Fires(this.map.fires);
    this.lib = new AssetLibrary(signal);
    this.seats = seats;
    this.self = self;
    this.teams = seats.some((s, i) => i !== self && s.team === seats[self]?.team);
    for (const s of SOLDIER_SKINS) this.crowd.skin(s);
    this.viewmodel.setSkin(SOLDIER_SKINS[Math.max(0, FIGHTER_IDS.indexOf(seats[self]?.fighter ?? 'male-a'))]);
    const wall = WEATHER[weather].wall;
    this.stormMat = new THREE.ShaderMaterial({
      transparent: true, side: THREE.DoubleSide, depthWrite: false,
      uniforms: { time: { value: 0 }, tint: { value: new THREE.Vector3(wall[0], wall[1], wall[2]) } },
      vertexShader: 'varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }',
      // Billowing fbm cloud wall: two warped noise layers scrolling up and around, fading at the foot and the top, with crackling sparks.
      fragmentShader: `uniform float time; uniform vec3 tint; varying vec2 vUv;
        float hash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
        float noise(vec2 p){ vec2 i = floor(p), f = fract(p); vec2 u = f * f * (3.0 - 2.0 * f);
          return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y); }
        float fbm(vec2 p){ float s = 0.0, a = 0.5; for (int i = 0; i < 4; i++) { s += a * noise(p); p *= 2.03; a *= 0.5; } return s; }
        void main(){
          vec2 p = vec2(vUv.x * 96.0, vUv.y * 12.0);
          float n = fbm(p + vec2(time * 0.25, -time * 0.55));
          float m = fbm(p * 0.6 + vec2(-time * 0.18, -time * 0.3) + n * 2.2);
          float a = (0.22 + m * 0.5) * smoothstep(0.0, 0.03, vUv.y) * (1.0 - smoothstep(0.62, 1.0, vUv.y));
          float spark = step(0.992, noise(vec2(vUv.x * 420.0, floor(time * 9.0) + vUv.y * 30.0))) * smoothstep(0.35, 0.7, m);
          vec3 col = mix(tint * 0.45, tint * 1.35 + 0.12, m) + spark * vec3(0.9, 0.95, 1.0);
          gl_FragColor = vec4(col, clamp(a + spark * 0.6, 0.0, 0.92)); }`,
    });
    this.storm = new THREE.Mesh(new THREE.CylinderGeometry(1, 1, 260, 128, 1, true), this.stormMat);
    this.storm.frustumCulled = false;
    this.storm.renderOrder = 4;
    this.balloon = new THREE.Group();
  }

  /** Structure material: textured, vertex-tinted, with cracks that spread as health drops. */
  private crackable(): THREE.MeshStandardMaterial {
    const m = new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.82, metalness: 0.05 });
    const crack = this.crack;
    m.onBeforeCompile = shader => {
      shader.uniforms.crackMap = { value: crack };
      // StructureMesh tints worn pieces toward grey (red channel down to ~0.67); growing ones go blue (> 1).
      shader.fragmentShader = `uniform sampler2D crackMap;\n${shader.fragmentShader.replace('#include <map_fragment>', `#include <map_fragment>
        #ifdef USE_MAP
          float dmg = clamp((1.0 - vColor.r) / 0.33, 0.0, 1.0) * step(vColor.b, 1.01) * step(0.05, 1.0 - vColor.r);
          float crk = texture2D(crackMap, vMapUv * 1.3).r;
          diffuseColor.rgb *= 1.0 - step(1.0 - dmg * 0.95, crk) * 0.72;
        #endif`)}`;
    };
    return m;
  }

  async load(): Promise<void> {
    await this.lib.loadKit('shooter-kit.glb');
    this.buildWorld();
    this.seats.forEach((seat, i) => this.fighters.push(this.buildFighter(seat, i)));
    const scene = this.view.scene;
    scene.add(this.sparks.points, this.smoke.points, this.storm, this.crowd.group, this.fx.group, this.atmosphere.group, this.fires.group);
    this.structMats.forEach((m, i) => { m.map = this.lib.own(materialTexture(i)); });
    this.structMeshes = this.structs.map((st, i) => {
      const mesh = new THREE.Mesh(st.geometry, this.structMats[i]);
      mesh.castShadow = true; mesh.receiveShadow = true; mesh.visible = false;
      scene.add(mesh);
      return mesh;
    });
    this.pads = [this.padBase, this.padTop].map((m, i) => {
      const mesh = new THREE.InstancedMesh(this.padGeos[i], m, MAX_PADS);
      mesh.count = 0; mesh.frustumCulled = false; mesh.receiveShadow = true;
      scene.add(mesh);
      return mesh;
    });
    this.ghosts = ghostGeometries().map(g => {
      const mesh = new THREE.Mesh(this.lib.own(g), this.ghostOk);
      mesh.visible = false; mesh.renderOrder = 5;
      scene.add(mesh);
      return mesh;
    });
    for (const r of [this.beamGeo, ...this.beamMats, this.gliderGeo, this.gliderMat, this.stormMat, this.storm.geometry, ...this.structMats, ...this.structs, this.ghostOk, this.ghostBad,
      ...this.padGeos, this.padBase, this.padTop, this.dropBalloon, this.dropBalloonMat, this.ringGeo, this.crack]) this.lib.own(r);
  }

  private buildWorld(): void {
    const map = this.map, scene = this.view.scene, own = <T extends { dispose(): void }>(r: T) => this.lib.own(r);
    scene.add(terrainMesh(map, this.theme, own), ...waterMeshes(map, this.theme.floor), ...architectureMeshes(map, own), ...roadMeshes(map, own));
    this.props = new PropChunks(map, this.lib);
    scene.add(this.props.group);
    this.felledShown = new Uint8Array(map.cylinders.length);
    for (const ch of map.chests) {
      const o = this.lib.instance('gun-crate-wide');
      o.scale.setScalar(2.4);
      o.position.set(ch.x, chestY(map, ch), ch.z);
      o.rotation.y = ch.yaw;
      scene.add(o);
      this.chests.push(o);
    }
    // The collapsed towers left scorched ground.
    for (const p of map.pois) if (p.kind === 'tower') this.fx.decal('scorch', p.x, terrainHeight(map, p.x, p.z + 18) + 0.05, p.z + 18, 0, 1, 0, 12);
    // Drop balloon: Kenney delivery van slung under a striped envelope.
    const van = this.lib.instance('car-delivery');
    van.scale.setScalar(3.4);
    const envelope = new THREE.Mesh(this.lib.own(new THREE.SphereGeometry(8, 24, 16)), this.lib.own(new THREE.MeshStandardMaterial({ color: 0x4c7bff, roughness: 0.6 })));
    envelope.position.y = 17; envelope.scale.set(1, 1.15, 1);
    const band = new THREE.Mesh(this.lib.own(new THREE.TorusGeometry(8.1, 0.7, 8, 32)), this.lib.own(new THREE.MeshStandardMaterial({ color: 0xffd23f, roughness: 0.6 })));
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
    const glider = new THREE.Mesh(this.gliderGeo, this.gliderMat);
    glider.position.y = 3.6; glider.visible = false; glider.castShadow = true;
    root.add(glider);
    let label: THREE.Sprite | null = null;
    // Teammates (team modes) are labelled in the team colour and ringed; in solo, other people are labelled.
    const mate = this.teams && index !== this.self && seat.team === this.seats[this.self]?.team;
    if ((seat.human && index !== this.self && !this.teams) || mate) {
      const tint = mate ? `#${teamColour(seat.team).toString(16).padStart(6, '0')}` : '#ffffff';
      label = new THREE.Sprite(this.lib.own(new THREE.SpriteMaterial({ map: this.lib.own(labelTexture(seat.name, tint)), depthTest: false, transparent: true })));
      label.scale.set(3.2, 0.8, 1); label.position.y = 2.4; label.renderOrder = 10;
      root.add(label);
    }
    if (mate || (this.teams && index === this.self)) {
      const ring = new THREE.Mesh(this.ringGeo, this.ringMat(seat.team));
      ring.position.y = 0.06; ring.renderOrder = 3;
      root.add(ring);
    }
    this.view.scene.add(root);
    return { root, glider, label, phase: 0, dead: 0, crouch: 0, slide: 0, aim: 0, kick: 0, muzzle: new THREE.Vector3(), seen: -1 };
  }

  effect(e: ShooterEvent, state: ShooterState): void {
    const near = Math.hypot(e.x - this.camPos.x, e.z - this.camPos.z);
    switch (e.type) {
      case 'shot': {
        const gun = SHOT_GUNS[e.value] ?? 'rifle';
        if (gun === 'rocket') break; // rockets draw their own trail
        const own = e.player === this.self && this.fp > 0.6;
        const v = this.fighters[e.player];
        // From the gun's muzzle: the viewmodel's (own, first person), the body's (drawn just now), else the event's origin.
        if (own) {
          const c = this.view.camera;
          _w.set(0.12, -0.1, -0.55).applyQuaternion(c.quaternion).add(c.position);
          this.viewmodel.kick(KICK[gun] ?? 0.4);
        } else if (v && this.time - v.seen < 0.2 && Math.hypot(v.muzzle.x - e.x, v.muzzle.z - e.z) < 3) { _w.copy(v.muzzle); v.kick = 1; }
        else _w.set(e.x, e.y, e.z);
        _d.set(e.x2 - _w.x, e.y2 - _w.y, e.z2 - _w.z);
        const dist = _d.length();
        if (dist < 0.1) break;
        _d.divideScalar(dist);
        this.fx.tracer(_w.x, _w.y, _w.z, e.x2, e.y2, e.z2, gun === 'sniper' ? 0xfff0c8 : 0xffcf7a);
        if (!own) this.fx.muzzle(_w.x, _w.y, _w.z, _d.x, _d.y, _d.z, gun === 'shotgun' ? 0.5 : gun === 'smg' || gun === 'pistol' ? 0.28 : 0.38);
        if (near < 60) this.fx.flash(_w.x + _d.x * 0.3, _w.y, _w.z + _d.z * 0.3, own ? 3 : 6, 0xffc070, 0.06, 12);
        // Impact: a hole and dust where the shot met the world (not a fighter).
        _v.set(e.x2 - e.x, e.y2 - e.y, e.z2 - e.z);
        const len = _v.length();
        if (len > 0.1) {
          _v.divideScalar(len);
          const hit = raycast(this.world, e.x, e.y, e.z, _v.x, _v.y, _v.z, len + 0.4, true, state.builds);
          if (hit.kind !== 'none' && hit.t > len - 0.7) {
            const hx = e.x + _v.x * hit.t, hy = e.y + _v.y * hit.t, hz = e.z + _v.z * hit.t;
            if (near < 120) this.fx.decal('hole', hx, hy, hz, hit.nx, hit.ny, hit.nz, gun === 'shotgun' ? 0.32 : 0.2);
            const dust = hit.kind === 'terrain' ? this.theme.sand : hit.kind === 'build' ? 0xc8b49a : 0xb8b2a6;
            for (let i = 0; i < 4; i++) this.smoke.spawn(hx, hy, hz, hit.nx * 1.5 + (Math.random() - 0.5), hit.ny * 1.5 + Math.random(), hit.nz * 1.5 + (Math.random() - 0.5), dust, 0.5, 0.7, -1, 2);
            this.sparks.burst(hx, hy, hz, hit.kind === 'terrain' ? 2 : 5, 0xffd27a, 3, 0.18, 0.2);
          }
        }
        break;
      }
      case 'hit': {
        if (e.flag & 1) { this.sparks.burst(e.x, e.y, e.z, 10, 0x6fc6ff, 4, 0.35, 0.35); break; }
        if (!this.blood) { this.sparks.burst(e.x, e.y, e.z, 7, 0xffe0a0, 4, 0.3, 0.3); break; }
        // Blood: a red mist, a splat on the ground, and one on the wall behind when it is close.
        for (let i = 0; i < 9; i++) this.smoke.spawn(e.x, e.y, e.z, (Math.random() - 0.5) * 3, Math.random() * 2, (Math.random() - 0.5) * 3, 0x7a0c0c, 0.32, 0.45, -4, 3);
        const victim = state.players[e.target], shooter = state.players[e.player];
        if (victim && Math.random() < 0.5) this.fx.decal('blood', victim.x + (Math.random() - 0.5) * 0.8, this.ground(victim.x, victim.z, victim.y + 0.3) + 0.03, victim.z + (Math.random() - 0.5) * 0.8, 0, 1, 0, 0.7 + Math.random() * 0.7);
        if (shooter) {
          _v.set(e.x - shooter.x, 0, e.z - shooter.z);
          if (_v.lengthSq() > 0.01) {
            _v.normalize();
            const wall = raycast(this.world, e.x, e.y, e.z, _v.x, 0, _v.z, 2.5, true, state.builds);
            if (wall.kind === 'prop' || wall.kind === 'build') this.fx.decal('blood', e.x + _v.x * wall.t, e.y, e.z + _v.z * wall.t, wall.nx, wall.ny, wall.nz, 0.9);
          }
        }
        break;
      }
      case 'explode': {
        const gy = this.groundTop(e.x, e.z);
        this.fx.fireball(e.x, e.y + 0.4, e.z, 3.2, gy);
        this.fx.burst(e.x, e.y + 0.5, e.z, 24, this.theme.rock, 16, 0.35);
        this.fx.burst(e.x, e.y + 0.5, e.z, 10, 0x2a2420, 12, 0.5);
        this.fx.flash(e.x, e.y + 2, e.z, 260, 0xffa040, 0.55, 48);
        if (e.y - gy < 1.5) this.fx.decal('scorch', e.x, gy + 0.04, e.z, 0, 1, 0, 5.5);
        this.sparks.burst(e.x, e.y, e.z, 80, 0xffa040, 18, 1.2, 0.7);
        for (let i = 0; i < 30; i++) this.smoke.spawn(e.x, e.y + 0.5, e.z, (Math.random() - 0.5) * 8, 2 + Math.random() * 5, (Math.random() - 0.5) * 8, i % 3 ? 0x3a3532 : 0x5a524a, 4.5, 2.2, 0.4, 1.4);
        this.shake = Math.max(this.shake, clamp(1 - near / 45, 0, 1));
        break;
      }
      case 'chest': this.sparks.burst(e.x, e.y + 0.8, e.z, 40, 0xffd34d, 7, 0.6, 0.8); break;
      case 'land': for (let i = 0; i < 12; i++) this.smoke.spawn(e.x, e.y + 0.2, e.z, (Math.random() - 0.5) * 6, 0.8, (Math.random() - 0.5) * 6, 0xcbb894, 1.6, 0.8, 0, 2); break;
      case 'kill': for (let i = 0; i < 10; i++) this.smoke.spawn(e.x, e.y + 0.4, e.z, (Math.random() - 0.5) * 3, 0.5 + Math.random(), (Math.random() - 0.5) * 3, 0x8a8278, 1.2, 0.9, 0, 2); break;
      case 'build': this.sparks.burst(e.x, e.y + 0.4, e.z, 16, 0x8fd0ff, 3, 0.4, 0.5); break;
      case 'edit': this.sparks.burst(e.x, e.y, e.z, 14, 0xffffff, 3, 0.3, 0.4); break;
      case 'break':
        this.sparks.burst(e.x, e.y, e.z, 30, MATERIAL_COLOURS[e.flag] ?? 0xc8955a, 7, 0.6, 0.9, 2);
        this.fx.burst(e.x, e.y, e.z, 14, MATERIAL_COLOURS[e.flag] ?? 0xc8955a, 7, 0.3);
        for (let i = 0; i < 10; i++) this.smoke.spawn(e.x, e.y, e.z, (Math.random() - 0.5) * 5, Math.random() * 2, (Math.random() - 0.5) * 5, 0xb7ab98, 2.2, 1.1, 0, 1.4);
        break;
      case 'harvest': this.sparks.burst(e.x, e.y, e.z, 8, MATERIAL_COLOURS[e.value] ?? 0xc8955a, 3, 0.3, 0.5, 2); this.fx.burst(e.x, e.y, e.z, 3, MATERIAL_COLOURS[e.value] ?? 0xc8955a, 4, 0.12); break;
      case 'use': this.sparks.burst(e.x, e.y + (e.flag === 1 ? 0.4 : 1.2), e.z, 24, e.value === 2 ? 0x62e0ff : e.value === 1 ? 0x62b4ff : 0x6fe07a, 2.5, 0.4, 0.8); break;
      case 'knock': this.sparks.burst(e.x, e.y + 0.6, e.z, 24, this.blood ? 0x9a1010 : 0xff5a4a, 5, 0.4, 0.8, 2); break;
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
  }

  /** Switches weather / time of day mid-match. */
  setWeather(id: WeatherId): void {
    this.atmosphere.apply(id);
    this.viewmodel.setLighting(weatherTheme(this.theme, id));
    const w = WEATHER[id].wall;
    this.stormMat.uniforms.tint.value.set(w[0], w[1], w[2]);
  }
  /** Blood on hits (off: sparks). */
  setBlood(on: boolean): void { this.blood = on; }

  /** Renders a frame; returns the world point under the crosshair. `local` drives the build ghost. */
  frame(state: ShooterState, selfIndex: number, cam: CameraState, dt: number, spectate: number, local?: LocalBuild): THREE.Vector3 {
    this.time += dt;
    syncFelled(this.world, state.felled);
    this.syncTrees(state.felled);
    this.syncPads(state.pads);
    this.syncDrops(state.drops);
    this.stormMat.uniforms.time.value = this.time;
    const s = state.storm;
    // The wall appears once the first circle is announced; Apocalypse Royale's toxic cloud is green.
    this.storm.visible = s.stage >= 0 || s.timer < 20;
    this.storm.position.set(s.cx, 80, s.cz);
    this.storm.scale.set(Math.max(0.5, s.r), 1, Math.max(0.5, s.r));
    if (state.mode === 'apoc') this.stormMat.uniforms.tint.value.set(0.35, 0.85, 0.3);
    // Balloon follows the flight path while anyone is aboard.
    const t = clamp(state.bus.t / state.bus.duration, 0, 1);
    this.balloon.visible = state.phase === 'bus' || state.players.some(p => p.mode === 'bus');
    this.balloon.position.set(lerp(state.bus.ax, state.bus.bx, t), BUS_HEIGHT - 9, lerp(state.bus.az, state.bus.bz, t));
    this.balloon.rotation.y = Math.atan2(state.bus.bx - state.bus.ax, state.bus.bz - state.bus.az);
    // Camera first, so the bodies are culled against this frame's view.
    const focus = state.players[spectate] ?? state.players[selfIndex];
    const aim = this.updateCamera(state, focus, cam, dt, spectate === selfIndex);
    const c = this.view.camera;
    c.updateMatrixWorld();
    _frustum.setFromProjectionMatrix(_pm.multiplyMatrices(c.projectionMatrix, c.matrixWorldInverse));
    this.crowd.begin(_frustum);
    this.atmosphere.beginBeams();
    state.players.forEach((p, i) => {
      if (!this.fighters[i]) return;
      if (this.seats[i] && this.seats[i].fighter !== p.fighter) this.seats[i] = { ...this.seats[i], fighter: p.fighter };
      this.updateFighter(this.fighters[i], p, dt, i === spectate && spectate === selfIndex && this.fp > 0.5);
    });
    this.crowd.end();
    this.atmosphere.endBeams();
    this.padTop.color.setHSL(0.53, 0.9, 0.55 + Math.sin(this.time * 5) * 0.12);
    state.chests.forEach((open, i) => { const o = this.chests[i]; if (o) { o.scale.y = open ? 1.2 : 2.4; if (!open && Math.random() < 0.05) this.sparks.spawn(o.position.x + (Math.random() - 0.5) * 2, o.position.y + 1, o.position.z + (Math.random() - 0.5) * 2, 0, 1.2, 0, 0xffd34d, 0.35, 0.8); } });
    this.lootCheck -= dt;
    if (this.lootCheck <= 0) { this.lootCheck = 0.2; this.syncLoot(state.loot); }
    for (const [, o] of this.loot) { o.rotation.y += dt * 1.2; }
    this.syncRockets(state);
    this.syncStructures(state, dt);
    // Infected, grenades, clouds, drones and the modes' objectives (modes/layer.ts).
    modeLayer(this.view.scene).frame(state, dt);
    this.updateGhost(state, state.players[selfIndex], local);
    this.fx.update(dt, this.groundTop);
    this.sparks.update(dt); this.smoke.update(dt);
    this.fires.update(dt, c.position.x, c.position.z);
    this.props?.update(c.position.x, c.position.z);
    const me = state.players[selfIndex];
    const lit = !!me && (me.mode === 'ground' || me.mode === 'down') && spectate === selfIndex;
    this.atmosphere.update(dt, c, lit ? _eye.set(me.x, me.y + EYE, me.z) : null, this.camLook, this.fp > 0.5);
    this.updateViewmodel(me, cam, dt, spectate === selfIndex);
    this.view.focusShadow(focus.x, focus.y, focus.z);
    this.view.render();
    return aim;
  }

  private updateViewmodel(p: Fighter | undefined, cam: CameraState, dt: number, own: boolean): void {
    const yawRate = dt > 0 ? wrapAngle(cam.yaw - this.lastYaw) / dt : 0, pitchRate = dt > 0 ? (cam.pitch - this.lastPitch) / dt : 0;
    this.lastYaw = cam.yaw; this.lastPitch = cam.pitch;
    const show = own && !!p && p.mode === 'ground' && !p.building && this.fp > 0.6;
    this.viewmodel.scene.visible = show;
    if (!show || !p) return;
    const held = heldItem(p), pick = p.current === PICKAXE_SLOT;
    const gun = pick ? GUNS.pickaxe : held ? gunFor(held.w) : null;
    const ex = p as Fighter & Stance;
    this.viewmodel.update(dt, {
      gun, ads: cam.aiming ? (ex.ads ?? 1) : 0, scoped: cam.aiming && !!held && isWeapon(held.w) && (held.w === 'sniper' || (held.att & 1) === 1), speed: Math.hypot(p.vx, p.vz), sprint: p.sprinting, reload: p.reload > 0 || p.useT > 0, grounded: p.grounded,
      yawRate, pitchRate, aspect: this.view.camera.aspect,
    });
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
    if (list === this.felledFrom || !this.props) return;
    this.felledFrom = list;
    const want = this.world.felled, trees = this.props.trees;
    for (let i = 0; i < trees.length; i++) {
      if (want[i] === this.felledShown[i]) continue;
      this.felledShown[i] = want[i];
      const t = trees[i];
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

  /** Poses one fighter and adds its body to the crowd (unless it is this player seen from their own eyes). */
  private updateFighter(v: FighterVisual, p: Fighter, dt: number, hideBody: boolean): void {
    const show = p.mode !== 'bus';
    v.root.visible = show;
    if (!show) return;
    v.root.position.set(p.x, p.y, p.z);
    v.root.rotation.y = p.yaw;
    v.glider.visible = p.mode === 'glide';
    const down = p.mode === 'down';
    if (v.label) {
      // Name tags are world-sized: one right next to the camera would cover the middle of the view, so
      // they fade out up close (and while looking down the sights).
      const m = v.label.material as THREE.SpriteMaterial, c = this.view.camera.position;
      m.color.setHex(down ? 0xff7a6a : 0xffffff);
      m.opacity = smoothstep(5, 10, Math.hypot(p.x - c.x, p.y + 2.4 - c.y, p.z - c.z)) * (1 - this.fp * 0.7);
      v.label.visible = m.opacity > 0.02;
    }
    const ex = p as Fighter & Stance;
    const speed = Math.hypot(p.vx, p.vz);
    v.phase += (speed / STRIDE) * Math.PI * 2 * dt;
    v.dead = p.mode === 'dead' ? Math.min(1, v.dead + dt * 1.8) : 0;
    v.crouch = approach(v.crouch, ex.crouch ? 1 : 0, 12, dt);
    v.slide = approach(v.slide, (ex.slideT ?? 0) > 0 ? 1 : 0, 14, dt);
    v.aim = approach(v.aim, ex.ads ?? (p.aiming ? 1 : 0), 14, dt);
    v.kick = approach(v.kick, 0, 12, dt);
    const held = heldItem(p);
    const pick = p.current === PICKAXE_SLOT && !p.building;
    const armed = (!!held || pick) && !p.building && !p.swimming && !down && p.mode === 'ground';
    const gun: GunSpec | null = !armed ? null : pick ? GUNS.pickaxe : held ? gunFor(held.w) : null;
    const oneHand = !!held && (held.w === 'pistol' || isConsumable(held.w));
    const i = this.poseIn;
    i.speed = speed; i.phase = v.phase; i.time = this.time + p.id * 1.7;
    i.air = p.mode === 'sky' || p.mode === 'glide' || (!p.grounded && !p.swimming && !down && p.mode !== 'dead');
    i.sprint = p.sprinting; i.crouch = v.crouch; i.slide = v.slide; i.aim = v.aim; i.kick = v.kick;
    i.swing = p.swingT > 0 ? clamp(1 - p.swingT / 0.35, 0, 1) : 0;
    i.grip = !gun ? GRIP_NONE : pick ? GRIP_PICK : oneHand ? GRIP_ONE : held && isWeapon(held.w) ? GRIP_TWO : GRIP_ONE;
    i.pitch = p.pitch; i.down = down; i.dead = v.dead; i.infected = false;
    const pose = solvePose(i, this.pose);
    if (p.mode === 'sky') {
      // Skydiving: flat, arms and legs spread.
      pose.tilt = 1.25; pose.shL = pose.shR = 0.4; pose.rollL = pose.rollR = 1.3; pose.elL = pose.elR = 0.4; pose.hipL = pose.hipR = 0.15; pose.kneeL = pose.kneeR = 0.5;
    } else if (p.mode === 'glide') { pose.shL = pose.shR = 2.6; pose.elL = pose.elR = 0.2; pose.rollL = pose.rollR = 0.35; }
    if (p.swimming) { pose.drop += 0.9; pose.tilt = 0.5; }
    if (hideBody) return;
    const skin = Math.max(0, FIGHTER_IDS.indexOf(p.fighter));
    const band = this.teams ? teamColour(p.team) : -1;
    if (this.crowd.add(p.x, p.y, p.z, p.yaw, pose, skin, band, gun)) { v.muzzle.copy(this.crowd.muzzle); v.seen = this.time; }
    // Night: everyone else's flashlight cone.
    if (p.mode === 'ground' && p.id !== this.self) this.atmosphere.beam(p.x, p.y + EYE - v.crouch * 0.45, p.z, p.yaw, p.pitch);
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
      if (!this.lib.has(key)) { key = 'gun-clip-small'; scale = 3.2; }
      const m = this.lib.instance(key);
      m.scale.setScalar(scale); m.position.y = lift;
      if (l.kind === 'weapon') m.rotation.z = Math.PI / 2.4;
      g.add(m);
      const ray = new THREE.Mesh(this.beamGeo, this.beamMats[beam] ?? this.beamMats[0]);
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
    // First person while aiming (or always, when asked) on foot.
    const wantFp = own && p.mode === 'ground' && !p.building && (cam.aiming || !!cam.firstPerson || this.fpPref);
    this.fp = approach(this.fp, wantFp ? 1 : 0, 16, dt);
    if (Math.abs(this.fp - (wantFp ? 1 : 0)) < 0.01) this.fp = wantFp ? 1 : 0;
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
    let dist = back;
    if (this.fp < 1) {
      // Pull the camera in front of walls between the pivot and the desired spot.
      const hit = raycast(this.world, pivotX, py, pivotZ, -fx, -fy, -fz, back + 0.3, false, state.builds);
      dist = Math.max(0.6, Math.min(back, hit.t - 0.3));
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
    }
    c.position.copy(this.camPos);
    if (this.fp > 0) {
      // The eye: lower when crouched or sliding, a touch ahead of the head so the body never shows.
      const v = this.fighters[p.id];
      const drop = (v?.crouch ?? 0) * 0.5 + (v?.slide ?? 0) * 0.75;
      _w.set(p.x + fx * 0.14, p.y + EYE + 0.06 - drop, p.z + fz * 0.14);
      c.position.lerp(_w, this.fp);
      // Fully in first person: keep the third-person spot fresh for the way back.
      if (this.fp >= 1) this.camPos.set(pivotX - fx * back, py - fy * back, pivotZ - fz * back);
    }
    // Explosion shake.
    if (this.shake > 0.001) {
      const k = this.shake * this.shake * 0.35;
      c.position.x += (Math.random() - 0.5) * k; c.position.y += (Math.random() - 0.5) * k; c.position.z += (Math.random() - 0.5) * k;
      this.shake = approach(this.shake, 0, 5, dt);
    }
    c.lookAt(c.position.x + fx, c.position.y + fy, c.position.z + fz);
    this.camLook.set(fx, fy, fz);
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
    tEnd = Math.max(tEnd, lerp(dist + 1, 0.5, this.fp));
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

  stats() {
    return {
      ...this.view.stats(), camera: { x: this.camPos.x, y: this.camPos.y, z: this.camPos.z }, loot: this.loot.size, structures: this.structCount, pads: this.pads[0]?.count ?? 0, drops: this.drops.size,
      bodies: this.crowd.drawn, weather: this.atmosphere.weather, firstPerson: this.fp,
    };
  }

  dispose(): void {
    this.sparks.dispose(); this.smoke.dispose();
    this.crowd.dispose(); this.fx.dispose(); this.atmosphere.dispose(); this.viewmodel.dispose(); this.fires.dispose();
    this.view.dispose();
    this.lib.dispose();
  }
}
