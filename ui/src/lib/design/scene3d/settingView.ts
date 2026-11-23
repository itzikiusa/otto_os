// Draws a Smart mode SETTING in three (the parameters come from `setting.ts`):
// a sky dome that follows the camera, the ground by kind (water with moving
// ripples — a scrolled, generated normal map on a glossy plane, so the sky
// and the lights reflect in it; the others a generated speckle, plank or tile
// texture; a road an asphalt strip with a dashed line), a room's two walls,
// fog, falling rain or snow, and a soft halo at each glowing light. Nothing
// is fetched. Shared by the viewport, the embed runtime and their captures;
// it lives outside the content group, so picking, framing and export never
// see it. Under prefers-reduced-motion nothing moves.
import type * as THREE_NS from 'three';
import type { Three } from './build';
import type { LocalEffect, Scene3dEnvironment, Scene3dLight, Vec3 } from './types';
import { fogOf, groundLook, halosOf, precipitationOf, ROAD_WIDTH, settingKey, SKY_BODY_LOOKS, wallColor, wallsOf, type GroundLook, type Precipitation } from './setting';

/** A tiny repeatable random (the same picture every time). */
function random(seed: number): () => number {
  let s = seed >>> 0;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
}

function canvas(size: number): [HTMLCanvasElement, CanvasRenderingContext2D] | null {
  if (typeof document === 'undefined') return null;
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const g = c.getContext('2d');
  return g ? [c, g] : null;
}

/** A tileable ripple normal map: summed waves of whole periods across the tile. */
function rippleNormals(T: Three): THREE_NS.Texture | null {
  const n = 128;
  const cv = canvas(n);
  if (!cv) return null;
  const [c, g] = cv;
  // Many small waves of different directions, so no one pattern repeats visibly.
  const waves = [
    [3, 1, 0.5, 0.2],
    [-2, 5, 0.4, 1.3],
    [7, -4, 0.3, 2.1],
    [1, 9, 0.25, 0.7],
    [-9, -2, 0.2, 2.9],
    [11, 6, 0.15, 1.1],
  ];
  const img = g.createImageData(n, n);
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      let dx = 0;
      let dy = 0;
      for (const [fx, fy, a, p] of waves) {
        const k = Math.cos(((fx * x + fy * y) / n) * 2 * Math.PI + p) * a * 2 * Math.PI / n;
        dx += k * fx;
        dy += k * fy;
      }
      const len = Math.hypot(dx * 2.5, dy * 2.5, 1);
      const i = (y * n + x) * 4;
      img.data[i] = (((-dx * 2.5) / len) * 0.5 + 0.5) * 255;
      img.data[i + 1] = (((-dy * 2.5) / len) * 0.5 + 0.5) * 255;
      img.data[i + 2] = ((1 / len) * 0.5 + 0.5) * 255;
      img.data[i + 3] = 255;
    }
  }
  g.putImageData(img, 0, 0);
  const t = new T.CanvasTexture(c);
  t.wrapS = t.wrapT = T.RepeatWrapping;
  return t;
}

/** The ground's colour texture: speckle, planks, tiles or a road's verge. */
function groundTexture(T: Three, look: GroundLook): THREE_NS.Texture | null {
  const n = 256;
  const cv = canvas(n);
  if (!cv) return null;
  const [c, g] = cv;
  const rnd = random(7);
  g.fillStyle = look.color;
  g.fillRect(0, 0, n, n);
  g.fillStyle = look.detail;
  for (let i = 0; i < 900; i++) {
    g.globalAlpha = 0.25 + rnd() * 0.4;
    const s = 1 + rnd() * 3;
    g.fillRect(rnd() * n, rnd() * n, s, s);
  }
  g.globalAlpha = 1;
  g.strokeStyle = look.detail;
  if (look.pattern === 'planks') {
    // Eight planks across the tile, their ends staggered.
    g.lineWidth = 2;
    for (let k = 0; k < 8; k++) {
      const y = (k * n) / 8;
      g.beginPath();
      g.moveTo(0, y);
      g.lineTo(n, y);
      const x = ((k * 5) % 8) * (n / 8);
      g.moveTo(x, y);
      g.lineTo(x, y + n / 8);
      g.stroke();
    }
  } else if (look.pattern === 'tiles') {
    g.lineWidth = 3;
    for (let k = 0; k <= 4; k++) {
      const p = (k * n) / 4;
      g.beginPath();
      g.moveTo(p, 0);
      g.lineTo(p, n);
      g.moveTo(0, p);
      g.lineTo(n, p);
      g.stroke();
    }
  }
  const t = new T.CanvasTexture(c);
  t.colorSpace = T.SRGBColorSpace;
  t.wrapS = t.wrapT = T.RepeatWrapping;
  return t;
}

