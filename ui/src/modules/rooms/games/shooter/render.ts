/**
 * Storm Royale renderer: island terrain + sea, instanced Kenney props, chests,
 * floor loot (nearest items only), animated mini-character fighters holding
 * Blaster Kit weapons, the drop balloon, gliders, tracers, rockets,
 * explosions and the storm wall. Third-person camera with shoulder offset,
 * aim zoom and collision.
 */
import * as THREE from 'three';
import { AssetLibrary, PropBatch } from '../common/assets';
import { GameView, owned, type Theme } from '../common/view';
import { Particles } from '../common/particles';
import { approach, clamp, lerp, smoothstep } from '../common/math';
import { groundDetailTexture, labelTexture } from '../common/textures';
import { GRID, HALF, SEA_LEVEL, mapFor, terrainHeight, type ShooterMap } from './map';
import { raycast, worldFor, type World } from './collide';
import { EYE, rayFighter, type Fighter, type Loot, type ShooterEvent, type ShooterState } from './sim';
import { RARITY_COLOURS, WEAPONS } from './weapons';
import { FIGHTER_IDS, type FighterId } from './catalog';

const THEMES: Record<ShooterMap['theme'], Theme & { low: number; mid: number; high: number; rock: number; sand: number }> = {
  isle: { skyTop: 0x3d84d6, skyHorizon: 0xcbe7ff, fog: 0xcfe6fb, fogNear: 120, fogFar: 520, sun: 0xfff2da, sunIntensity: 2.7, ambientSky: 0xd4e8ff, ambientGround: 0x5e7a3c, ambient: 1.1, low: 0x7cc35a, mid: 0x5da746, high: 0x8fbf63, rock: 0x8d8f8c, sand: 0xe8d39a },
  mesa: { skyTop: 0x3c7ccb, skyHorizon: 0xf4d6ab, fog: 0xf0d3a8, fogNear: 120, fogFar: 520, sun: 0xffe3bb, sunIntensity: 2.9, ambientSky: 0xffe6c6, ambientGround: 0x9a5b36, ambient: 1.05, low: 0xd99a5c, mid: 0xc97a45, high: 0xe3b276, rock: 0xa65a34, sand: 0xe7c08a },
};
const CHAR_SCALE = 2.7;
const UPPER = new Set(['arm-left', 'arm-right']);
const LOOT_RANGE = 70;
const _v = new THREE.Vector3();
const _q = new THREE.Quaternion();
const UP = new THREE.Vector3(0, 1, 0);
const PROBE_HEIGHTS = [1.65, 1.1];

