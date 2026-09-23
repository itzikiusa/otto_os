<script lang="ts">
  // The game-studio viewport: three.js (lazy-loaded — ~650 kB, only when a scene3d
  // artifact opens), orbit camera, grid + axes, PBR lighting from the doc, shadows,
  // click-to-select and a TransformControls gizmo (W/E/R, F frame, Del, ⌘D, Esc).
  //
  // Data flow is one-way: `doc` is the truth (validated JSON the agent also edits),
  // this component RECONCILES the three scene to it by id on every change, and every
  // user edit goes through `ops.ts` → `onchange(newDoc)` (undebounced — the arena
  // owns the 600 ms autosave + dirty/conflict state). We never mutate `doc`.
  //
  // scene3d v2 (3D Studio 1.5): physical materials (presets + brand `token:`
  // colours resolved through `colors`), procedural environments (IBL + gradient
  // backdrop), named STATES — `stateId` picks one and the viewer tweens every
  // object's pose to it (gizmo edits made in a non-default state are written to
  // that state's overrides) — a turntable orbit, a view cube, and camera moves
  // to named views. Motion respects `prefers-reduced-motion` (instant tweens).
  //
  // gltf objects load through `resolveAttachment(ref)` → blob URL, where `ref` is
  // the object's `attachment_id` or its v2 `otto://design/…` src (the host fetches
  // with auth); there is no URL surface here. Meshopt-compressed GLBs (Optimize
  // for web) decode with three's bundled decoder.
  import { onDestroy } from 'svelte';
  import type * as THREE_NS from 'three';
  import type { OrbitControls as OrbitControlsT } from 'three/examples/jsm/controls/OrbitControls.js';
  import type { TransformControls as TransformControlsT } from 'three/examples/jsm/controls/TransformControls.js';
  import type { GLTFLoader as GLTFLoaderT } from 'three/examples/jsm/loaders/GLTFLoader.js';
  import type { Easing, GizmoMode, Scene3dCamera, Scene3dDoc, Scene3dObject, Vec3 } from './types';
  import {
    applyTransform,
    buildLight,
    buildMesh,
    disposeTree,
    geometryKey,
    loadThree,
    RAD,
    DEG,
    updateLight,
    updateMeshMaterial,
    USERDATA_ID,
    USERDATA_KIND,
    plainColors,
    type ColorResolver,
    type Three,
  } from './build';
  import { duplicate, findNode, gizmoModeForKey, remove, setCamera, setStateOverride, setTransform, summarize } from './ops';
  import { resolveMaterial, type ResolvedMaterial } from './presets';
  import { editTargetState, findState, initialState, lerpPoses, stateTiming, statePoses, transitionProgress, type Pose } from './states';
  import { buildEnvironment, environmentKey, type BuiltEnvironment } from './environment';

  interface Props {
    doc: Scene3dDoc;
    readonly?: boolean;
    selectedId?: string | null;
    /** Presentation view: hides gizmo/grid/helpers and looks through the doc camera. */
    play?: boolean;
    onchange: (doc: Scene3dDoc) => void;
    resolveAttachment: (ref: string) => Promise<string>;
    /** v2: colour resolver binding `token:color.<name>` to the host's brand kit
     *  (Design Hall passes Brand Kit's `resolveToken`); default: hex only. */
    colors?: ColorResolver;
    /** v2: the active state (null / unknown = the document's initial state). */
    stateId?: string | null;
    /** v2: orbit the camera slowly around its target (speed from `doc.turntable`). */
    turntable?: boolean;
    /** Show the view cube (top-right). */
    viewCube?: boolean;
    /** Extra text for the stats pill ("saved", "edited"…). */
    statusNote?: string;
    /** Hide the W/E/R key hint line (hosts with their own chrome). */
    compact?: boolean;
  }
  let {
    doc,
    readonly = false,
    selectedId = $bindable<string | null>(null),
    play = false,
    onchange,
    resolveAttachment,
    colors = plainColors,
    stateId = null,
    turntable = false,
    viewCube = false,
    statusNote = '',
    compact = false,
  }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);
  let cubeEl = $state<HTMLDivElement | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let mode = $state<GizmoMode>('translate');
  let hovering = $state(false);
  let gltfErrors = $state<Record<string, string>>({});
  const status = $derived(summarize(doc));
  const canEdit = $derived(!readonly && !play);
  const colorFn: ColorResolver = (ref, fallback) => colors(ref, fallback);

  // ── three state (plain lets, never reactive — these are heavy mutable objects) ──
  let THREE: Three | null = null;
  let renderer: THREE_NS.WebGLRenderer | null = null;
  let scene: THREE_NS.Scene | null = null;
  let camera: THREE_NS.PerspectiveCamera | null = null;
  let orbit: OrbitControlsT | null = null;
  let gizmo: TransformControlsT | null = null;
  let gizmoHelper: THREE_NS.Object3D | null = null;
  let content: THREE_NS.Group | null = null; // doc objects + groups (exported as-is)
  let lightsRoot: THREE_NS.Group | null = null; // doc lights + their targets
  let helpers: THREE_NS.Group | null = null; // grid, axes, light markers, selection box
  let grid: THREE_NS.GridHelper | null = null;
  let axes: THREE_NS.AxesHelper | null = null;
  let selectionBox: THREE_NS.BoxHelper | null = null;
  let loader: GLTFLoaderT | null = null;
  let env: BuiltEnvironment | null = null;
  let envKey = '';
  let ro: ResizeObserver | null = null;
  let raf = 0;
  let dirty = true;
  let destroyed = false;
  let dragging = false;
  let cameraInitialised = false;
  let lastFrame = 0;
  const nodes = new Map<string, THREE_NS.Object3D>(); // id → object/group/light node
  const nodeType = new Map<string, string>(); // id → geometry key ('box' | 'box:r0.04' | 'gltf:<ref>' | 'group' | 'directional' …)
  const lightTargets = new Map<string, THREE_NS.Object3D>();
  const lightMarkers = new Map<string, THREE_NS.Mesh>();
  const gltfCache = new Map<string, Promise<THREE_NS.Group>>(); // ref → template scene
  const baseMaterial = new Map<string, ResolvedMaterial>(); // object id → resolved doc material (pose overrides restore to it)
  let pointerDown: { x: number; y: number } | null = null;

  // v2 states: the pose currently on screen, and an in-flight tween.
  let shownState: string | null = null;
  let poses: Map<string, Pose> | null = null;
  let tween: { from: Map<string, Pose>; to: Map<string, Pose>; start: number; duration: number; easing: Easing } | null = null;
  // Camera move to a named view.
  let camTween: { fromPos: THREE_NS.Vector3; fromTarget: THREE_NS.Vector3; toPos: THREE_NS.Vector3; toTarget: THREE_NS.Vector3; fromFov: number; toFov: number; start: number; duration: number } | null = null;

  const invalidate = () => {
    dirty = true;
  };
  const reducedMotion = (): boolean =>
    typeof window !== 'undefined' && !!window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  /** The state the viewer shows: the prop, else the document's initial state. */
  function effectiveState(d: Scene3dDoc): string | null {
    if (stateId && findState(d, stateId)) return stateId;
    return initialState(d);
  }

  // ── bootstrap ────────────────────────────────────────────────────────────────
  async function boot(el: HTMLDivElement): Promise<void> {
    try {
      const [three, orbitMod, tcMod, gltfMod, meshopt] = await Promise.all([
        loadThree(),
        import('three/examples/jsm/controls/OrbitControls.js'),
        import('three/examples/jsm/controls/TransformControls.js'),
        import('three/examples/jsm/loaders/GLTFLoader.js'),
        import('three/examples/jsm/libs/meshopt_decoder.module.js'),
      ]);
      if (destroyed) return;
      THREE = three;
      const T = THREE;

      renderer = new T.WebGLRenderer({ antialias: true, alpha: false, preserveDrawingBuffer: true });
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.shadowMap.enabled = true;
      renderer.shadowMap.type = T.PCFSoftShadowMap;
      renderer.outputColorSpace = T.SRGBColorSpace;
      renderer.toneMapping = T.ACESFilmicToneMapping;
      renderer.toneMappingExposure = 1;
      renderer.domElement.style.display = 'block';
      renderer.domElement.style.width = '100%';
      renderer.domElement.style.height = '100%';
      renderer.domElement.style.touchAction = 'none';
      el.appendChild(renderer.domElement);

      scene = new T.Scene();
      content = new T.Group();
      content.name = 'content';
      lightsRoot = new T.Group();
      lightsRoot.name = 'lights';
      helpers = new T.Group();
      helpers.name = 'helpers';
      scene.add(content, lightsRoot, helpers);

      camera = new T.PerspectiveCamera(50, 1, 0.1, 500);
      applyDocCamera();

      orbit = new orbitMod.OrbitControls(camera, renderer.domElement);
      orbit.enableDamping = false;
      orbit.screenSpacePanning = true;
      orbit.maxPolarAngle = Math.PI * 0.999;
      orbit.addEventListener('change', invalidate);
      orbit.addEventListener('start', () => (camTween = null));

      gizmo = new tcMod.TransformControls(camera, renderer.domElement);
      gizmo.setMode(mode);
      gizmo.addEventListener('change', invalidate);
      gizmo.addEventListener('dragging-changed', (e) => {
        dragging = Boolean((e as unknown as { value: boolean }).value);
        if (orbit) orbit.enabled = !dragging;
        if (!dragging) commitGizmo(); // final, rounded write at drag end
      });
      gizmo.addEventListener('objectChange', () => commitGizmo());
      gizmoHelper = gizmo.getHelper();
      scene.add(gizmoHelper);

      grid = new T.GridHelper(40, 40);
      axes = new T.AxesHelper(1.5);
      grid.renderOrder = -1;
      helpers.add(grid, axes);

      loader = new gltfMod.GLTFLoader();
      loader.setMeshoptDecoder(meshopt.MeshoptDecoder);

      ro = new ResizeObserver(() => resize());
      ro.observe(el);
      resize();
      reconcile(doc);
      applyEnvironment();
      applyPlay();
      loading = false;
      loop(performance.now());
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
      loading = false;
    }
  }

  function resize(): void {
    if (!host || !renderer || !camera) return;
    const w = Math.max(1, host.clientWidth);
    const h = Math.max(1, host.clientHeight);
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    invalidate();
  }

  function loop(now: number): void {
    if (destroyed) return;
    raf = requestAnimationFrame(loop);
    const dt = lastFrame ? Math.min(0.1, (now - lastFrame) / 1000) : 0;
    lastFrame = now;
    stepTween(now);
    stepCamera(now);
    if (turntable && !dragging && camera && orbit && !camTween) {
      const speed = doc.turntable?.speed ?? 20;
      const offset = camera.position.clone().sub(orbit.target);
      offset.applyAxisAngle(new THREE!.Vector3(0, 1, 0), speed * DEG * dt);
      camera.position.copy(orbit.target).add(offset);
      camera.lookAt(orbit.target);
      dirty = true;
    }
    if (!dirty || !renderer || !scene || !camera) return;
    dirty = false;
    if (selectionBox) selectionBox.update();
    renderer.render(scene, camera);
    syncCube();
  }

  // ── doc → three reconciliation ───────────────────────────────────────────────
  function reconcile(d: Scene3dDoc): void {
    if (!THREE || !content || !lightsRoot || !helpers) return;
    const T = THREE;
    const live = new Set<string>();

    // Groups first so children can be parented under them.
    for (const g of d.groups) {
      live.add(g.id);
      let node = nodes.get(g.id);
      if (!node || nodeType.get(g.id) !== 'group') {
        if (node) dropNode(g.id);
        node = new T.Group();
        node.name = g.name;
        node.userData[USERDATA_ID] = g.id;
        node.userData[USERDATA_KIND] = 'group';
        nodes.set(g.id, node);
        nodeType.set(g.id, 'group');
        content.add(node);
      }
      node.name = g.name;
      node.visible = g.visible !== false;
    }
    // Objects.
    for (const o of d.objects) {
      live.add(o.id);
      let node = nodes.get(o.id);
      const typeKey = geometryKey(o);
      if (!node || nodeType.get(o.id) !== typeKey) {
        if (node) dropNode(o.id);
        node = o.type === 'gltf' ? spawnGltf(o) : buildMesh(T, o, colorFn);
        nodes.set(o.id, node);
        nodeType.set(o.id, typeKey);
        content.add(node);
      } else {
        applyTransform(node, o);
        node.name = o.name;
        node.visible = o.visible !== false;
        if (o.type !== 'gltf') updateMeshMaterial(T, node as THREE_NS.Mesh, o, colorFn);
        if (o.type === 'text') node.userData.text = o.text;
      }
      if (o.type !== 'gltf') baseMaterial.set(o.id, resolveMaterial(o.material, colorFn));
    }
    // Lights (+ a pickable marker in edit mode).
    for (const l of d.lights) {
      live.add(l.id);
      let node = nodes.get(l.id);
      if (node && nodeType.get(l.id) !== l.type) {
        dropNode(l.id);
        node = undefined;
      }
      if (node) {
        // Same type: patch in place so a gizmo attached to this light survives the drag.
        updateLight(T, node as THREE_NS.Light, lightTargets.get(l.id), l);
        const marker = lightMarkers.get(l.id);
        if (marker) {
          marker.position.copy(node.position);
          (marker.material as THREE_NS.MeshBasicMaterial).color.set(l.color ?? '#ffffff');
          marker.visible = !play && l.visible !== false;
        }
      } else {
        const { light, target } = buildLight(T, l);
        nodes.set(l.id, light);
        nodeType.set(l.id, l.type);
        lightsRoot.add(light);
        if (target) {
          lightsRoot.add(target);
          lightTargets.set(l.id, target);
        }
        if (l.type !== 'ambient' && l.type !== 'hemisphere') {
          const marker = new T.Mesh(
            new T.SphereGeometry(0.12, 12, 8),
            new T.MeshBasicMaterial({ color: l.color ?? '#ffffff', wireframe: true }),
          );
          marker.position.copy(light.position);
          marker.userData[USERDATA_ID] = l.id;
          marker.userData[USERDATA_KIND] = 'light-marker';
          marker.visible = !play && l.visible !== false;
          helpers.add(marker);
          lightMarkers.set(l.id, marker);
        }
      }
    }
    // Remove what left the document.
    for (const id of [...nodes.keys()]) if (!live.has(id)) dropNode(id);
    for (const id of [...baseMaterial.keys()]) if (!live.has(id)) baseMaterial.delete(id);

    // Parenting: groups are organisational (identity transform), so local == world.
    const parentOf = new Map<string, string>();
    for (const g of d.groups) for (const c of g.children) parentOf.set(c, g.id);
    for (const [id, node] of nodes) {
      if (nodeType.get(id) && findNode(d, id)?.kind === 'light') continue;
      const pid = parentOf.get(id);
      const want = (pid && nodes.get(pid)) || content;
      if (node.parent !== want) want.add(node); // Object3D.add re-parents
    }

    // v2 states: re-pose on top of the base transforms (unless a tween owns the pose).
    const want = effectiveState(d);
    if (tween && want === shownState) {
      tween.to = statePoses(d, want, (c) => colorFn(c, '#000000'));
    } else if (want !== shownState && shownState !== null && cameraInitialised) {
      startStateTween(d, want);
    } else {
      shownState = want;
      poses = statePoses(d, want, (c) => colorFn(c, '#000000'));
      applyPoses(poses);
    }

    // Selection may have vanished (agent deleted it, or we did).
    if (selectedId && !live.has(selectedId)) selectedId = null;
    syncSelection();
    invalidate();
  }

  /** Put every object at its pose (transform, visibility, colour/opacity overrides). */
  function applyPoses(p: Map<string, Pose>): void {
    for (const [id, pose] of p) {
      const node = nodes.get(id);
      if (!node) continue;
      node.position.set(pose.position[0], pose.position[1], pose.position[2]);
      node.rotation.set(pose.rotation[0] * DEG, pose.rotation[1] * DEG, pose.rotation[2] * DEG);
      node.scale.set(pose.scale[0] || 1e-4, pose.scale[1] || 1e-4, pose.scale[2] || 1e-4);
      node.visible = pose.visible;
      const base = baseMaterial.get(id);
      const mat = (node as THREE_NS.Mesh).material as THREE_NS.MeshPhysicalMaterial | undefined;
      if (base && mat && 'clearcoat' in mat && !mat.map) {
        mat.color.set(pose.color ?? base.color);
        mat.emissive.set(pose.emissive ?? base.emissive);
        const op = pose.opacity ?? base.opacity;
        if (mat.opacity !== op || mat.transparent !== op < 1) {
          mat.opacity = op;
          const t = op < 1;
          if (mat.transparent !== t) {
            mat.transparent = t;
            mat.needsUpdate = true;
          }
        }
      }
    }
    invalidate();
  }

  function startStateTween(d: Scene3dDoc, to: string | null): void {
    const from = poses ?? statePoses(d, shownState, (c) => colorFn(c, '#000000'));
    const target = statePoses(d, to, (c) => colorFn(c, '#000000'));
    const { duration, easing } = stateTiming(findState(d, to), reducedMotion());
    shownState = to;
    if (duration <= 0) {
      tween = null;
      poses = target;
      applyPoses(target);
      return;
    }
    tween = { from, to: target, start: performance.now(), duration, easing };
    syncSelection(); // the gizmo stays detached while a tween owns the pose
  }

  function stepTween(now: number): void {
    if (!tween) return;
    const t = transitionProgress(now - tween.start, tween.duration, tween.easing);
    const done = now - tween.start >= tween.duration;
    poses = done ? tween.to : lerpPoses(tween.from, tween.to, t);
    applyPoses(poses);
    if (done) {
      tween = null;
      syncSelection();
    }
  }

  function dropNode(id: string): void {
    const node = nodes.get(id);
    if (node) {
      if (gizmo && gizmo.object === node) gizmo.detach();
      node.removeFromParent();
      disposeTree(node);
    }
    nodes.delete(id);
    nodeType.delete(id);
    const t = lightTargets.get(id);
    if (t) {
      t.removeFromParent();
      lightTargets.delete(id);
    }
    const m = lightMarkers.get(id);
    if (m) {
      m.removeFromParent();
      disposeTree(m);
      lightMarkers.delete(id);
    }
  }

  /** A holder group that receives the loaded model asynchronously (placeholder until then). */
  function spawnGltf(o: Scene3dObject): THREE_NS.Object3D {
    const T = THREE!;
    const holder = new T.Group();
    holder.name = o.name;
    holder.userData[USERDATA_ID] = o.id;
    holder.userData[USERDATA_KIND] = 'object';
    applyTransform(holder, o);
    holder.visible = o.visible !== false;
    const placeholder = new T.Mesh(
      new T.BoxGeometry(1, 1, 1),
      new T.MeshBasicMaterial({ color: '#64748b', wireframe: true }),
    );
    placeholder.position.y = 0.5;
    placeholder.name = 'placeholder';
    holder.add(placeholder);
    const ref = (o.src ?? o.attachment_id)!;
    let p = gltfCache.get(ref);
    if (!p) {
      p = (async () => {
        const url = await resolveAttachment(ref);
        const gltf = await loader!.loadAsync(url);
        gltf.scene.traverse((n) => {
          const m = n as THREE_NS.Mesh;
          if (m.isMesh) {
            m.castShadow = true;
            m.receiveShadow = true;
          }
        });
        return gltf.scene;
      })();
      gltfCache.set(ref, p);
    }
    p.then(
      (template) => {
        if (destroyed || !holder.parent) return;
        holder.remove(placeholder);
        disposeTree(placeholder);
        holder.add(template.clone(true)); // materials/geometry shared with the template — disposed once with the cache
        delete gltfErrors[ref];
        gltfErrors = { ...gltfErrors };
        syncSelection();
        invalidate();
      },
      (err: unknown) => {
        if (destroyed) return;
        gltfCache.delete(ref);
        (placeholder.material as THREE_NS.MeshBasicMaterial).color.set('#ef4444');
        gltfErrors = { ...gltfErrors, [ref]: err instanceof Error ? err.message : String(err) };
        invalidate();
      },
    );
    return holder;
  }

  let lastBg = '';
  function applyEnvironment(): void {
    if (!THREE || !scene || !grid || !axes || !renderer) return;
    const e = doc.environment;
    // v2 environment: prefiltered IBL + (optionally) its gradient backdrop.
    const key = environmentKey(e);
    if (key !== envKey) {
      envKey = key;
      env?.dispose();
      env = e ? buildEnvironment(THREE, renderer, e) : null;
      scene.environment = env?.envMap ?? null;
      renderer.toneMappingExposure = env?.preset.exposure ?? 1;
      lastBg = ''; // backdrop may change with the preset
    }
    scene.environmentIntensity = e?.intensity ?? 1;
    const useBackdrop = !!(e && e.background !== false && env?.backdrop);
    const bg = useBackdrop ? `env:${envKey}` : (doc.background ?? '#0f172a');
    if (bg !== lastBg) {
      lastBg = bg;
      const lumOf = useBackdrop ? env!.preset.backdrop[1] : bg;
      if (useBackdrop) {
        scene.background = env!.backdrop;
        if (host) host.style.background = `linear-gradient(${env!.preset.backdrop[0]}, ${env!.preset.backdrop[1]})`;
      } else {
        scene.background = new THREE.Color(bg);
        if (host) host.style.background = bg;
      }
      // Grid contrast follows the background luminance so it reads on light and dark scenes.
      const c = new THREE.Color(lumOf);
      const lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
      const major = lum > 0.5 ? 0xa8a4bd : 0x475569;
      const minor = lum > 0.5 ? 0xd4d1e0 : 0x1e293b;
      grid.removeFromParent();
      grid.dispose();
      grid = new THREE.GridHelper(40, 40, major, minor);
      grid.renderOrder = -1;
      helpers?.add(grid);
    }
    grid.visible = (doc.grid ?? true) && !play;
    axes.visible = !play;
    invalidate();
  }

  function applyDocCamera(): void {
    if (!camera) return;
    camera.fov = doc.camera.fov;
    camera.near = doc.camera.near ?? 0.1;
    camera.far = doc.camera.far ?? 500;
    camera.position.set(...doc.camera.position);
    camera.updateProjectionMatrix();
    if (orbit) {
      orbit.target.set(...doc.camera.target);
      orbit.update();
    } else {
      camera.lookAt(...doc.camera.target);
    }
    cameraInitialised = true;
    invalidate();
  }

  function applyPlay(): void {
    if (!gizmoHelper) return;
    gizmoHelper.visible = !play && !readonly;
    for (const m of lightMarkers.values()) m.visible = !play;
    if (selectionBox) selectionBox.visible = !play;
    if (grid) grid.visible = (doc.grid ?? true) && !play;
    if (axes) axes.visible = !play;
    if (play) {
      gizmo?.detach();
      applyDocCamera();
    } else {
      syncSelection();
    }
    invalidate();
  }

  // ── selection & gizmo ────────────────────────────────────────────────────────
  function syncSelection(): void {
    if (!THREE || !helpers || !gizmo) return;
    const node = selectedId ? nodes.get(selectedId) : undefined;
    const kind = selectedId ? findNode(doc, selectedId)?.kind : null;
    if (selectionBox) {
      selectionBox.removeFromParent();
      selectionBox.dispose();
      selectionBox = null;
    }
    if (!node || play) {
      gizmo.detach();
      invalidate();
      return;
    }
    if (kind === 'group') {
      // Groups carry no transform in the doc (organisational only): outline, no gizmo.
      gizmo.detach();
    } else if (canEdit && !tween) {
      // Lights only translate; force the mode while one is selected.
      gizmo.setMode(kind === 'light' ? 'translate' : mode);
      gizmo.attach(node);
    } else {
      gizmo.detach();
    }
    if (kind === 'light') {
      const marker = lightMarkers.get(selectedId!);
      if (marker) {
        selectionBox = new THREE.BoxHelper(marker, 0xfbbf24);
        helpers.add(selectionBox);
      }
    } else {
      selectionBox = new THREE.BoxHelper(node, 0x8b7cff);
      helpers.add(selectionBox);
    }
    invalidate();
  }

  /** Gizmo moved the selected node → write the transform back through ops. */
  function commitGizmo(): void {
    if (!gizmo || !gizmo.object || !canEdit || !selectedId) return;
    const obj = gizmo.object;
    const n = findNode(doc, selectedId);
    if (!n) return;
    if (n.kind === 'light') {
      const marker = lightMarkers.get(selectedId);
      if (marker) marker.position.copy(obj.position);
      onchange(setTransform(doc, selectedId, { position: [obj.position.x, obj.position.y, obj.position.z] }));
      return;
    }
    if (n.kind !== 'object') return;
    const t = {
      position: [obj.position.x, obj.position.y, obj.position.z] as Vec3,
      rotation: [obj.rotation.x * RAD, obj.rotation.y * RAD, obj.rotation.z * RAD] as Vec3,
      scale: [obj.scale.x, obj.scale.y, obj.scale.z] as Vec3,
    };
    const st = editTargetState(doc, shownState, selectedId);
    onchange(st ? setStateOverride(doc, st, selectedId, t) : setTransform(doc, selectedId, t));
  }

  function pick(clientX: number, clientY: number): string | null {
    if (!THREE || !renderer || !camera || !content || !helpers) return null;
    const rect = renderer.domElement.getBoundingClientRect();
    const ndc = new THREE.Vector2(((clientX - rect.left) / rect.width) * 2 - 1, -((clientY - rect.top) / rect.height) * 2 + 1);
    const ray = new THREE.Raycaster();
    ray.setFromCamera(ndc, camera);
    const candidates: THREE_NS.Object3D[] = [content];
    if (!play) candidates.push(...lightMarkers.values());
    const hits = ray.intersectObjects(candidates, true);
    for (const h of hits) {
      // Walk up to the doc node that owns this mesh (gltf children are unnamed to us).
      let cur: THREE_NS.Object3D | null = h.object;
      while (cur) {
        const id = cur.userData?.[USERDATA_ID] as string | undefined;
        const kind = cur.userData?.[USERDATA_KIND] as string | undefined;
        if (id && (kind === 'object' || kind === 'light-marker')) return id;
        if (kind === 'group') break; // never select a group by clicking its children — pick the child
        cur = cur.parent;
      }
    }
    return null;
  }

  function onPointerDown(e: PointerEvent): void {
    pointerDown = { x: e.clientX, y: e.clientY };
    host?.focus({ preventScroll: true });
  }
  function onPointerUp(e: PointerEvent): void {
    const start = pointerDown;
    pointerDown = null;
    if (!start || dragging || e.button !== 0) return;
    if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > 4) return; // it was an orbit drag
    if (play) return;
    selectedId = pick(e.clientX, e.clientY);
    syncSelection();
  }

  /** F: frame the selection (or everything) — orbit target on the centre, distance from the bounds. */
  function frame(): void {
    if (!THREE || !camera || !orbit || !content) return;
    const target = selectedId ? nodes.get(selectedId) : undefined;
    const box = new THREE.Box3();
    if (target) box.setFromObject(target);
    else box.setFromObject(content);
    if (box.isEmpty()) {
      box.setFromCenterAndSize(new THREE.Vector3(0, 0.5, 0), new THREE.Vector3(2, 1, 2));
    }
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3()).length() || 1;
    const dist = (size / 2) / Math.tan((camera.fov * Math.PI) / 360) * 1.25;
    const dir = camera.position.clone().sub(orbit.target).normalize();
    if (!Number.isFinite(dir.length()) || dir.length() === 0) dir.set(1, 0.8, 1).normalize();
    moveCamera(center.clone().add(dir.multiplyScalar(dist)), center, camera.fov);
  }

  /** Write the current free camera into the doc (so Play / the agent / Blender see this view). */
  function saveViewAsCamera(): void {
    if (!camera || !orbit || readonly) return;
    onchange(setCamera(doc, currentView()));
  }

  // ── camera moves (named views, view cube) ────────────────────────────────────
  function moveCamera(pos: THREE_NS.Vector3, target: THREE_NS.Vector3, fov: number): void {
    if (!camera || !orbit) return;
    if (reducedMotion()) {
      camera.position.copy(pos);
      orbit.target.copy(target);
      camera.fov = fov;
      camera.updateProjectionMatrix();
      orbit.update();
      invalidate();
      return;
    }
    camTween = {
      fromPos: camera.position.clone(),
      fromTarget: orbit.target.clone(),
      toPos: pos.clone(),
      toTarget: target.clone(),
      fromFov: camera.fov,
      toFov: fov,
      start: performance.now(),
      duration: 450,
    };
  }

  function stepCamera(now: number): void {
    if (!camTween || !camera || !orbit) return;
    const t = transitionProgress(now - camTween.start, camTween.duration, 'ease-in-out');
    camera.position.lerpVectors(camTween.fromPos, camTween.toPos, t);
    orbit.target.lerpVectors(camTween.fromTarget, camTween.toTarget, t);
    camera.fov = camTween.fromFov + (camTween.toFov - camTween.fromFov) * t;
    camera.updateProjectionMatrix();
    orbit.update();
    dirty = true;
    if (now - camTween.start >= camTween.duration) camTween = null;
  }

  /** Fly to a camera (a named view, or the document camera). */
  export function goToView(cam: { position: Vec3; target: Vec3; fov?: number }): void {
    if (!THREE || !camera) return;
    moveCamera(new THREE.Vector3(...cam.position), new THREE.Vector3(...cam.target), cam.fov ?? camera.fov);
  }

  /** The free camera right now (for "Save view as…"). */
  export function currentView(): Scene3dCamera {
    if (!camera || !orbit) return doc.camera;
    return {
      position: [camera.position.x, camera.position.y, camera.position.z],
      target: [orbit.target.x, orbit.target.y, orbit.target.z],
      fov: camera.fov,
    };
  }

  const CUBE_FACES = [
    { id: 'front', label: 'Front', dir: [0, 0, 1] },
    { id: 'back', label: 'Back', dir: [0, 0, -1] },
    { id: 'right', label: 'Right', dir: [1, 0, 0] },
    { id: 'left', label: 'Left', dir: [-1, 0, 0] },
    { id: 'top', label: 'Top', dir: [0, 1, 0.0001] },
    { id: 'bottom', label: 'Bot', dir: [0, -1, 0.0001] },
  ] as const;

  function snapTo(dir: readonly [number, number, number]): void {
    if (!THREE || !camera || !orbit) return;
    const dist = camera.position.distanceTo(orbit.target);
    const d = new THREE.Vector3(...dir).normalize().multiplyScalar(dist);
    moveCamera(orbit.target.clone().add(d), orbit.target.clone(), camera.fov);
  }

  /** Rotate the DOM view cube to match the camera (written straight to style — no reactivity per frame). */
  function syncCube(): void {
    if (!cubeEl || !camera || !orbit) return;
    const off = camera.position.clone().sub(orbit.target);
    const r = off.length() || 1;
    const yaw = Math.atan2(off.x, off.z) * RAD;
    const pitch = Math.asin(Math.max(-1, Math.min(1, off.y / r))) * RAD;
    cubeEl.style.transform = `translateZ(-28px) rotateX(${pitch.toFixed(2)}deg) rotateY(${(-yaw).toFixed(2)}deg)`;
  }

  function setMode(m: GizmoMode): void {
    mode = m;
    if (gizmo && findNode(doc, selectedId)?.kind === 'object') gizmo.setMode(m);
    invalidate();
  }

  function isTypingTarget(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el) return false;
    const tag = el.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
  }

  function onKey(e: KeyboardEvent): void {
    if (isTypingTarget(e.target)) return;
    // Only react while the viewport is the thing the user is working in.
    const active = hovering || (host?.contains(document.activeElement) ?? false);
    if (!active || loading || loadError) return;
    const meta = e.metaKey || e.ctrlKey;
    const gm = !meta ? gizmoModeForKey(e.key) : null;
    if (gm) {
      if (canEdit) setMode(gm);
      e.preventDefault();
      return;
    }
    switch (e.key) {
      case 'f':
      case 'F':
        if (!meta) {
          frame();
          e.preventDefault();
        }
        return;
      case 'Escape':
        if (selectedId) {
          selectedId = null;
          syncSelection();
          e.preventDefault();
        }
        return;
      case 'Delete':
      case 'Backspace':
        if (canEdit && selectedId) {
          onchange(remove(doc, selectedId));
          selectedId = null;
          e.preventDefault();
        }
        return;
      case 'd':
      case 'D':
        if (meta && canEdit && selectedId) {
          const r = duplicate(doc, selectedId);
          if (r) {
            onchange(r.doc);
            selectedId = r.id;
          }
          e.preventDefault();
        }
        return;
    }
  }

  // ── lifecycle & effects ──────────────────────────────────────────────────────
  $effect(() => {
    if (host && !renderer && !destroyed) void boot(host);
  });
  // Reconcile the three scene whenever the document (or the colour resolver / state)
  // changes (agent edit, inspector, hierarchy, or our own gizmo write-back —
  // idempotent for the latter).
  $effect(() => {
    // Deep read: also re-runs if the host hands us a `$state` proxy and mutates it in place.
    const d = $state.snapshot(doc) as Scene3dDoc;
    void colors;
    void stateId;
    if (!renderer) return;
    reconcile(d);
    applyEnvironment();
    if (play) applyDocCamera();
    else if (!cameraInitialised) applyDocCamera();
  });
  $effect(() => {
    void selectedId;
    if (renderer) syncSelection();
  });
  $effect(() => {
    void play;
    void readonly;
    if (renderer) applyPlay();
  });
  $effect(() => {
    void turntable;
    lastFrame = 0;
    invalidate();
  });
  $effect(() => {
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  onDestroy(() => {
    destroyed = true;
    cancelAnimationFrame(raf);
    ro?.disconnect();
    orbit?.dispose();
    gizmo?.detach();
    gizmo?.dispose();
    selectionBox?.dispose();
    env?.dispose();
    if (content) disposeTree(content);
    if (lightsRoot) disposeTree(lightsRoot);
    if (helpers) disposeTree(helpers);
    grid?.dispose();
    axes?.dispose();
    for (const p of gltfCache.values()) p.then((t) => disposeTree(t)).catch(() => {});
    gltfCache.clear();
    if (renderer) {
      renderer.dispose();
      renderer.forceContextLoss();
      renderer.domElement.remove();
    }
    renderer = null;
    scene = null;
  });

  /** For the arena's Export ▾ → PNG: the current frame as a PNG blob. */
  export async function snapshotPng(): Promise<Blob | null> {
    if (!renderer || !scene || !camera) return null;
    renderer.render(scene, camera);
    return new Promise((res) => renderer!.domElement.toBlob((b) => res(b), 'image/png'));
  }

  /**
   * Export ▾ → PNG turntable: `frames` renders around the orbit target, laid
   * out as one sprite sheet (4 per row) — helpers hidden, the camera restored.
   */
  export async function turntableSheet(frames = 8, cell = 480): Promise<Blob | null> {
    if (!THREE || !renderer || !scene || !camera || !orbit || !helpers) return null;
    const cols = Math.min(4, frames);
    const rows = Math.ceil(frames / cols);
    const sheet = document.createElement('canvas');
    const src = renderer.domElement;
    const aspect = src.width / Math.max(1, src.height);
    const cw = cell;
    const ch = Math.round(cell / aspect);
    sheet.width = cw * cols;
    sheet.height = ch * rows;
    const ctx = sheet.getContext('2d');
    if (!ctx) return null;
    const pos = camera.position.clone();
    const helpersWere = helpers.visible;
    const gizmoWas = gizmoHelper?.visible ?? false;
    helpers.visible = false;
    if (gizmoHelper) gizmoHelper.visible = false;
    const offset = pos.clone().sub(orbit.target);
    try {
      for (let i = 0; i < frames; i++) {
        const o = offset.clone().applyAxisAngle(new THREE.Vector3(0, 1, 0), (i / frames) * Math.PI * 2);
        camera.position.copy(orbit.target).add(o);
        camera.lookAt(orbit.target);
        renderer.render(scene, camera);
        ctx.drawImage(src, (i % cols) * cw, Math.floor(i / cols) * ch, cw, ch);
      }
    } finally {
      camera.position.copy(pos);
      camera.lookAt(orbit.target);
      helpers.visible = helpersWere;
      if (gizmoHelper) gizmoHelper.visible = gizmoWas;
      invalidate();
    }
    return new Promise((res) => sheet.toBlob((b) => res(b), 'image/png'));
  }

  /** For Export ▾ → GLB: a clone of the live content (objects + groups + lights; helpers
   *  excluded). Geometry/materials are SHARED with the viewport — hand it to
   *  `exportObjectToGlb` and drop it; never `disposeTree` it. */
  export function contentRoot(): THREE_NS.Object3D | null {
    if (!THREE || !content || !lightsRoot) return null;
    const root = new THREE.Group();
    root.name = 'scene3d';
    root.add(content.clone(true), lightsRoot.clone(true));
    return root;
  }
</script>

<div
  class="s3d-viewport"
  class:play
  bind:this={host}
  tabindex="-1"
  role="application"
  aria-label="3D viewport"
  onpointerdown={onPointerDown}
  onpointerup={onPointerUp}
  onpointerenter={() => (hovering = true)}
  onpointerleave={() => (hovering = false)}
>
  {#if loading}
    <div class="s3d-overlay center"><span class="s3d-dim">Loading 3D engine…</span></div>
  {:else if loadError}
    <div class="s3d-overlay center">
      <div class="s3d-error">
        <strong>3D viewport unavailable</strong>
        <div class="s3d-dim">{loadError}</div>
      </div>
    </div>
  {/if}

  {#if !loading && !loadError && !play}
    <div class="s3d-hud" role="group" aria-label="Viewport tools" onpointerdown={(e) => e.stopPropagation()} onpointerup={(e) => e.stopPropagation()}>
      {#if canEdit}
        <div class="s3d-seg" role="radiogroup" aria-label="Gizmo mode">
          <button role="radio" aria-checked={mode === 'translate'} class:on={mode === 'translate'} title="Move (W)" onclick={() => setMode('translate')}>Move</button>
          <button role="radio" aria-checked={mode === 'rotate'} class:on={mode === 'rotate'} title="Rotate (E)" onclick={() => setMode('rotate')}>Rotate</button>
          <button role="radio" aria-checked={mode === 'scale'} class:on={mode === 'scale'} title="Scale (R)" onclick={() => setMode('scale')}>Scale</button>
        </div>
      {/if}
      <button class="s3d-hbtn" title="Frame selection (F)" onclick={frame}>Frame</button>
      {#if !readonly}
        <button class="s3d-hbtn" title="Use this view as the scene camera (Play + agent + Blender)" onclick={saveViewAsCamera}>
          View → camera
        </button>
      {/if}
    </div>
    {#if viewCube}
      <div class="s3d-cube-wrap" onpointerdown={(e) => e.stopPropagation()} onpointerup={(e) => e.stopPropagation()} role="group" aria-label="View cube">
        <div class="s3d-cube" bind:this={cubeEl}>
          {#each CUBE_FACES as f (f.id)}
            <button class="s3d-face {f.id}" title="{f.label} view" aria-label="{f.label} view" onclick={() => snapTo(f.dir)}>{f.label}</button>
          {/each}
        </div>
      </div>
    {/if}
    <div class="s3d-status" class:compact>
      {#if !compact}<span>{status}</span>{/if}
      {#if Object.keys(gltfErrors).length}
        <span class="s3d-warn" title={Object.values(gltfErrors).join('\n')}>
          {Object.keys(gltfErrors).length} model{Object.keys(gltfErrors).length === 1 ? '' : 's'} failed to load
        </span>
      {/if}
      {#if canEdit && !compact}
        <span class="s3d-dim s3d-keys">W/E/R gizmo · F frame · ⌫ delete · ⌘D duplicate · Esc deselect</span>
      {/if}
      {#if compact}
        <span class="s3d-pill" data-testid="s3d-stats">{status}{statusNote ? ` · ${statusNote}` : ''}</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .s3d-viewport {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 240px;
    overflow: hidden;
    background: var(--term-bg);
    outline: none;
    border-radius: var(--radius-m, 8px);
    user-select: none;
  }
  .s3d-viewport:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .s3d-overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .s3d-overlay.center {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .s3d-error {
    pointer-events: auto;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    padding: 12px 14px;
    max-width: 360px;
    font-size: var(--fs-s);
    display: grid;
    gap: 4px;
  }
  .s3d-dim {
    color: var(--text-dim);
  }
  .s3d-hud {
    position: absolute;
    inset-block-start: 8px;
    inset-inline-start: 8px;
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .s3d-seg {
    display: inline-flex;
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 5px);
    overflow: hidden;
    backdrop-filter: blur(6px);
  }
  .s3d-seg button,
  .s3d-hbtn {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--text);
    font: var(--fs-xs) / 1 var(--font-ui);
    padding: 6px 9px;
    cursor: pointer;
  }
  .s3d-seg button + button {
    border-inline-start: 1px solid var(--border);
  }
  .s3d-seg button.on {
    background: var(--accent-solid, var(--accent));
    color: var(--accent-contrast);
  }
  .s3d-hbtn {
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 5px);
    backdrop-filter: blur(6px);
  }
  .s3d-hbtn:hover,
  .s3d-seg button:hover:not(.on) {
    background: color-mix(in srgb, var(--surface-2) 92%, transparent);
  }
  .s3d-seg button:focus-visible,
  .s3d-hbtn:focus-visible,
  .s3d-face:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .s3d-status {
    position: absolute;
    inset-inline: 8px;
    inset-block-end: 6px;
    display: flex;
    gap: 12px;
    align-items: center;
    font: var(--fs-xs) / 1.2 var(--font-ui);
    color: var(--text);
    pointer-events: none;
    text-shadow: 0 1px 2px color-mix(in srgb, var(--bg) 60%, transparent);
  }
  .s3d-status.compact {
    justify-content: flex-end;
    inset-block-end: 12px;
    inset-inline-end: 12px;
    text-shadow: none;
  }
  .s3d-status > span:first-child {
    font-variant-numeric: tabular-nums;
  }
  .s3d-pill {
    padding: 5px 10px;
    border-radius: var(--radius-m);
    background: color-mix(in srgb, var(--surface) 90%, transparent);
    border: 1px solid var(--border);
    color: var(--text);
    font-variant-numeric: tabular-nums;
    backdrop-filter: blur(6px);
  }
  .s3d-keys {
    margin-inline-start: auto;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .s3d-warn {
    color: var(--warning);
    pointer-events: auto;
  }
  /* View cube: a CSS 3D cube synced to the orbit camera; faces snap the view. */
  .s3d-cube-wrap {
    position: absolute;
    inset-block-start: 12px;
    inset-inline-end: 12px;
    width: 76px;
    height: 76px;
    perspective: 260px;
  }
  .s3d-cube {
    position: relative;
    width: 56px;
    height: 56px;
    margin: 10px;
    transform-style: preserve-3d;
    transform: translateZ(-28px);
  }
  .s3d-face {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--border-strong);
    background: color-mix(in srgb, var(--surface) 92%, transparent);
    color: var(--text-dim);
    font: 600 var(--fs-xs) / 1 var(--font-ui);
    letter-spacing: 0.02em;
    text-transform: uppercase;
    cursor: pointer;
    backface-visibility: hidden;
  }
  .s3d-face:hover {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .s3d-face.front { transform: translateZ(28px); }
  .s3d-face.back { transform: rotateY(180deg) translateZ(28px); }
  .s3d-face.right { transform: rotateY(90deg) translateZ(28px); }
  .s3d-face.left { transform: rotateY(-90deg) translateZ(28px); }
  .s3d-face.top { transform: rotateX(90deg) translateZ(28px); }
  .s3d-face.bottom { transform: rotateX(-90deg) translateZ(28px); }
  @media (max-width: 720px) {
    .s3d-keys {
      display: none;
    }
  }
</style>
