/**
 * Kart Grand Prix renderer: builds the track world and animates it from
 * simulation / snapshot state. The world: a sky dome with sun and drifting
 * clouds (stars at dusk), softened sun shadows, terrain with themed detail,
 * lagoon water (waves, Fresnel sky, glitter, shoreline foam, caustics below)
 * or flowing lava that lights the rock around it, every route's road
 * (render/road.ts: world-space asphalt, grooves, tyre marks, paint, rumble
 * curbs, special surfaces, banners, neon, dash panels), barrier walls and
 * rails, banking, bridges with pillars and abutments, embankments, tunnels
 * with ribs and lamps, ramps, launch pads, half-pipes, drawbridges, the start
 * grid, item boxes, coins, obstacles, fork signage, chevron boards, themed
 * Kenney props and cheering crowds on the grandstands. Placement of anything
 * solid comes from layout.ts (and render/dressing.ts, held to the same rule).
 *
 * Animated: karts, drivers, gliders, boost flames tinted by what fired them,
 * drift-spark glows per tier, shells by kind, lightning strikes, shockwaves,
 * dust / snow / water spray, slipstream streaks, burnouts, fireworks at the
 * finish, a spring-lagged chase camera with FOV kick, speed lines and shake.
 * Under the water surface the camera fogs over blue.
 */
import * as THREE from 'three';
import { approach, clamp, lerp, smoothstep, wrapAngle } from '../common/math';
import { labelTexture } from '../common/textures';
import { DRAW_ANGLE, drawLift, F_TUNNEL, nearest, obstacleAt, pointAt, ROLLER_RADIUS, sampleAt, TUNNEL_HEIGHT } from './tracks';
import { barriers } from './layout';
import { racerDef, RACERS } from './roster';
import { SHELL_KINDS, type KartEvent, type KartState } from './sim';
import { fireworkSpots } from './render/dressing';
import { Bolts, Rings, ShellKit, SpeedLines } from './render/fx';
import { chaseFov, springTo, type Spring } from './render/camera';
import { KART_SCALE, SPARK, FLAME, SHELL_TINT, FIREWORK, DEEP, _near, _ob, _cp, _v, _look, _m, _q, _s, _p, _up, type KartVisual, type RenderRacer } from './render/shared';
import { KartWorld } from './render/world';
export type { RenderRacer } from './render/shared';

