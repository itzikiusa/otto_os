/**
 * Pooled GPU particles (one draw call): sparks, dust, smoke, explosions.
 * Fixed capacity; spawning when full recycles the oldest slot. No per-frame
 * allocation.
 */
import * as THREE from 'three';

const ATTRIBUTES = ['position', 'color', 'size', 'alpha'];

export class Particles {
  readonly points: THREE.Points;
  private pos: Float32Array; private vel: Float32Array; private col: Float32Array;
  private life: Float32Array; private maxLife: Float32Array; private size: Float32Array; private drag: Float32Array; private grav: Float32Array;
  private alpha: Float32Array;
  private next = 0;
  private geometry: THREE.BufferGeometry;
  private material: THREE.ShaderMaterial;
  private capacity: number;
  private _c = new THREE.Color();

  constructor(capacity = 1500, additive = true) {
    this.capacity = capacity;
    this.pos = new Float32Array(capacity * 3); this.vel = new Float32Array(capacity * 3); this.col = new Float32Array(capacity * 3);
    this.life = new Float32Array(capacity); this.maxLife = new Float32Array(capacity); this.size = new Float32Array(capacity);
    this.drag = new Float32Array(capacity); this.grav = new Float32Array(capacity); this.alpha = new Float32Array(capacity);
    this.geometry = new THREE.BufferGeometry();
    this.geometry.setAttribute('position', new THREE.BufferAttribute(this.pos, 3).setUsage(THREE.DynamicDrawUsage));
    this.geometry.setAttribute('color', new THREE.BufferAttribute(this.col, 3).setUsage(THREE.DynamicDrawUsage));
    this.geometry.setAttribute('size', new THREE.BufferAttribute(this.size, 1).setUsage(THREE.DynamicDrawUsage));
    this.geometry.setAttribute('alpha', new THREE.BufferAttribute(this.alpha, 1).setUsage(THREE.DynamicDrawUsage));
    this.material = new THREE.ShaderMaterial({
      transparent: true, depthWrite: false, blending: additive ? THREE.AdditiveBlending : THREE.NormalBlending,
      uniforms: { scale: { value: 600 } },
      vertexShader: `attribute float size; attribute float alpha; varying vec3 vColor; varying float vAlpha; uniform float scale;
        void main(){ vColor = color; vAlpha = alpha; vec4 mv = modelViewMatrix * vec4(position, 1.0); gl_PointSize = size * scale / -mv.z; gl_Position = projectionMatrix * mv; }`,
      fragmentShader: `varying vec3 vColor; varying float vAlpha;
        void main(){ vec2 d = gl_PointCoord - 0.5; float r = dot(d, d); if (r > 0.25) discard; gl_FragColor = vec4(vColor, vAlpha * (1.0 - r * 4.0)); }`,
      vertexColors: true,
    });
    this.points = new THREE.Points(this.geometry, this.material);
    this.points.frustumCulled = false;
    this.points.renderOrder = 5;
  }

  spawn(x: number, y: number, z: number, vx: number, vy: number, vz: number, color: number, size: number, life: number, gravity = 0, drag = 1): void {
    const i = this.next;
    this.next = (this.next + 1) % this.capacity;
    this.pos[i * 3] = x; this.pos[i * 3 + 1] = y; this.pos[i * 3 + 2] = z;
    this.vel[i * 3] = vx; this.vel[i * 3 + 1] = vy; this.vel[i * 3 + 2] = vz;
    this._c.setHex(color);
    this.col[i * 3] = this._c.r; this.col[i * 3 + 1] = this._c.g; this.col[i * 3 + 2] = this._c.b;
    this.life[i] = life; this.maxLife[i] = life; this.size[i] = size; this.grav[i] = gravity; this.drag[i] = drag; this.alpha[i] = 1;
  }

  /** Radial burst. */
  burst(x: number, y: number, z: number, count: number, color: number, speed: number, size: number, life: number, gravity = -6): void {
    for (let k = 0; k < count; k++) {
      const a = Math.random() * Math.PI * 2, u = Math.random() * 2 - 1, s = speed * (0.4 + Math.random() * 0.6);
      const h = Math.sqrt(1 - u * u);
      this.spawn(x, y, z, Math.cos(a) * h * s, Math.abs(u) * s, Math.sin(a) * h * s, color, size * (0.6 + Math.random() * 0.6), life * (0.6 + Math.random() * 0.5), gravity, 1.5);
    }
  }

  update(dt: number): void {
    for (let i = 0; i < this.capacity; i++) {
      if (this.life[i] <= 0) { if (this.alpha[i] !== 0) { this.alpha[i] = 0; } continue; }
      this.life[i] -= dt;
      const k = Math.exp(-this.drag[i] * dt);
      this.vel[i * 3] *= k; this.vel[i * 3 + 2] *= k;
      this.vel[i * 3 + 1] = this.vel[i * 3 + 1] * k + this.grav[i] * dt;
      this.pos[i * 3] += this.vel[i * 3] * dt; this.pos[i * 3 + 1] += this.vel[i * 3 + 1] * dt; this.pos[i * 3 + 2] += this.vel[i * 3 + 2] * dt;
      this.alpha[i] = Math.max(0, this.life[i] / this.maxLife[i]);
    }
    for (const name of ATTRIBUTES) (this.geometry.getAttribute(name) as THREE.BufferAttribute).needsUpdate = true;
  }

  dispose(): void { this.geometry.dispose(); this.material.dispose(); }
}