interface FighterVisual {
  fighter: FighterId; root: THREE.Group; model: THREE.Object3D; mixer: THREE.AnimationMixer; hand: THREE.Object3D | null;
  lower: Map<string, THREE.AnimationAction>; upper: Map<string, THREE.AnimationAction>; die: THREE.AnimationAction;
  lowerName: string; upperName: string; dead: boolean; gun: THREE.Object3D | null; gunKey: string; glider: THREE.Mesh; label: THREE.Sprite | null;
}
interface Tracer { line: THREE.Line; life: number }

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
  private time = 0;
  private camPos = new THREE.Vector3();
  private self: number;
  private seats: { fighter: FighterId; human: boolean; name: string }[];
  private alive = new Set<number>();
  aimPoint = new THREE.Vector3();

  constructor(host: HTMLElement, mapId: string, seats: { fighter: FighterId; human: boolean; name: string }[], self: number, signal: AbortSignal) {
    this.map = mapFor(mapId);
    this.world = worldFor(this.map);
    this.view = new GameView(host, THEMES[this.map.theme]);
    this.view.camera.far = 1200;
    this.lib = new AssetLibrary(signal);
    this.seats = seats;
    this.self = self;
    this.stormMat = new THREE.ShaderMaterial({
      transparent: true, side: THREE.DoubleSide, depthWrite: false,
      uniforms: { time: { value: 0 } },
      vertexShader: 'varying vec2 vUv; varying vec3 vPos; void main(){ vUv = uv; vPos = position; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }',
      fragmentShader: `uniform float time; varying vec2 vUv; varying vec3 vPos;
        void main(){ float band = sin(vUv.x * 160.0 + time * 1.5) * 0.5 + 0.5; float swirl = sin(vUv.y * 24.0 - time * 2.0 + vUv.x * 40.0) * 0.5 + 0.5;
          float a = 0.32 + band * 0.12 + swirl * 0.1; a *= smoothstep(0.0, 0.05, vUv.y) * (1.0 - smoothstep(0.85, 1.0, vUv.y));
          gl_FragColor = vec4(mix(vec3(0.42, 0.18, 0.78), vec3(0.75, 0.45, 1.0), swirl), a); }`,
    });
    this.storm = new THREE.Mesh(new THREE.CylinderGeometry(1, 1, 220, 96, 1, true), this.stormMat);
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
    for (const r of [this.tracerMat, this.beamGeo, ...this.beamMats, this.gliderGeo, this.gliderMat, this.stormMat, this.storm.geometry]) this.lib.own(r);
  }

  private buildWorld(): void {
    const map = this.map, theme = THEMES[map.theme], scene = this.view.scene;
    const n = map.size;
    const pos = new Float32Array(n * n * 3), col = new Float32Array(n * n * 3), uv = new Float32Array(n * n * 2);
    const c = new THREE.Color(), low = new THREE.Color(theme.low), mid = new THREE.Color(theme.mid), high = new THREE.Color(theme.high), rock = new THREE.Color(theme.rock), sand = new THREE.Color(theme.sand);
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
      else c.lerp(sand, smoothstep(0.3, 0.6, Math.sin(x * 0.05) * Math.cos(z * 0.04) * 0.5 + 0.5) * 0.35);
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
    const terrain = owned(new THREE.Mesh(geo, new THREE.MeshLambertMaterial({ vertexColors: true, map: this.lib.own(groundDetailTexture()) })));
    terrain.receiveShadow = true;
    scene.add(terrain);
    if (map.theme === 'isle') {
      const sea = owned(new THREE.Mesh(new THREE.PlaneGeometry(3000, 3000), new THREE.MeshLambertMaterial({ color: 0x2e8fd0, transparent: true, opacity: 0.88 })));
      sea.rotation.x = -Math.PI / 2; sea.position.y = SEA_LEVEL - 0.25;
      scene.add(sea);
    } else {
      // Desert floor beyond the playable cliffs.
      const floor = owned(new THREE.Mesh(new THREE.PlaneGeometry(3000, 3000), new THREE.MeshLambertMaterial({ color: theme.mid })));
      floor.rotation.x = -Math.PI / 2; floor.position.y = -1;
      scene.add(floor);
    }
    const batch = new PropBatch(this.lib);
    for (const p of map.props) batch.add(p.key, p.x, p.y - 0.05, p.z, p.yaw, p.scale);
    const group = new THREE.Group();
    batch.build(group, true);
    scene.add(group);
    for (const ch of map.chests) {
      const o = this.lib.instance('gun-crate-wide');
      o.scale.setScalar(2.4);
      o.position.set(ch.x, terrainHeight(map, ch.x, ch.z), ch.z);
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

  private buildFighter(seat: { fighter: FighterId; human: boolean; name: string }, index: number): FighterVisual {
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
    if (seat.human && index !== this.self) {
      label = new THREE.Sprite(this.lib.own(new THREE.SpriteMaterial({ map: this.lib.own(labelTexture(seat.name, '#ffffff')), depthTest: false, transparent: true })));
      label.scale.set(3.2, 0.8, 1); label.position.y = 2.5; label.renderOrder = 10;
      root.add(label);
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
      default: break;
    }
    void state;
  }

  /** Renders a frame; returns the world point under the crosshair. */
  frame(state: ShooterState, selfIndex: number, cam: CameraState, dt: number, spectate: number): THREE.Vector3 {
    this.time += dt;
    this.stormMat.uniforms.time.value = this.time;
    const s = state.storm;
    // The wall appears once the first circle is announced.
    this.storm.visible = s.stage >= 0 || s.timer < 20;
    this.storm.position.set(s.cx, 70, s.cz);
    this.storm.scale.set(Math.max(0.5, s.r), 1, Math.max(0.5, s.r));
    // Balloon follows the flight path while anyone is aboard.
    const t = clamp(state.bus.t / state.bus.duration, 0, 1);
    this.balloon.visible = state.phase === 'bus' || state.players.some(p => p.mode === 'bus');
    this.balloon.position.set(lerp(state.bus.ax, state.bus.bx, t), 86, lerp(state.bus.az, state.bus.bz, t));
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
    state.chests.forEach((open, i) => { const o = this.chests[i]; if (o) { o.scale.y = open ? 1.2 : 2.4; if (!open && Math.random() < 0.05) this.sparks.spawn(o.position.x + (Math.random() - 0.5) * 2, o.position.y + 1, o.position.z + (Math.random() - 0.5) * 2, 0, 1.2, 0, 0xffd34d, 0.35, 0.8); } });
    this.lootCheck -= dt;
    if (this.lootCheck <= 0) { this.lootCheck = 0.2; this.syncLoot(state.loot); }
    for (const [, o] of this.loot) { o.rotation.y += dt * 1.2; }
    this.syncRockets(state);
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
    const speed = Math.hypot(p.vx, p.vz);
    const air = p.mode === 'sky' || p.mode === 'glide' || !p.grounded;
    const lower = air ? (p.vy > 1 ? 'jump' : 'fall') : speed > 6.2 ? 'sprint' : speed > 0.6 ? 'walk' : 'idle';
    v.lowerName = this.play(v.lower, v.lowerName, lower);
    v.lower.get('walk')!.timeScale = clamp(speed / 4, 0.6, 1.6);
    const held = p.weapons[p.current];
    const pistol = held?.w === 'pistol';
    const upper = !held || p.mode !== 'ground' ? (air ? 'fall' : lower === 'jump' ? 'fall' : lower) : p.sprinting ? 'sprint' : p.shootT > 0 ? (pistol ? 'holding-right-shoot' : 'holding-both-shoot') : pistol ? 'holding-right' : 'holding-both';
    v.upperName = this.play(v.upper, v.upperName, upper, 0.08);
    v.mixer.update(dt);
    v.glider.visible = p.mode === 'glide';
    // Weapon in the right hand.
    const key = held && p.mode === 'ground' ? WEAPONS[held.w].model : '';
    if (key !== v.gunKey) {
      v.gun?.removeFromParent();
      v.gun = null; v.gunKey = key;
      if (key) {
        // Held at chest height in front of the right shoulder, barrel forward
        // (+Z); the holding pose raises the arms to meet it.
        const gun = this.lib.instance(key);
        const pistol = held?.w === 'pistol';
        gun.scale.setScalar(pistol ? 0.3 : 0.34);
        gun.position.set(-0.12, 0.3, pistol ? 0.19 : 0.16);
        v.model.add(gun);
        v.gun = gun;
      }
    }
    if (v.gun) v.gun.visible = true;
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
      let key: string, scale: number, lift = 0.5;
      if (l.kind === 'weapon' && l.weapon) { key = WEAPONS[l.weapon].model; scale = 1.5; lift = 0.7; }
      else if (l.kind === 'ammo') { key = l.ammo === 'shells' || l.ammo === 'rockets' ? 'gun-clip-large' : 'gun-clip-small'; scale = 3.2; }
      else if (l.kind === 'heal') { key = 'plat-heart'; scale = 2.2; }
      else { key = 'plat-jewel'; scale = 2.2; }
      const m = this.lib.instance(key);
      m.scale.setScalar(scale); m.position.y = lift;
      if (l.kind === 'weapon') m.rotation.z = Math.PI / 2.4;
      g.add(m);
      const beam = new THREE.Mesh(this.beamGeo, this.beamMats[l.kind === 'weapon' ? l.rarity : l.kind === 'shield' ? 2 : l.kind === 'heal' ? 1 : 0]);
      beam.position.y = 1.5;
      g.add(beam);
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
    else if (cam.aiming) { back = 2.8; side = 1.15; up = 1.95; }
    // Over the right shoulder: the survivor sits in the lower-left third and
    // the crosshair stays clear.
    else { back = 5.6; side = 1.55; up = 2.3; }
    const py = baseY + up;
    const pivotX = px + rx * side, pivotZ = pz + rz * side;
    // Pull the camera in front of walls between the pivot and the desired spot.
    const hit = raycast(this.world, pivotX, py, pivotZ, -fx, -fy, -fz, back + 0.3, false);
    const dist = Math.max(0.6, Math.min(back, hit.t - 0.3));
    _v.set(pivotX - fx * dist, py - fy * dist, pivotZ - fz * dist);
    _v.y = Math.max(_v.y, terrainHeight(this.map, _v.x, _v.z) + 0.4);
    // Keep the survivor in view: cast from the head and the chest to the
    // camera; a prop in between (car, barrel, crate stack) pulls the camera in
    // front of it, or lifts it over the prop when that would be too close.
    if (p.mode === 'ground' || p.mode === 'dead') {
      for (let attempt = 0; attempt < 3; attempt++) {
        let nearest = Infinity, ox = 0, oy = 0, oz = 0, ex = 0, ey = 0, ez = 0, el = 0;
        for (const h of PROBE_HEIGHTS) {
          const sx = p.x, sy = p.y + h, sz = p.z;
          const dx = _v.x - sx, dy = _v.y - sy, dz = _v.z - sz, dl = Math.hypot(dx, dy, dz);
          if (dl < 0.5) continue;
          const block = raycast(this.world, sx, sy, sz, dx / dl, dy / dl, dz / dl, dl, false);
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
    // Crosshair target: first prop/terrain/fighter along the view ray.
    const max = 320;
    const wall = raycast(this.world, c.position.x, c.position.y, c.position.z, fx, fy, fz, max);
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

  stats() { return { ...this.view.stats(), camera: { x: this.camPos.x, y: this.camPos.y, z: this.camPos.z }, loot: this.loot.size }; }

  dispose(): void {
    for (const tr of this.tracers) tr.line.geometry.dispose();
    this.sparks.dispose(); this.smoke.dispose();
    for (const f of this.fighters) f.mixer.stopAllAction();
    this.view.dispose();
    this.lib.dispose();
  }
}
