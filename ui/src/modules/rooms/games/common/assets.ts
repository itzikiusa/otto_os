/**
 * GPU asset library for one game session: loads the meshopt-compressed Kenney
 * kits from /room-games, exposes every kit model by key, batches static props
 * into InstancedMeshes and clones animated characters. `dispose()` frees every
 * geometry, material and texture it created (the HTTP cache keeps the bytes).
 */
import * as THREE from 'three';
import { GLTFLoader, type GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { MeshoptDecoder } from 'three/examples/jsm/libs/meshopt_decoder.module.js';
import { clone as cloneSkinned } from 'three/examples/jsm/utils/SkeletonUtils.js';

export const ASSET_ROOT = '/room-games';
export interface ModelPart { geometry: THREE.BufferGeometry; material: THREE.Material; matrix: THREE.Matrix4 }
export interface KitModel { key: string; root: THREE.Object3D; parts: ModelPart[]; box: THREE.Box3 }

const _m = new THREE.Matrix4();

export class AssetLibrary {
  private loader = new GLTFLoader();
  private models = new Map<string, KitModel>();
  private characters = new Map<string, THREE.Object3D>();
  private owned = new Set<{ dispose(): void }>();
  clips: THREE.AnimationClip[] = [];
  private signal: AbortSignal;

  constructor(signal: AbortSignal) {
    this.signal = signal;
    this.loader.setMeshoptDecoder(MeshoptDecoder);
  }

  private async load(file: string): Promise<GLTF> {
    if (this.signal.aborted) throw new DOMException('Aborted', 'AbortError');
    const gltf = await this.loader.loadAsync(`${ASSET_ROOT}/${file}`);
    gltf.scene.traverse(o => {
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh) return;
      this.owned.add(mesh.geometry);
      for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) {
        this.owned.add(m);
        const std = m as THREE.MeshStandardMaterial;
        if (std.map) { this.owned.add(std.map); std.map.anisotropy = 4; }
        // Kenney palettes are flat color swatches; matte reads best in-game.
        if ('roughness' in std) { std.roughness = Math.max(std.roughness, 0.8); std.metalness = Math.min(std.metalness, 0.1); }
      }
    });
    return gltf;
  }

  /** Loads a kit; every top-level node becomes a model keyed by its name. */
  async loadKit(file: string): Promise<void> {
    const gltf = await this.load(file);
    gltf.scene.updateMatrixWorld(true);
    for (const root of [...gltf.scene.children]) {
      const parts: ModelPart[] = [];
      const inverse = new THREE.Matrix4().copy(root.matrixWorld).invert();
      root.traverse(o => {
        const mesh = o as THREE.Mesh;
        if (!mesh.isMesh) return;
        const matrix = new THREE.Matrix4().multiplyMatrices(inverse, mesh.matrixWorld);
        parts.push({ geometry: mesh.geometry, material: mesh.material as THREE.Material, matrix });
      });
      const box = new THREE.Box3();
      for (const p of parts) {
        p.geometry.computeBoundingBox();
        box.union(p.geometry.boundingBox!.clone().applyMatrix4(p.matrix));
      }
      root.removeFromParent();
      root.position.set(0, 0, 0); root.rotation.set(0, 0, 0); root.scale.set(1, 1, 1);
      this.models.set(root.name, { key: root.name, root, parts, box });
    }
  }

  model(key: string): KitModel {
    const model = this.models.get(key);
    if (!model) throw new Error(`Missing game model ${key}`);
    return model;
  }
  has(key: string): boolean { return this.models.has(key); }

  /** A deep clone (shared geometry/materials) for animated or individually moved models. */
  instance(key: string): THREE.Object3D {
    const copy = this.model(key).root.clone(true);
    copy.traverse(o => { if ((o as THREE.Mesh).isMesh) { o.castShadow = true; o.receiveShadow = true; } });
    return copy;
  }

  async loadCharacters(ids: string[]): Promise<void> {
    const wanted = [...new Set(ids)].filter(id => !this.characters.has(id));
    const [anims, ...chars] = await Promise.all([
      this.clips.length ? Promise.resolve(null) : this.load('char-anims.glb'),
      ...wanted.map(id => this.load(`char-${id}.glb`)),
    ]);
    if (anims) this.clips = anims.animations;
    wanted.forEach((id, i) => this.characters.set(id, chars[i].scene));
  }

  /** Skinned clone of a mini character; animate with the shared clips by bone name. */
  character(id: string): THREE.Object3D {
    const source = this.characters.get(id);
    if (!source) throw new Error(`Missing character ${id}`);
    const copy = cloneSkinned(source);
    copy.traverse(o => {
      if ((o as THREE.Mesh).isMesh) { o.castShadow = true; o.frustumCulled = false; }
    });
    return copy;
  }

  clip(name: string): THREE.AnimationClip {
    const clip = this.clips.find(c => c.name === name);
    if (!clip) throw new Error(`Missing animation ${name}`);
    return clip;
  }

  /** Registers runtime-created GPU resources so dispose() frees them too. */
  own<T extends { dispose(): void }>(resource: T): T { this.owned.add(resource); return resource; }

  dispose(): void {
    for (const r of this.owned) r.dispose();
    this.owned.clear(); this.models.clear(); this.characters.clear(); this.clips = [];
  }
}

/** Collects static prop placements, then emits one InstancedMesh per model part. */
export class PropBatch {
  private placements = new Map<string, THREE.Matrix4[]>();
  private lib: AssetLibrary;
  constructor(lib: AssetLibrary) { this.lib = lib; }
  /** Places a model with its footprint centred on (x, z) and its base at y. */
  add(key: string, x: number, y: number, z: number, yaw = 0, scale = 1, sy = scale): void {
    const box = this.lib.model(key).box;
    const cx = (box.min.x + box.max.x) / 2, cz = (box.min.z + box.max.z) / 2;
    const c = Math.cos(yaw), s = Math.sin(yaw);
    const m = new THREE.Matrix4().compose(
      new THREE.Vector3(x - (cx * c + cz * s) * scale, y - box.min.y * sy, z - (-cx * s + cz * c) * scale),
      new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), yaw),
      new THREE.Vector3(scale, sy, scale));
    const list = this.placements.get(key) ?? [];
    list.push(m);
    this.placements.set(key, list);
  }
  count(): number { let n = 0; for (const l of this.placements.values()) n += l.length; return n; }
  /**
   * Small decoration (grass, flowers…) never casts shadows: it is cheap fill,
   * not shape. `material` may swap a part's material (e.g. a charred copy).
   */
  build(group: THREE.Group, shadows: boolean, decor: RegExp = /grass|flower|bush|plant_|stones|mushroom/, material?: (m: THREE.Material) => THREE.Material): void {
    for (const [key, list] of this.placements) {
      const model = this.lib.model(key);
      for (const part of model.parts) {
        const mesh = new THREE.InstancedMesh(part.geometry, material ? material(part.material) : part.material, list.length);
        list.forEach((m, i) => mesh.setMatrixAt(i, _m.multiplyMatrices(m, part.matrix)));
        mesh.instanceMatrix.needsUpdate = true;
        mesh.computeBoundingSphere();
        mesh.castShadow = shadows && !decor.test(key); mesh.receiveShadow = true;
        mesh.name = key;
        group.add(mesh);
      }
    }
    this.placements.clear();
  }
}