export class KartRenderer extends KartWorld {
  async load(): Promise<void> {
    // Every seated driver is loaded up front: guests may switch racer during the intro.
    const drivers = RACERS.map(r => r.driver).filter((d): d is string => !!d);
    await Promise.all([this.lib.loadKit('kart-kit.glb'), drivers.length ? this.lib.loadCharacters(drivers) : Promise.resolve()]);
    for (const m of [this.flameGeo, this.coreGeo, this.coreMat, this.glow, this.bubbleGeo, this.bubbleMat, this.wingGeo, this.strutGeo, this.strutMat]) this.lib.own(m);
    this.buildWorld();
    for (const r of this.racers) this.karts.push(this.buildKart(r));
    const scene = this.view.scene;
    scene.add(this.particles.points, this.smoke.points);
    // The camera carries the speed lines, so it joins the scene graph.
    scene.add(this.view.camera);
    this.speedLines = new SpeedLines(this.lib, this.view.camera);
    this.bolts = new Bolts(this.lib, scene);
    this.rings = new Rings(this.lib, scene);
    this.shellKit = new ShellKit(this.lib);
    this.spots = fireworkSpots(this.track);
  }
  private buildKart(r: RenderRacer): KartVisual {
    const def = racerDef(r.racer);
    const root = new THREE.Group();
    const body = new THREE.Group();
    const kart = this.lib.instance(def.kart);
    body.add(kart);
    body.scale.setScalar(KART_SCALE);
    root.add(body);
    const wheels: THREE.Object3D[] = [], front: THREE.Object3D[] = [];
    let character: THREE.Object3D | null = null;
    kart.traverse(o => {
      if (/wheel-(front|back)-(left|right)$/.test(o.name)) { wheels.push(o); if (o.name.includes('front')) front.push(o); }
      if (o.name.endsWith('character')) character = o;
    });
    let mixer: THREE.AnimationMixer | null = null;
    if (def.driver) {
      if (character) (character as THREE.Object3D).visible = false;
      const driver = this.lib.character(def.driver);
      driver.position.set(0, 0.2, -0.12);
      body.add(driver);
      mixer = new THREE.AnimationMixer(driver);
      mixer.clipAction(this.lib.clip('drive')).play();
    }
    // Boost: a tinted outer flame and a white-hot core per exhaust, with a glow at the pipe.
    const flameMat = this.lib.own(new THREE.MeshBasicMaterial({ color: FLAME.pad, transparent: true, opacity: 0.85, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false }));
    const exhaustMat = this.lib.own(new THREE.SpriteMaterial({ map: this.glow, color: FLAME.pad, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false }));
    const flames: THREE.Mesh[] = [], cores: THREE.Mesh[] = [], exhaust: THREE.Sprite[] = [];
    for (const x of [-0.18, 0.18]) {
      const f = new THREE.Mesh(this.flameGeo, flameMat);
      f.rotation.x = -Math.PI / 2; f.position.set(x * KART_SCALE, 0.55, -1.3); f.visible = false;
      const c = new THREE.Mesh(this.coreGeo, this.coreMat);
      c.rotation.x = -Math.PI / 2; c.position.set(x * KART_SCALE, 0.55, -1.2); c.visible = false;
      const g = new THREE.Sprite(exhaustMat);
      g.position.set(x * KART_SCALE, 0.58, -1.05); g.scale.setScalar(1.2); g.visible = false;
      root.add(f, c, g);
      flames.push(f); cores.push(c); exhaust.push(g);
    }
    // Drift: a spark glow at each rear wheel in the tier's colour.
    const sparkMat = this.lib.own(new THREE.SpriteMaterial({ map: this.glow, color: SPARK[1], blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false }));
    const sparks = [-1, 1].map(side => {
      const s = new THREE.Sprite(sparkMat);
      s.position.set(side * 0.95, 0.3, -0.95); s.visible = false;
      root.add(s);
      return s;
    });
    const bubble = new THREE.Mesh(this.bubbleGeo, this.bubbleMat);
    bubble.position.y = 1.1; bubble.visible = false;
    root.add(bubble);
    // Glider: a canopy in the racer's colour on two struts, folded away until launch.
    const wing = new THREE.Group();
    const canopy = new THREE.Mesh(this.wingGeo, this.lib.own(new THREE.MeshLambertMaterial({ color: def.color })));
    canopy.position.y = 1.35; canopy.castShadow = true;
    wing.add(canopy);
    for (const x of [-0.7, 0.7]) { const strut = new THREE.Mesh(this.strutGeo, this.strutMat); strut.position.set(x, 0.7, 0); wing.add(strut); }
    wing.position.set(0, 1.2, -0.2);
    wing.visible = false;
    root.add(wing);
    let label: THREE.Sprite | null = null;
    if (r.human && r.id !== this.self) {
      const tex = this.lib.own(labelTexture(r.name, '#ffffff'));
      label = new THREE.Sprite(this.lib.own(new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true })));
      label.scale.set(4, 1, 1); label.position.y = 3.6; label.renderOrder = 10;
      root.add(label);
    }
    this.view.scene.add(root);
    return {
      racer: r.racer, root, body, wheels, front, mixer, flames, cores, flameMat, exhaust, exhaustMat, sparks, sparkMat, bubble, label, wing,
      wheelSpin: 0, tilt: 0, bounce: 0, visualYaw: 0, spinAngle: 0, pitch: 0, wingOpen: 0, flameTint: FLAME.pad, lastLevel: 0, size: 1,
    };
  }

  // ── Frame ────────────────────────────────────────────────────────────────
  /** Distance from the camera's racer, for shake and nearby-only effects. */
  private nearSelf(x: number, z: number): number {
    const k = this.karts[this.self];
    return k ? Math.hypot(x - k.root.position.x, z - k.root.position.z) : Infinity;
  }
  effect(e: KartEvent): void {
    const p = this.particles, own = e.racer === this.self, kart = this.karts[e.racer];
    switch (e.type) {
      case 'box': p.burst(e.x, e.y + 1.2, e.z, 30, 0xc58cff, 9, 0.7, 0.6); p.burst(e.x, e.y + 1.2, e.z, 12, 0xffffff, 6, 0.5, 0.4); break;
      case 'explode': {
        const kind = SHELL_KINDS[e.value ?? 3] ?? 'bomb';
        if (kind === 'bomb' || kind === 'blue') {
          // A fireball, a smoke column, a shockwave and a jolt for anyone close.
          p.burst(e.x, e.y + 0.8, e.z, 90, 0xffa040, 18, 1.5, 0.8);
          p.burst(e.x, e.y + 1.2, e.z, 40, kind === 'blue' ? 0x7fb0ff : 0xfff0a0, 10, 1.1, 0.5);
          for (let i = 0; i < 22; i++) this.smoke.spawn(e.x, e.y + 1, e.z, (Math.random() - 0.5) * 7, 3 + Math.random() * 4, (Math.random() - 0.5) * 7, 0x4a4a4a, 3.4, 1.6, 0, 1.4);
          this.rings.spawn(e.x, e.y, e.z, kind === 'blue' ? 0x7fb0ff : 0xffb060, 9, 0.55);
          this.shake.add(clamp(0.7 - this.nearSelf(e.x, e.z) / 30, 0, 0.7));
        } else {
          // A shell shatters: shards in its colour and white.
          p.burst(e.x, e.y + 0.8, e.z, 36, SHELL_TINT[kind], 11, 0.8, 0.6, -14);
          p.burst(e.x, e.y + 0.8, e.z, 16, 0xffffff, 8, 0.6, 0.4, -14);
          this.rings.spawn(e.x, e.y, e.z, SHELL_TINT[kind], 3.5, 0.35);
          this.shake.add(clamp(0.35 - this.nearSelf(e.x, e.z) / 30, 0, 0.35));
        }
        break;
      }
      case 'spin': p.burst(e.x, e.y + 1.6, e.z, 18, 0xffff88, 5, 0.6, 0.8, 0); if (own) this.shake.add(0.25); break;
      case 'bump': p.burst(e.x, e.y + 0.6, e.z, 10, 0xffd080, 6, 0.4, 0.3); if (own) this.shake.add(0.15); break;
      case 'throw': p.burst(e.x, e.y + 1, e.z, 12, SHELL_TINT[SHELL_KINDS[e.value ?? 0] ?? 'green'], 5, 0.5, 0.35, 0); break;
      case 'boost': {
        // value 0 ramp / pad, 1 mushroom, 2 perfect start, 3 golden.
        const tint = e.value === 1 ? FLAME.mushroom : e.value === 2 ? FLAME.start : e.value === 3 ? FLAME.golden : FLAME.pad;
        if (kart) kart.flameTint = tint;
        p.burst(e.x, e.y + 0.6, e.z, e.value ? 30 : 18, tint, 8, 0.7, 0.4);
        this.rings.spawn(e.x, e.y, e.z, tint, e.value ? 4 : 2.5, 0.35);
        if (own) this.shake.add(e.value ? 0.18 : 0.08);
        break;
      }
      case 'turbo':
        if (kart) kart.flameTint = SPARK[e.value ?? 1];
        p.burst(e.x, e.y + 0.6, e.z, 26 + (e.value ?? 1) * 8, SPARK[e.value ?? 1], 8, 0.65, 0.45);
        this.rings.spawn(e.x, e.y, e.z, SPARK[e.value ?? 1], 2 + (e.value ?? 1), 0.3);
        break;
      case 'draft':
        if (kart) kart.flameTint = FLAME.draft;
        p.burst(e.x, e.y + 1, e.z, 24, 0xe8fbff, 7, 0.45, 0.35, 0);
        break;
      case 'burnout': for (let i = 0; i < 16; i++) this.smoke.spawn(e.x + (Math.random() - 0.5) * 2, e.y + 0.4, e.z + (Math.random() - 0.5) * 2, (Math.random() - 0.5) * 2, 1.5 + Math.random(), (Math.random() - 0.5) * 2, 0x3a3a3a, 2.4, 1.2, 0, 1.5); break;
      case 'star': p.burst(e.x, e.y + 1, e.z, 50, 0xffe066, 10, 0.8, 0.8); p.burst(e.x, e.y + 1, e.z, 20, 0xffffff, 8, 0.6, 0.6); break;
      // Lightning: the user's own bolt flashes the sky (no burst over them: it never strikes its user);
      // each opponent struck gets a `zap` (below).
      case 'bolt': this.flash = 1; this.rings.spawn(e.x, e.y, e.z, 0x9ad7ff, 5, 0.45); break;
      case 'zap':
        this.bolts.strike(e.x, e.y, e.z);
        p.burst(e.x, e.y + 1, e.z, 40, 0x9ad7ff, 10, 0.8, 0.5);
        this.rings.spawn(e.x, e.y, e.z, 0xbfe8ff, 4, 0.4);
        if (own) this.shake.add(0.5);
        break;
      case 'respawn': p.burst(e.x, e.y + 1, e.z, 30, 0xffffff, 6, 1, 0.8); this.rings.spawn(e.x, e.y, e.z, 0xffffff, 3, 0.4); break;
      case 'land':
        for (let i = 0; i < 12; i++) this.smoke.spawn(e.x, e.y + 0.3, e.z, (Math.random() - 0.5) * 6, 1, (Math.random() - 0.5) * 6, this.dustColor(), 1.8, 0.7, 0, 2);
        if (own) this.shake.add(0.22);
        break;
      case 'glide': p.burst(e.x, e.y + 2, e.z, 30, 0x9fe6ff, 7, 0.7, 0.6, 0); break;
      case 'crush': for (let i = 0; i < 16; i++) this.smoke.spawn(e.x, e.y + 0.4, e.z, (Math.random() - 0.5) * 9, 1.5, (Math.random() - 0.5) * 9, 0x8a8a8a, 2.2, 0.9, 0, 2); if (own) this.shake.add(0.6); break;
      case 'splash': {
        // value 1: diving under on purpose; 0: fell in the water; 2: lava.
        const y = this.track.liquid ?? e.y, lava = e.value === 2;
        p.burst(e.x, y + 0.3, e.z, e.value === 1 ? 30 : 60, lava ? 0xff7a2a : 0xe8fbff, e.value === 1 ? 6 : 10, lava ? 0.9 : 1.1, 0.9, -12);
        this.rings.spawn(e.x, y, e.z, lava ? 0xff7a2a : 0xffffff, e.value === 1 ? 3 : 5, 0.6);
        if (lava) for (let i = 0; i < 14; i++) this.smoke.spawn(e.x, y + 0.5, e.z, (Math.random() - 0.5) * 3, 3 + Math.random() * 2, (Math.random() - 0.5) * 3, 0x3a3030, 3, 1.6, 0, 1);
        break;
      }
      case 'finish': this.fireworks = Math.max(this.fireworks, own ? 7 : 3.5); break;
      default: break;
    }
  }
  private dustColor(): number {
    const k = this.theme.offroad;
    return k === 'snow' ? 0xffffff : k === 'cinder' ? 0x4a3a36 : k === 'mud' ? 0xb0a080 : this.theme.bed;
  }

  frame(state: KartState, dt: number, lookBack: boolean): void {
    this.time += dt;
    this.shared.time.value = this.time;
    const self = state.racers[this.self] ?? state.racers[0];
    state.racers.forEach((r, i) => {
      const visual = this.karts[i];
      if (!visual) return;
      if (visual.racer !== r.racer) {
        // A guest picked a different racer: swap the kart model in place.
        visual.mixer?.stopAllAction();
        visual.root.removeFromParent();
        this.racers[i] = { ...this.racers[i], racer: r.racer };
        this.karts[i] = this.buildKart(this.racers[i]);
      }
      this.updateKart(this.karts[i], r, dt, i === this.self);
    });
    // Item boxes spin and pop back.
    state.boxes.forEach((b, i) => {
      const box = this.boxes[i];
      if (!box) return;
      const shown = b.timer <= 0;
      const target = shown ? 4 : 0.01;
      box.scale.setScalar(approach(box.scale.x, target, shown ? 6 : 20, dt));
      box.visible = box.scale.x > 0.05;
      box.position.set(b.x, b.y - 0.9 + Math.sin(this.time * 2 + i) * 0.25, b.z);
      box.rotation.set(0.3, this.time * 1.5 + i, 0.2);
    });
    this.updateObstacles(state);
    this.updateCoins(state);
    for (const db of this.drawbridges) {
      const th = DRAW_ANGLE * drawLift(db.def, state.raceTime);
      for (const leaf of db.leaves) leaf.rotation.x = -th;
    }
    this.syncBananas(state);
    this.syncShells(state);
    this.road.dash.offset.y = (-this.time * 1.4) % 1;
    this.updateFireworks(dt);
    // The crowd cheers as the race passes the grandstands and goes wild at the finish.
    const d = Math.hypot(self.x - this.track.px[0], self.z - this.track.pz[0]);
    this.cheer.value = state.phase === 'done' ? 1 : approach(this.cheer.value, 0.25 + 0.75 * (1 - smoothstep(30, 140, d)), 2, dt);
    // A lightning flash lights the whole sky for a moment.
    this.flash = Math.max(0, this.flash - dt * 3.5);
    this.view.hemi.intensity = this.theme.ambient * (1 + this.flash * 1.6);
    this.particles.update(dt);
    this.smoke.update(dt);
    this.bolts.update(dt, this.view.camera);
    this.rings.update(dt);
    this.updateCamera(state, self, dt, lookBack);
    this.speedLines.update(dt, state.phase === 'race' ? clamp((self.boost > 0 ? 0.65 : 0) + smoothstep(26, 40, self.speed) * 0.5, 0, 1) : 0, Math.max(0, self.speed));
    this.updateFog();
    if (this.pin) this.view.focusShadow(this.pin[3], this.pin[4], this.pin[5]); else this.view.focusShadow(self.x, self.y, self.z);
    this.view.render();
  }

  /** Fireworks over the finish line after a racer crosses it: a burst every ~0.3 s from the launch spots. */
  private updateFireworks(dt: number): void {
    if (this.fireworks <= 0) return;
    this.fireworks -= dt;
    this.fireworkAt -= dt;
    if (this.fireworkAt > 0 || !this.spots.length) return;
    this.fireworkAt = 0.22 + Math.random() * 0.2;
    const k = Math.floor(Math.random() * (this.spots.length / 3)) * 3;
    const x = this.spots[k] + (Math.random() - 0.5) * 16, y = this.spots[k + 1] + (Math.random() - 0.5) * 8, z = this.spots[k + 2] + (Math.random() - 0.5) * 16;
    const color = FIREWORK[Math.floor(Math.random() * FIREWORK.length)];
    this.particles.burst(x, y, z, 110, color, 16, 1.3, 1.4, -5);
    this.particles.burst(x, y, z, 30, 0xffffff, 9, 0.9, 0.7, -5);
  }

  /** Rollers roll across the road, crushers rise and slam (same clock as the simulation). */
  private updateObstacles(state: KartState): void {
    const { track } = this;
    track.obstacles.forEach((o, i) => {
      const v = this.obstacles[i];
      if (!v) return;
      obstacleAt(track, o, state.raceTime, _ob);
      if (o.kind === 'roller') {
        const path = track.paths[o.seg];
        const k = sampleAt(path, o.s);
        v.mesh.position.set(_ob[0], _ob[1], _ob[2]);
        // Roll about the road's direction as it crosses.
        const dLat = _ob[3] - v.roll;
        v.roll = _ob[3];
        v.mesh.rotateOnWorldAxis(_p.set(path.tx[k], 0, path.tz[k]), -dLat / ROLLER_RADIUS);
      } else {
        const path = track.paths[o.seg];
        v.mesh.position.set(_ob[0], _ob[1] + 1.3 + _ob[3], _ob[2]);
        v.mesh.rotation.y = Math.atan2(path.tx[sampleAt(path, o.s)], path.tz[sampleAt(path, o.s)]);
        if (_ob[3] < 0.05 && v.roll > 0.05) {
          for (let n = 0; n < 8; n++) this.smoke.spawn(_ob[0], _ob[1] + 0.3, _ob[2], (Math.random() - 0.5) * 8, 1, (Math.random() - 0.5) * 8, 0x8a8a8a, 2, 0.6, 0, 2);
          if (this.nearSelf(_ob[0], _ob[2]) < 25) this.shake.add(0.2);
        }
        v.roll = _ob[3];
      }
    });
  }

  /** Coins spin in place; collected ones shrink away until they reappear. */
  private updateCoins(state: KartState): void {
    const mesh = this.coins;
    if (!mesh) return;
    const n = Math.min(mesh.count, state.coins.length);
    for (let i = 0; i < n; i++) {
      const c = state.coins[i];
      const shown = c.timer <= 0 ? 1 : 0;
      _p.set(c.x, c.y + Math.sin(this.time * 2.4 + i * 0.7) * 0.15, c.z);
      _q.setFromAxisAngle(_up, this.time * 3 + i * 0.4);
      mesh.setMatrixAt(i, _m.compose(_p, _q, _s.set(shown, shown, shown)));
    }
    mesh.instanceMatrix.needsUpdate = true;
  }

  private alive = new Set<number>();
  private syncBananas(state: KartState): void {
    const alive = this.alive, pool = this.bananas;
    alive.clear();
    for (const h of state.hazards) {
      alive.add(h.id);
      let o = pool.get(h.id);
      if (!o) { o = this.lib.instance('toy-item-banana'); o.scale.setScalar(5); this.view.scene.add(o); pool.set(h.id, o); }
      o.position.set(h.x, h.y + 0.05, h.z);
    }
    for (const [id, o] of pool) if (!alive.has(id)) { o.removeFromParent(); pool.delete(id); }
  }
  /** Shells by kind (green, red, blue models; the bomb is the kit's), spinning, with a trail in their colour. */
  private syncShells(state: KartState): void {
    const alive = this.alive, pool = this.shells;
    alive.clear();
    for (const s of state.shells) {
      alive.add(s.id);
      let entry = pool.get(s.id);
      if (entry && entry.kind !== s.kind) { entry.object.removeFromParent(); entry = undefined; }
      if (!entry) {
        const object = s.kind === 'bomb' ? this.lib.instance('plat-bomb') : this.shellKit.build(s.kind);
        if (s.kind === 'bomb') object.scale.setScalar(2.6);
        this.view.scene.add(object);
        entry = { kind: s.kind, object };
        pool.set(s.id, entry);
      }
      const o = entry.object;
      if (s.kind === 'bomb') {
        o.position.set(s.x, s.y - 0.4 + Math.abs(Math.sin(this.time * 9)) * 0.4, s.z); o.rotation.y = s.yaw;
        if (Math.random() < 0.6) this.particles.spawn(s.x, s.y + 1.4, s.z, (Math.random() - 0.5), 1.5, (Math.random() - 0.5), 0xffd080, 0.4, 0.3);
      } else {
        // The blue shell flies high over the pack; the others skim the road spinning.
        o.position.set(s.x, s.y - 0.3 + (s.kind === 'blue' ? 3.5 + Math.sin(this.time * 4) * 0.3 : 0), s.z);
        o.rotation.y = this.time * (s.kind === 'blue' ? 4 : 14);
        if (Math.random() < 0.7) this.particles.spawn(s.x, o.position.y + 0.4, s.z, 0, 0.6, 0, SHELL_TINT[s.kind], 0.55, 0.35);
      }
    }
    for (const [id, e] of pool) if (!alive.has(id)) { e.object.removeFromParent(); pool.delete(id); }
  }

  private updateKart(v: KartVisual, r: KartState['racers'][number], dt: number, isSelf: boolean): void {
    const hidden = r.respawn > 0;
    v.root.visible = !hidden;
    if (hidden) return;
    v.root.position.set(r.x, r.y, r.z);
    // Lightning shrinks a racer while it is slowed; after a rescue the kart blinks while it is untouchable.
    v.size = approach(v.size, r.slow > 0 ? 0.62 : 1, 8, dt);
    v.root.scale.setScalar(v.size);
    v.body.visible = !(r.ghost > 0 && Math.floor(this.time * 14) % 2 === 0);
    // Drift angle + spin-out spin are visual on top of the travel heading.
    const driftAngle = r.drift !== 0 ? -r.drift * 0.38 : 0;
    v.visualYaw = approach(v.visualYaw, driftAngle, 8, dt);
    if (r.spin > 0) v.spinAngle += dt * 14; else v.spinAngle = approach(wrapAngle(v.spinAngle), 0, 10, dt);
    v.root.rotation.y = r.yaw + v.visualYaw + v.spinAngle + (r.trick > 0 ? (0.45 - r.trick) / 0.45 * Math.PI * 2 : 0);
    // Road shape under the kart: banking rolls it, slopes pitch it.
    const path = this.track.paths[r.seg] ?? this.track;
    const k = r.hint >= 0 && r.hint < path.count ? r.hint : sampleAt(path, 0);
    const bank = r.grounded ? Math.atan(-path.bank[k]) : 0;
    const k2 = path.closed ? (k + 1) % path.count : Math.min(path.count - 1, k + 1);
    const slope = (path.py[k2] - path.py[k]) / Math.max(0.5, path.length / path.count);
    // Slope sign relative to the kart's heading (driving the wrong way flips it).
    const along = Math.sin(r.yaw) * path.tx[k] + Math.cos(r.yaw) * path.tz[k];
    v.tilt = approach(v.tilt, clamp(-r.steer * Math.abs(r.speed) * 0.004, -0.12, 0.12) + (r.drift !== 0 ? r.drift * 0.08 : 0) + bank * Math.sign(along || 1), 8, dt);
    v.body.rotation.z = v.tilt;
    v.pitch = r.grounded ? approach(v.pitch, -Math.atan(slope) * Math.sign(along || 1), 10, dt) : clamp(-r.vy * 0.02, -0.25, 0.25);
    v.body.rotation.x = v.pitch;
    v.bounce = approach(v.bounce, 0, 12, dt);
    // A burnout spins the wheels on the spot.
    v.wheelSpin += (r.stall > 0 ? 40 : r.speed) * dt / 0.42;
    for (const w of v.wheels) w.rotation.x = v.wheelSpin;
    for (const w of v.front) w.rotation.y = -r.steer * 0.45;
    v.mixer?.update(dt);
    // Glider canopy unfolds on launch.
    v.wingOpen = approach(v.wingOpen, r.glide ? 1 : 0, 8, dt);
    v.wing.visible = v.wingOpen > 0.02;
    if (v.wing.visible) { v.wing.scale.set(v.wingOpen, 1, 1); v.wing.rotation.z = -r.steer * 0.25 * v.wingOpen; }
    // Boost flames: longer for stronger boosts, flickering, in the colour of whatever fired them
    // (gold all through a golden-mushroom window).
    const boosting = r.boost > 0;
    if (r.golden > 0) v.flameTint = FLAME.golden;
    if (boosting) { v.flameMat.color.setHex(v.flameTint); v.exhaustMat.color.setHex(v.flameTint); }
    const len = 0.8 + clamp(r.boostPower - 1.25, 0, 0.35) * 2.6;
    for (let n = 0; n < 2; n++) {
      v.flames[n].visible = v.cores[n].visible = v.exhaust[n].visible = boosting;
      if (!boosting) continue;
      const s = len * (0.8 + Math.random() * 0.45) * Math.min(1, r.boost * 4 + 0.3);
      v.flames[n].scale.set(1, s, 1);
      v.flames[n].position.z = -1.3 - (s - 1) * 0.6;
      v.cores[n].scale.set(1, s * 0.85, 1);
      v.exhaust[n].scale.setScalar(1.1 + Math.random() * 0.5);
    }
    v.bubble.visible = r.star > 0;
    if (r.star > 0) { (v.bubble.material as THREE.MeshBasicMaterial).color.setHSL((this.time * 0.8) % 1, 1, 0.6); v.bubble.scale.setScalar(1 + Math.sin(this.time * 12) * 0.04); }
    // Particles from the rear wheels.
    const fx = Math.sin(r.yaw), fz = Math.cos(r.yaw), lx = fz, lz = -fx;
    const camDist = Math.hypot(r.x - this.view.camera.position.x, r.z - this.view.camera.position.z);
    const near = isSelf || camDist < 90;
    // Drift sparks: a glow at each rear wheel in the tier's colour (flaring on each new tier) plus sparks.
    const drifting = near && r.grounded && r.drift !== 0;
    for (const s of v.sparks) s.visible = drifting && r.driftLevel > 0;
    if (drifting && r.driftLevel > 0) {
      v.sparkMat.color.setHex(SPARK[r.driftLevel]);
      const flare = r.driftLevel > v.lastLevel ? 2.2 : 1;
      for (const s of v.sparks) s.scale.setScalar((0.9 + r.driftLevel * 0.35 + Math.random() * 0.5) * flare);
      if (r.driftLevel > v.lastLevel) this.particles.burst(r.x - fx, r.y + 0.4, r.z - fz, 22, SPARK[r.driftLevel], 7, 0.5, 0.35, -10);
    }
    v.lastLevel = r.drift !== 0 ? r.driftLevel : 0;
    if (drifting) {
      for (const side of [-1, 1]) {
        const x = r.x - fx * 1.0 + lx * side * 0.9, z = r.z - fz * 1.0 + lz * side * 0.9;
        this.particles.spawn(x, r.y + 0.25, z, (Math.random() - 0.5) * 3 - fx * 2, 1.5 + Math.random() * 2, (Math.random() - 0.5) * 3 - fz * 2, SPARK[r.driftLevel], r.driftLevel ? 0.45 : 0.3, 0.3, -9, 2);
        if (r.driftLevel === 0 && Math.random() < 0.3) this.smoke.spawn(x, r.y + 0.2, z, -fx, 0.6, -fz, 0xdadada, 0.9, 0.5, 0, 2);
      }
    }
    // Off-road: dust, snow or cinders kicked up behind.
    if (near && r.grounded && r.surface === 'grass' && Math.abs(r.speed) > 6 && Math.random() < 0.8) {
      this.smoke.spawn(r.x - fx * 1.2, r.y + 0.3, r.z - fz * 1.2, (Math.random() - 0.5) * 2 - fx * 2, 1.2 + Math.random(), (Math.random() - 0.5) * 2 - fz * 2, this.dustColor(), 1.0, 0.55, 0, 2);
    }
    // Skimming the water surface: a rooster tail of spray.
    const liquid = this.track.liquid;
    if (near && liquid !== null && !this.track.lava && r.grounded && !r.under && r.y < liquid + 0.3 && r.y > liquid - 1.4 && Math.abs(r.speed) > 8) {
      for (const side of [-1, 1]) this.particles.spawn(r.x - fx * 1.3 + lx * side * 0.8, liquid + 0.2, r.z - fz * 1.3 + lz * side * 0.8, -fx * 3 + lx * side * 2.5, 4 + Math.random() * 3, -fz * 3 + lz * side * 2.5, 0xe8fbff, 0.55, 0.6, -14, 1);
    }
    // Underwater: a trail of bubbles rising to the surface.
    if (near && r.under && Math.random() < 0.6) this.particles.spawn(r.x - fx * 1.4 + (Math.random() - 0.5), r.y + 0.6, r.z - fz * 1.4 + (Math.random() - 0.5), (Math.random() - 0.5) * 0.6, 2 + Math.random() * 1.5, (Math.random() - 0.5) * 0.6, 0xd8f6ff, 0.35, 1.6, 1.5, 0.5);
    if (near && boosting && Math.random() < 0.9) this.particles.spawn(r.x - fx * 1.9, r.y + 0.6, r.z - fz * 1.9, -fx * 5, 0.5, -fz * 5, v.flameTint, 0.55, 0.25, 0, 3);
    // Burnout: tyre smoke pouring off the spinning rear wheels.
    if (near && r.stall > 0 && Math.random() < 0.8) for (const side of [-1, 1]) this.smoke.spawn(r.x - fx * 1.0 + lx * side * 0.9, r.y + 0.3, r.z - fz * 1.0 + lz * side * 0.9, (Math.random() - 0.5) * 1.5 - fx * 1.5, 0.8 + Math.random(), (Math.random() - 0.5) * 1.5 - fz * 1.5, 0x5a5a5a, 2, 0.9, 0, 1.5);
    // Slipstream: wind streaks pouring past while the draft charges.
    if (near && r.draft > 0.25 && Math.random() < 0.7) {
      const side = Math.random() < 0.5 ? -1 : 1;
      this.particles.spawn(r.x + fx * 2.5 + lx * side * (1 + Math.random()), r.y + 0.6 + Math.random() * 1.4, r.z + fz * 2.5 + lz * side * (1 + Math.random()), -fx * 22, 0, -fz * 22, 0xdff6ff, 0.18, 0.3, 0, 0);
    }
    if (r.golden > 0 && Math.random() < 0.6) this.particles.spawn(r.x + (Math.random() - 0.5) * 2.4, r.y + Math.random() * 2, r.z + (Math.random() - 0.5) * 2.4, 0, 1.2, 0, 0xffd84a, 0.4, 0.5);
    if (r.star > 0 && Math.random() < 0.5) this.particles.spawn(r.x + (Math.random() - 0.5) * 2, r.y + Math.random() * 2, r.z + (Math.random() - 0.5) * 2, 0, 1, 0, 0xfff27a, 0.4, 0.5);
  }

  /** Blue fog once the camera dips under the water surface. */
  private updateFog(): void {
    const { track, theme } = this;
    const under = track.liquid !== null && !track.lava && this.view.camera.position.y < track.liquid;
    if (under === this.underwater) return;
    this.underwater = under;
    const fog = this.view.scene.fog as THREE.Fog;
    fog.color.setHex(under ? DEEP.color : theme.fog);
    fog.near = under ? DEEP.near : theme.fogNear;
    fog.far = under ? DEEP.far : theme.fogFar;
    (this.view.scene.background as THREE.Color).setHex(under ? DEEP.color : theme.skyHorizon);
    this.sky.uniforms.under.value = under ? 1 : 0;
  }
  /** Whether the camera is under water (the HUD tints the view). */
  get isUnderwater(): boolean { return this.underwater; }

  private updateCamera(state: KartState, self: KartState['racers'][number], dt: number, lookBack: boolean): void {
    const cam = this.view.camera;
    const path = this.track.paths[self.seg] ?? this.track;
    if (this.pin) {
      cam.position.set(this.pin[0], this.pin[1], this.pin[2]);
      cam.lookAt(this.pin[3], this.pin[4], this.pin[5]);
      cam.fov = 68; cam.updateProjectionMatrix();
      return;
    }
    if (state.phase === 'intro') {
      // Sweep around the grid before the countdown.
      const t = Math.min(1, state.clock / 2.5);
      const ease = t * t * (3 - 2 * t);
      const a = self.yaw + Math.PI * (1 - ease);
      const d = lerp(34, 8, ease), h = lerp(30, 3.4, ease * ease);
      cam.position.set(self.x + Math.sin(a) * d, self.y + h, self.z + Math.cos(a) * d);
      // Keep the sweep above walls/cliffs beside the grid.
      const n0 = nearest(path, cam.position.x, cam.position.z, self.hint, _near, 40);
      cam.position.y = Math.max(cam.position.y, this.track.ground(cam.position.x, cam.position.z, n0) + 6);
      cam.lookAt(self.x, self.y + 1.2, self.z);
      this.resetCamera(cam.position, _look.set(self.x, self.y + 1.2, self.z));
      return;
    }
    const heading = r_heading(self);
    const back = lookBack ? -1 : 1;
    // Pulled back a little at speed and on a boost, further on the glider.
    const dist = 7.8 + clamp(self.speed, 0, 40) * 0.05 + (self.boost > 0 ? 0.9 : 0) + (self.glide ? 3 : 0);
    _v.set(self.x - Math.sin(heading) * dist * back, self.y + 3.4 + (self.glide ? 1.6 : 0), self.z - Math.cos(heading) * dist * back);
    // Keep the camera above the terrain, and under the roof in tunnels.
    const n = nearest(path, _v.x, _v.z, self.hint, _near, 20);
    // On walled roads keep the camera on the road side of the barrier: a kart drifting along a wall
    // swings it outward, into pit canopies, grandstands and buildings (fork mouths have no wall).
    let walls = this.camWalls.get(path.index);
    if (!walls) { walls = barriers(this.track, path).filter(b => b.kind === 'wall'); this.camWalls.set(path.index, walls); }
    for (const b of walls) {
      const limit = Math.abs(b.lateral) - 0.6;
      if (b.mask[n.index] && Math.sign(n.lateral) === b.side && Math.abs(n.lateral) > limit) {
        pointAt(path, n.s, b.side * limit, _cp);
        _v.x = _cp[0]; _v.z = _cp[2];
      }
    }
    const ground = this.track.ground(_v.x, _v.z, n, state.raceTime);
    if (Math.abs(n.lateral) < path.halfWidth + 6 || !(path.drop[n.index] > 0)) _v.y = Math.max(_v.y, ground + 1.5);
    if (path.flags[n.index] & F_TUNNEL && Math.abs(n.lateral) < path.halfWidth + 1) _v.y = Math.min(_v.y, n.y + TUNNEL_HEIGHT - 1.4);
    _look.set(self.x + Math.sin(heading) * 4 * back, self.y + 1.3, self.z + Math.cos(heading) * 4 * back);
    if (!this.camReady || lookBack) this.resetCamera(_v, _look);
    // A critically damped spring trails the kart (it lags ~2·speed/ω behind: a few metres more on a boost)
    // the look point leads it more tightly.
    const pos = this.camPos;
    cam.position.set(springTo(pos.x, _v.x, 15, dt), springTo(pos.y, _v.y, 11, dt), springTo(pos.z, _v.z, 15, dt));
    this.camLook.x = approach(this.camLook.x, _look.x, 14, dt); this.camLook.y = approach(this.camLook.y, _look.y, 10, dt); this.camLook.z = approach(this.camLook.z, _look.z, 14, dt);
    cam.lookAt(this.camLook);
    // Shake on landings, hits and nearby blasts.
    this.shake.update(dt);
    if (this.shake.trauma > 0) {
      cam.position.x += this.shake.offset(this.time, 0, 0.35); cam.position.y += this.shake.offset(this.time, 1, 0.3);
      cam.rotation.z += this.shake.offset(this.time, 2, 0.05);
    }
    cam.fov = approach(cam.fov, chaseFov(self.speed, self.boost, self.boostPower, self.glide), self.boost > 0 ? 7 : 3, dt);
    cam.updateProjectionMatrix();
  }
  private resetCamera(pos: THREE.Vector3, look: THREE.Vector3): void {
    const c = this.camPos;
    for (const [s, x] of [[c.x, pos.x], [c.y, pos.y], [c.z, pos.z]] as [Spring, number][]) { s.x = x; s.v = 0; }
    this.camLook.copy(look);
    this.camReady = true;
  }

  stats() { return { ...this.view.stats(), camera: { x: this.camPos.x.x, y: this.camPos.y.x, z: this.camPos.z.x }, underwater: this.underwater }; }

  dispose(): void {
    this.particles.dispose();
    this.smoke.dispose();
    for (const k of this.karts) k.mixer?.stopAllAction();
    for (const o of this.obstacles) o.mesh.removeFromParent();
    this.view.dispose();
    this.lib.dispose();
  }
}

/** Camera follows the direction of travel (velocity) blended with the heading. */
function r_heading(r: KartState['racers'][number]): number {
  const sp = Math.hypot(r.vx, r.vz);
  if (sp < 3 || r.speed < 0) return r.yaw;
  const vel = Math.atan2(r.vx, r.vz);
  return r.yaw + wrapAngle(vel - r.yaw) * 0.6;
}