/** A soft round glow (white; the sprite tints it). */
function haloTexture(T: Three): THREE_NS.Texture | null {
  const cv = canvas(64);
  if (!cv) return null;
  const [c, g] = cv;
  const r = g.createRadialGradient(32, 32, 0, 32, 32, 32);
  r.addColorStop(0, 'rgba(255,255,255,1)');
  r.addColorStop(0.25, 'rgba(255,255,255,0.55)');
  r.addColorStop(1, 'rgba(255,255,255,0)');
  g.fillStyle = r;
  g.fillRect(0, 0, 64, 64);
  const t = new T.CanvasTexture(c);
  t.colorSpace = T.SRGBColorSpace;
  return t;
}

/** A sky dome round the camera: zenith → horizon → below, in vertex colours. */
function skyDome(T: Three, sky: [string, string, string], radius: number): THREE_NS.Mesh {
  const geo = new T.SphereGeometry(radius, 32, 16);
  const [top, mid, bottom] = sky.map((s) => new T.Color(s));
  const pos = geo.getAttribute('position');
  const colors = new Float32Array(pos.count * 3);
  const c = new T.Color();
  for (let i = 0; i < pos.count; i++) {
    const y = pos.getY(i) / radius;
    if (y >= 0) c.copy(mid).lerp(top, Math.pow(y, 0.5));
    else c.copy(mid).lerp(bottom, Math.min(1, -y * 4));
    colors.set([c.r, c.g, c.b], i * 3);
  }
  geo.setAttribute('color', new T.BufferAttribute(colors, 3));
  const mat = new T.MeshBasicMaterial({ vertexColors: true, side: T.BackSide, fog: false, depthWrite: false });
  const dome = new T.Mesh(geo, mat);
  dome.name = 'sky';
  dome.frustumCulled = false;
  dome.renderOrder = -2;
  dome.matrixAutoUpdate = false;
  // Centred on the camera every frame (before its model-view matrix is made).
  dome.onBeforeRender = (_r, _s, cam) => dome.matrixWorld.setPosition(cam.position);
  return dome;
}

/** A body in the sky: a disc, lit on one side (the Earth with a few lands and clouds, the Moon with seas). */
function skyBodyTexture(T: Three, kind: keyof typeof SKY_BODY_LOOKS): THREE_NS.Texture | null {
  const cv = canvas(128);
  if (!cv) return null;
  const [c, g] = cv;
  const look = SKY_BODY_LOOKS[kind];
  if (look.shines) {
    const r = g.createRadialGradient(64, 64, 0, 64, 64, 64);
    r.addColorStop(0, look.color);
    r.addColorStop(0.35, look.color);
    r.addColorStop(0.5, look.edge);
    r.addColorStop(1, 'rgba(255,200,120,0)');
    g.fillStyle = r;
    g.fillRect(0, 0, 128, 128);
  } else {
    g.save();
    g.beginPath();
    g.arc(64, 64, 60, 0, Math.PI * 2);
    g.clip();
    g.fillStyle = look.color;
    g.fillRect(0, 0, 128, 128);
    const rnd = random(kind.length * 31);
    g.fillStyle = kind === 'earth' ? '#5f9a4a' : look.edge;
    for (let i = 0; i < (kind === 'earth' ? 7 : 9); i++) {
      g.beginPath();
      g.ellipse(20 + rnd() * 88, 20 + rnd() * 88, 6 + rnd() * 16, 5 + rnd() * 10, rnd() * 3, 0, Math.PI * 2);
      g.fill();
    }
    // The night side.
    const shade = g.createLinearGradient(20, 0, 128, 0);
    shade.addColorStop(0, 'rgba(0,0,0,0)');
    shade.addColorStop(1, 'rgba(0,0,0,0.75)');
    g.fillStyle = shade;
    g.fillRect(0, 0, 128, 128);
    g.restore();
  }
  const t = new T.CanvasTexture(c);
  t.colorSpace = T.SRGBColorSpace;
  return t;
}

