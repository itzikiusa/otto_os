/**
 * Shared three.js stage: renderer, sky, sun with a camera-following shadow
 * frustum, fog, resize handling and render stats. One per mounted game.
 */
import * as THREE from 'three';

export interface Theme {
  skyTop: number; skyHorizon: number; fog: number; fogNear: number; fogFar: number;
  sun: number; sunIntensity: number; ambientSky: number; ambientGround: number; ambient: number;
}
export interface RenderStats { frames: number; frameMs: number; fps: number; drawCalls: number; triangles: number }

export class GameView {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene = new THREE.Scene();
  readonly camera = new THREE.PerspectiveCamera(65, 1, 0.1, 1400);
  readonly sun: THREE.DirectionalLight;
  readonly canvas: HTMLCanvasElement;
  private resizeObserver: ResizeObserver;
  private sky: THREE.Mesh;
  private sunOffset = new THREE.Vector3(60, 110, 40);
  private frames = 0; private frameMs = 16; private fps = 60; private last = 0;
  private host: HTMLElement;

  constructor(host: HTMLElement, theme: Theme, shadowSize = 2048) {
    this.host = host;
    this.renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: 'high-performance' });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.75));
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
    this.renderer.toneMappingExposure = 1.05;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = THREE.PCFShadowMap;
    this.canvas = this.renderer.domElement;
    this.canvas.tabIndex = 0;
    this.canvas.setAttribute('aria-label', 'Game view');
    host.appendChild(this.canvas);

    this.scene.fog = new THREE.Fog(theme.fog, theme.fogNear, theme.fogFar);
    this.scene.background = new THREE.Color(theme.skyHorizon);
    const hemi = new THREE.HemisphereLight(theme.ambientSky, theme.ambientGround, theme.ambient);
    this.scene.add(hemi);
    this.sun = new THREE.DirectionalLight(theme.sun, theme.sunIntensity);
    this.sun.castShadow = true;
    this.sun.shadow.mapSize.set(shadowSize, shadowSize);
    const s = this.sun.shadow.camera;
    s.left = -55; s.right = 55; s.top = 55; s.bottom = -55; s.near = 10; s.far = 320;
    this.sun.shadow.bias = -0.0006;
    this.sun.shadow.normalBias = 0.04;
    this.scene.add(this.sun, this.sun.target);

    const skyMat = new THREE.ShaderMaterial({
      side: THREE.BackSide, depthWrite: false, fog: false,
      uniforms: { top: { value: new THREE.Color(theme.skyTop) }, horizon: { value: new THREE.Color(theme.skyHorizon) } },
      vertexShader: 'varying vec3 vDir; void main(){ vDir = normalize(position); vec4 p = modelViewMatrix * vec4(position,1.0); gl_Position = projectionMatrix * p; gl_Position.z = gl_Position.w; }',
      fragmentShader: 'uniform vec3 top; uniform vec3 horizon; varying vec3 vDir; void main(){ float h = clamp(vDir.y * 1.6, 0.0, 1.0); gl_FragColor = vec4(mix(horizon, top, pow(h, 0.7)), 1.0); }',
    });
    this.sky = new THREE.Mesh(new THREE.SphereGeometry(1000, 24, 12), skyMat);
    this.sky.frustumCulled = false;
    this.sky.renderOrder = -1;
    this.scene.add(this.sky);

    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(host);
    this.resize();
  }

  resize(): void {
    const w = Math.max(1, this.host.clientWidth), h = Math.max(1, this.host.clientHeight);
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  /** Keeps the shadow frustum centred on the action (texel-snapped to avoid shimmer). */
  focusShadow(x: number, y: number, z: number): void {
    const texel = 110 / this.sun.shadow.mapSize.x;
    const sx = Math.round(x / texel) * texel, sz = Math.round(z / texel) * texel;
    this.sun.position.set(sx + this.sunOffset.x, y + this.sunOffset.y, sz + this.sunOffset.z);
    this.sun.target.position.set(sx, y, sz);
  }

  render(): void {
    const now = performance.now();
    if (this.last) {
      const dt = now - this.last;
      this.frameMs = this.frameMs * 0.92 + dt * 0.08;
      this.fps = 1000 / this.frameMs;
    }
    this.last = now;
    this.sky.position.copy(this.camera.position);
    this.renderer.render(this.scene, this.camera);
    this.frames++;
  }

  stats(): RenderStats {
    const info = this.renderer.info.render;
    return { frames: this.frames, frameMs: this.frameMs, fps: this.fps, drawCalls: info.calls, triangles: info.triangles };
  }

  dispose(): void {
    this.resizeObserver.disconnect();
    this.scene.traverse(o => {
      const mesh = o as THREE.Mesh;
      // Kit resources are freed by AssetLibrary; free only what the scene built itself.
      if (mesh.userData.owned) {
        mesh.geometry?.dispose();
        for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) m?.dispose();
      }
    });
    (this.sky.material as THREE.Material).dispose();
    this.sky.geometry.dispose();
    this.sun.shadow.map?.dispose();
    this.renderer.dispose();
    this.renderer.forceContextLoss();
    this.canvas.remove();
  }
}

/** Marks a runtime-built mesh so GameView.dispose() frees its geometry/material. */
export function owned<T extends THREE.Object3D>(object: T): T {
  object.traverse(o => { o.userData.owned = true; });
  return object;
}
