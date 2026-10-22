/**
 * Weather in the scene: rain streaks and drifting ash / embers in a box that
 * follows the camera (one buffer each, rewritten in place), lightning flashes
 * in rain, the local survivor's flashlight at night and faint beam cones for
 * everyone else's. Applies the preset's sky, fog and light to the GameView.
 */
import * as THREE from 'three';
import type { GameView, Theme } from '../../common/view';
import { WEATHER, weatherTheme, type WeatherId } from './weather';

const RAIN = 2600, ASH = 1400, BEAMS = 24;
const BOX = 34, HEIGHT = 26;
const _m = new THREE.Matrix4(), _q = new THREE.Quaternion(), _p = new THREE.Vector3(), _s = new THREE.Vector3(), _e = new THREE.Euler(0, 0, 0, 'YXZ');
const ZERO = new THREE.Matrix4().makeScale(0, 0, 0);

export class Atmosphere {
  readonly group = new THREE.Group();
  readonly flashlight: THREE.SpotLight;
  weather: WeatherId;
  private view: GameView;
  private base: Theme;
  private rain: THREE.LineSegments; private rainPos: Float32Array;
  private ash: THREE.Points; private ashPos: Float32Array; private ashSeed: Float32Array;
  private beams: THREE.InstancedMesh; private beamCount = 0;
  private lightning = 0; private nextBolt = 6; private time = 0;
  private resources: { dispose(): void }[] = [];

  constructor(view: GameView, base: Theme, weather: WeatherId) {
    this.view = view; this.base = base; this.weather = weather;
    this.rainPos = new Float32Array(RAIN * 6);
    for (let i = 0; i < RAIN; i++) this.seedDrop(i, true);
    const rg = new THREE.BufferGeometry();
    rg.setAttribute('position', new THREE.BufferAttribute(this.rainPos, 3).setUsage(THREE.DynamicDrawUsage));
    const rm = new THREE.LineBasicMaterial({ color: 0xb8c6d4, transparent: true, opacity: 0.32, depthWrite: false });
    this.rain = new THREE.LineSegments(rg, rm);
    this.rain.frustumCulled = false;
    this.ashPos = new Float32Array(ASH * 3); this.ashSeed = new Float32Array(ASH);
    const colors = new Float32Array(ASH * 3);
    for (let i = 0; i < ASH; i++) {
      this.ashPos[i * 3] = (Math.random() - 0.5) * BOX * 2; this.ashPos[i * 3 + 1] = Math.random() * HEIGHT; this.ashPos[i * 3 + 2] = (Math.random() - 0.5) * BOX * 2;
      this.ashSeed[i] = Math.random() * 100;
      // One in twelve flakes is a glowing ember.
      const ember = i % 12 === 0;
      colors[i * 3] = ember ? 1 : 0.78; colors[i * 3 + 1] = ember ? 0.48 : 0.74; colors[i * 3 + 2] = ember ? 0.18 : 0.7;
    }
    const ag = new THREE.BufferGeometry();
    ag.setAttribute('position', new THREE.BufferAttribute(this.ashPos, 3).setUsage(THREE.DynamicDrawUsage));
    ag.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    // Soft round flakes, a few pixels at most even right in front of the camera.
    const am = new THREE.ShaderMaterial({
      transparent: true, depthWrite: false, vertexColors: true,
      uniforms: { scale: { value: 420 } },
      vertexShader: 'varying vec3 vColor; uniform float scale; void main(){ vColor = color; vec4 mv = modelViewMatrix * vec4(position, 1.0); gl_PointSize = clamp(0.09 * scale / -mv.z, 1.0, 4.5); gl_Position = projectionMatrix * mv; }',
      fragmentShader: 'varying vec3 vColor; void main(){ vec2 d = gl_PointCoord - 0.5; float r = dot(d, d); if (r > 0.25) discard; gl_FragColor = vec4(vColor, 0.85 * (1.0 - r * 4.0)); }',
    });
    this.ash = new THREE.Points(ag, am);
    this.ash.frustumCulled = false;
    // A cone from the lamp (apex) that fades out toward its far end.
    const beamGeo = new THREE.ConeGeometry(1, 1, 18, 4, true).translate(0, -0.5, 0).rotateX(-Math.PI / 2);
    const bp = beamGeo.getAttribute('position'), fade = new Float32Array(bp.count * 3);
    for (let i = 0; i < bp.count; i++) { const k = Math.pow(1 - Math.min(1, bp.getZ(i)), 1.6); fade[i * 3] = fade[i * 3 + 1] = fade[i * 3 + 2] = k; }
    beamGeo.setAttribute('color', new THREE.BufferAttribute(fade, 3));
    const beamMat = new THREE.MeshBasicMaterial({ color: 0xfff1cc, vertexColors: true, transparent: true, opacity: 0.12, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide, fog: true });
    this.beams = new THREE.InstancedMesh(beamGeo, beamMat, BEAMS);
    this.beams.count = 0; this.beams.frustumCulled = false; this.beams.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    this.flashlight = new THREE.SpotLight(0xfff0d6, 0, 48, 0.42, 0.55, 1.4);
    this.group.add(this.rain, this.ash, this.beams, this.flashlight, this.flashlight.target);
    this.resources.push(rg, rm, ag, am, beamGeo, beamMat);
    this.apply(weather);
  }