export class SettingView {
  private readonly root: THREE_NS.Group;
  private key = '';
  private water: THREE_NS.Texture | null = null;
  private rain: { obj: THREE_NS.Object3D; p: Precipitation; pos: Float32Array; attr: THREE_NS.BufferAttribute } | null = null;
  private effects: ((time: number) => void)[] = [];
  private elapsed = 0;
  private last = 0;
  private readonly still: boolean;

  constructor(
    private readonly T: Three,
    private readonly scene: THREE_NS.Scene,
  ) {
    this.root = new T.Group();
    this.root.name = 'setting';
    scene.add(this.root);
    this.still = typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  }

  /** The water ripples or the weather falls (a frame is wanted every tick). */
  get animating(): boolean {
    return !this.still && (!!this.water || !!this.rain || this.effects.length > 0);
  }

  /** Draw `env` (rebuilt only when what it draws changed). */
  apply(env: Scene3dEnvironment | undefined, lights: Scene3dLight[], camera: { position: Vec3; target: Vec3; far?: number }): void {
    const key = settingKey(env, lights, camera.position, camera.target);
    if (key === this.key) return;
    this.key = key;
    this.clear();
    const T = this.T;
    const fog = fogOf(env);
    this.scene.fog = fog ? new T.FogExp2(fog.color, fog.density) : null;
    if (!env || !key) return;
    const radius = 0.4 * (camera.far ?? 500);
    if (env.sky) this.root.add(skyDome(T, env.sky, radius));
    if (env.stars) this.root.add(this.stars(camera.position, 0.95 * radius));
    for (const b of env.sky_bodies ?? []) {
      const look = SKY_BODY_LOOKS[b.kind];
      const s = new T.Sprite(new T.SpriteMaterial({ map: skyBodyTexture(T, b.kind), fog: false, depthWrite: false, transparent: true }));
      s.name = b.kind;
      const d = new T.Vector3(...b.direction).normalize().multiplyScalar(0.9 * radius);
      s.position.set(...camera.position).add(d);
      s.scale.setScalar(2 * 0.9 * radius * Math.tan(((look.size / 2) * Math.PI) / 180));
      s.renderOrder = -1;
      this.root.add(s);
    }
    const g = env.ground;
    if (g) {
      const look = groundLook(g.kind);
      const mat = new T.MeshStandardMaterial({ color: look.pattern === 'water' ? look.color : '#ffffff', roughness: look.roughness, metalness: look.metalness });
      const repeat = (s: number) => Math.max(1, s / look.tile);
      if (look.pattern === 'water') {
        this.water = rippleNormals(T);
        mat.normalMap = this.water;
        mat.normalScale.set(0.35, 0.35);
        this.water?.repeat.set(repeat(g.size[0]), repeat(g.size[1]));
      } else {
        mat.map = groundTexture(T, look);
        mat.map?.repeat.set(repeat(g.size[0]), repeat(g.size[1]));
      }
      const plane = new T.Mesh(new T.PlaneGeometry(g.size[0], g.size[1]), mat);
      plane.name = 'ground';
      plane.rotation.x = -Math.PI / 2;
      plane.position.set(...g.center);
      plane.receiveShadow = true;
      this.root.add(plane);
      if (look.pattern === 'road') this.root.add(this.road(g.center, g.size[g.axis === 'z' ? 1 : 0], look.detail, g.axis));
    }
    const sh = env.shore;
    if (sh) {
      // A low land strip from under the water up to its top.
      const look = groundLook(sh.kind);
      const mat = new T.MeshStandardMaterial({ color: '#ffffff', roughness: look.roughness, map: groundTexture(T, look) });
      mat.map?.repeat.set(Math.max(1, sh.size[0] / look.tile), Math.max(1, sh.size[1] / look.tile));
      const depth = sh.center[1] + 2;
      const land = new T.Mesh(new T.BoxGeometry(sh.size[0], depth, sh.size[1]), mat);
      land.name = 'shore';
      land.position.set(sh.center[0], sh.center[1] - depth / 2, sh.center[2]);
      land.receiveShadow = true;
      this.root.add(land);
    }
    for (const w of wallsOf(env, camera.position)) {
      const wall = new T.Mesh(new T.PlaneGeometry(w.width, w.height), new T.MeshStandardMaterial({ color: wallColor(g?.kind), roughness: 0.9 }));
      wall.name = 'wall';
      wall.position.set(...w.position);
      wall.rotation.y = w.rotationY;
      wall.receiveShadow = true;
      this.root.add(wall);
    }
    for (const effect of env.local_effects ?? []) this.addEffect(effect);
    const p = precipitationOf(env, camera.target);
    if (p) this.rain = this.fall(p);
    const tex = halosOf(lights).length ? haloTexture(T) : null;
    for (const h of halosOf(lights)) {
      const s = new T.Sprite(new T.SpriteMaterial({ map: tex, color: h.color, blending: T.AdditiveBlending, depthWrite: false, fog: false, transparent: true }));
      s.name = 'glow';
      s.position.set(...h.position);
      s.scale.setScalar(h.radius * 2);
      this.root.add(s);
    }
  }

  /** A film on a pane or a plume rooted in a steaming surface. Both are already
   * visible in the opening frame and stay still when reduced motion is on. */
  private addEffect(effect: LocalEffect): void {
    const T = this.T;
    const [w, h] = effect.size;
    const group = new T.Group();
    group.name = effect.kind;
    group.position.set(...effect.position);
    group.rotation.y = (effect.rotation * Math.PI) / 180;
    this.root.add(group);
    if (effect.kind === 'rain_on_glass') {
      const cv = canvas(256);
      if (!cv) return;
      const [c, g] = cv;
      const rnd = random(18);
      g.fillStyle = 'rgba(83,126,161,0.29)';
      g.fillRect(0, 0, 256, 256);
      // A tile of uneven wet trails, with bright beads collecting at their tips.
      for (let i = 0; i < 64; i++) {
        const x = rnd() * 256, y = rnd() * 256, length = 4 + 30 * rnd(), bend = 6 * rnd() - 3;
        for (const off of [-256, 0, 256]) {
          g.strokeStyle = 'rgba(162,202,226,0.65)';
          g.lineWidth = 1.2 + length / 24;
          g.beginPath();
          g.moveTo(x - bend, y + off - length);
          g.quadraticCurveTo(x + bend, y + off - length * 0.4, x, y + off);
          g.stroke();
          g.fillStyle = 'rgba(213,235,250,0.84)';
          g.beginPath();
          g.ellipse(x, y + off, 2, 3, 0, 0, Math.PI * 2);
          g.fill();
        }
      }
      const tex = new T.CanvasTexture(c);
      tex.colorSpace = T.SRGBColorSpace;
      tex.wrapT = T.RepeatWrapping;
      const pane = new T.Mesh(new T.PlaneGeometry(w, h), new T.MeshBasicMaterial({ map: tex, transparent: true, depthWrite: false, side: T.DoubleSide }));
      group.add(pane);
      this.effects.push((t) => { tex.offset.y = (t * 0.064) % 1; });
      return;
    }
    const tex = haloTexture(T);
    for (let i = 0; i < 11; i++) {
      const puff = new T.Sprite(new T.SpriteMaterial({ map: tex, color: '#ffffff', transparent: true, depthWrite: false }));
      group.add(puff);
      const update = (t: number) => {
        const phase = (i / 11 + t * 0.38) % 1;
        const sway = Math.sin(phase * 8 + 0.6) * w * phase * 0.42;
        puff.position.set(sway, h * phase, Math.cos(phase * 6) * w * 0.12);
        const diameter = w * (0.27 + 0.68 * phase);
        puff.scale.set(diameter, diameter * 1.45, 1);
        puff.material.opacity = Math.min(1, 0.18 + phase * 10) * (1 - phase) * 0.74;
      };
      update(0);
      this.effects.push(update);
    }
  }

  /** Stars over the upper sky, round where the camera stands. */
  private stars(at: Vec3, r: number): THREE_NS.Object3D {
    const T = this.T;
    const rnd = random(5);
    const n = 700;
    const pos = new Float32Array(n * 3);
    for (let i = 0; i < n; i++) {
      const a = rnd() * Math.PI * 2;
      const y = 0.03 + rnd() * 0.97;
      const h = Math.sqrt(1 - y * y);
      pos.set([at[0] + Math.cos(a) * h * r, at[1] + y * r, at[2] + Math.sin(a) * h * r], i * 3);
    }
    const geo = new T.BufferGeometry();
    geo.setAttribute('position', new T.BufferAttribute(pos, 3));
    const stars = new T.Points(geo, new T.PointsMaterial({ color: '#ffffff', size: 1.6, sizeAttenuation: false, fog: false, depthWrite: false, transparent: true, opacity: 0.85 }));
    stars.name = 'stars';
    stars.frustumCulled = false;
    stars.renderOrder = -1;
    return stars;
  }