  /** Switches the preset: sky, fog, lights, exposure and which particles show. */
  apply(id: WeatherId): void {
    this.weather = id;
    const w = WEATHER[id];
    this.view.setAtmosphere(weatherTheme(this.base, id), _p.set(w.sun[0], w.sun[1], w.sun[2]));
    this.view.renderer.toneMappingExposure = w.exposure;
    this.rain.visible = w.rain > 0; this.ash.visible = w.ash > 0;
    this.flashlight.intensity = 0;
    this.beams.visible = w.flashlight;
  }

  private seedDrop(i: number, anywhere: boolean): void {
    const x = (Math.random() - 0.5) * BOX * 2, z = (Math.random() - 0.5) * BOX * 2, y = anywhere ? Math.random() * HEIGHT : HEIGHT;
    const o = i * 6;
    this.rainPos[o] = x; this.rainPos[o + 1] = y; this.rainPos[o + 2] = z;
    this.rainPos[o + 3] = x + 0.12; this.rainPos[o + 4] = y + 0.9; this.rainPos[o + 5] = z + 0.06;
  }

  /** Starts a frame of flashlight beams (others' lights at night). */
  beginBeams(): void { this.beamCount = 0; }
  /** A beam from a head at (x, y, z) looking along yaw / pitch. */
  beam(x: number, y: number, z: number, yaw: number, pitch: number): void {
    if (!this.beams.visible || this.beamCount >= BEAMS) return;
    _e.set(-pitch, yaw, 0, 'YXZ');
    _q.setFromEuler(_e);
    _m.compose(_p.set(x, y, z), _q, _s.set(3.2, 3.2, 16));
    this.beams.setMatrixAt(this.beamCount++, _m);
  }
  endBeams(): void {
    this.beams.count = this.beamCount;
    for (let i = this.beamCount; i < Math.min(BEAMS, this.beamCount + 1); i++) this.beams.setMatrixAt(i, ZERO);
    this.beams.instanceMatrix.needsUpdate = true;
  }

  /**
   * Moves the particle boxes with the camera, steps rain and ash, rolls
   * lightning, and aims the flashlight from the eye along the view.
   * Returns a lightning brightness 0..1 (the caller may flash the screen).
   */
  update(dt: number, camera: THREE.Camera, eye: THREE.Vector3 | null, look: THREE.Vector3, fpv: boolean): number {
    this.time += dt;
    const w = WEATHER[this.weather];
    const cx = camera.position.x, cy = camera.position.y, cz = camera.position.z;
    if (this.rain.visible) {
      this.rain.position.set(cx, cy - HEIGHT * 0.45, cz);
      const fall = 30 * dt, drift = 2.4 * dt;
      for (let i = 0; i < RAIN; i++) {
        const o = i * 6;
        this.rainPos[o + 1] -= fall; this.rainPos[o + 4] -= fall; this.rainPos[o] += drift; this.rainPos[o + 3] += drift;
        if (this.rainPos[o + 1] < 0) this.seedDrop(i, false);
      }
      (this.rain.geometry.getAttribute('position') as THREE.BufferAttribute).needsUpdate = true;
    }
    if (this.ash.visible) {
      this.ash.position.set(cx, cy - HEIGHT * 0.4, cz);
      for (let i = 0; i < ASH; i++) {
        const o = i * 3, s = this.ashSeed[i];
        this.ashPos[o] += (Math.sin(this.time * 0.7 + s) * 0.6 + 0.5) * dt;
        this.ashPos[o + 1] -= (0.45 + (s % 1) * 0.5) * dt;
        this.ashPos[o + 2] += Math.cos(this.time * 0.5 + s * 1.3) * 0.5 * dt;
        if (this.ashPos[o + 1] < 0) this.ashPos[o + 1] += HEIGHT;
        if (this.ashPos[o] > BOX) this.ashPos[o] -= BOX * 2;
        if (this.ashPos[o + 2] > BOX) this.ashPos[o + 2] -= BOX * 2; else if (this.ashPos[o + 2] < -BOX) this.ashPos[o + 2] += BOX * 2;
      }
      (this.ash.geometry.getAttribute('position') as THREE.BufferAttribute).needsUpdate = true;
    }
    // Flashlight: from the eye (or just under the camera in third person), along the view.
    if (w.flashlight && eye) {
      this.flashlight.intensity = 55;
      this.flashlight.position.copy(eye);
      if (!fpv) this.flashlight.position.y -= 0.1;
      this.flashlight.target.position.copy(eye).addScaledVector(look, 10);
    } else this.flashlight.intensity = 0;
    // Lightning: a double flicker every 6-16 s in the rain.
    let bolt = 0;
    if (w.lightning) {
      this.nextBolt -= dt;
      if (this.nextBolt <= 0) { this.lightning = 0.5; this.nextBolt = 6 + Math.random() * 10; }
      if (this.lightning > 0) {
        this.lightning -= dt;
        const t = 0.5 - this.lightning;
        bolt = t < 0.08 ? 1 : t < 0.16 ? 0.2 : t < 0.24 ? 0.8 : Math.max(0, 1 - (t - 0.24) * 4) * 0.5;
      }
      this.view.hemi.intensity = (w.theme.ambient ?? this.base.ambient) + bolt * 2.2;
    }
    return bolt;
  }

  dispose(): void { for (const r of this.resources) r.dispose(); this.beams.dispose(); }
}