  /** An asphalt strip with a dashed middle line, centred on the ground, oriented along the parked vehicles. */
  private road(center: Vec3, length: number, color: string, axis = 'x'): THREE_NS.Object3D {
    const T = this.T;
    const strip = new T.Group();
    strip.position.set(...center);
    if (axis === 'z') strip.rotation.y = Math.PI / 2;
    const lay = (w: number, l: number, c: string, x: number, lift: number) => {
      const m = new T.Mesh(new T.PlaneGeometry(l, w), new T.MeshStandardMaterial({ color: c, roughness: 0.9 }));
      m.rotation.x = -Math.PI / 2;
      m.position.set(x, lift, 0);
      m.receiveShadow = true;
      strip.add(m);
    };
    lay(ROAD_WIDTH, Math.min(length, 400), color, 0, 0.01);
    for (let x = -60; x <= 60; x += 4) lay(0.15, 2, '#e8e2c8', x, 0.02);
    return strip;
  }

  private fall(p: Precipitation): SettingView['rain'] {
    const T = this.T;
    const rnd = random(11);
    const rain = p.kind === 'rain';
    const pos = new Float32Array(p.count * (rain ? 6 : 3));
    for (let i = 0; i < p.count; i++) {
      const at = [0, 1, 2].map((k) => p.center[k] + (rnd() - 0.5) * p.size[k]);
      pos.set(at, i * (rain ? 6 : 3));
      if (rain) pos.set([at[0], at[1] + 0.35, at[2]], i * 6 + 3);
    }
    const geo = new T.BufferGeometry();
    const attr = new T.BufferAttribute(pos, 3);
    geo.setAttribute('position', attr);
    const obj = rain
      ? new T.LineSegments(geo, new T.LineBasicMaterial({ color: '#b8c4d0', transparent: true, opacity: 0.45 }))
      : new T.Points(geo, new T.PointsMaterial({ color: '#ffffff', size: 0.07, transparent: true, opacity: 0.9 }));
    obj.name = p.kind;
    obj.frustumCulled = false;
    this.root.add(obj);
    return { obj, p, pos, attr };
  }

  /** Move the ripples and the rain one tick; true while it keeps moving. */
  step(now: number): boolean {
    const dt = this.last ? Math.min(0.1, (now - this.last) / 1000) : 0;
    this.last = now;
    if (!this.animating) return false;
    this.elapsed += dt;
    for (const effect of this.effects) effect(this.elapsed);
    if (this.water) {
      this.water.offset.x += dt * 0.03;
      this.water.offset.y += dt * 0.018;
    }
    if (this.rain) {
      const { p, pos, attr } = this.rain;
      // A streak (two points) or a flake (one) falls and wraps to the top whole.
      const [stride, ends] = p.kind === 'rain' ? [6, [1, 4]] : [3, [1]];
      const bottom = p.center[1] - p.size[1] / 2;
      for (let i = 0; i < pos.length; i += stride) {
        const wrap = pos[i + 1] - p.speed * dt < bottom ? p.size[1] : 0;
        for (const e of ends) pos[i + e] += wrap - p.speed * dt;
      }
      attr.needsUpdate = true;
    }
    return true;
  }

  private clear(): void {
    for (const o of [...this.root.children]) {
      o.removeFromParent();
      o.traverse((n) => {
        const m = n as THREE_NS.Mesh;
        m.geometry?.dispose();
        const mat = m.material as (THREE_NS.Material & { map?: THREE_NS.Texture | null; normalMap?: THREE_NS.Texture | null }) | undefined;
        mat?.map?.dispose();
        mat?.normalMap?.dispose();
        mat?.dispose();
      });
    }
    this.effects = [];
    this.elapsed = 0;
    this.water = null;
    this.rain = null;
  }

  dispose(): void {
    this.clear();
    this.root.removeFromParent();
    this.scene.fog = null;
  }
}
