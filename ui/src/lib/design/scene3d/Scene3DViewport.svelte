<script lang="ts">
  import { plural } from '../../plural';
  // 3D Studio viewport: lazy three.js, orbit + TransformControls gizmo,
  // PBR lighting, procedural environments, named states/tweens, view cube,
  // shading/wireframe inspection, sub-mesh outliner, and polygon picking.
  import { onDestroy } from 'svelte';
  import { demandLoop, onContextLost, releaseRenderer, watchVisibility } from '../../three/stage';
  import { fitCamera } from './cameraFit';
  import { standOnFloor, subjectBox, subjectBoxes } from './subjectBox';
  import { viewportHasWork, viewportOwnsKey } from './viewportLoop';
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
  import { SettingView } from './settingView';
  import { cloneModel, loadModelAsset, renderSnapshotPng, renderTurntableSheet } from './viewportCapture';
  import { ViewportInspection, type Pin3dInput } from './viewportInspection';
  import ViewCube from './ViewCube.svelte';
  import ViewportHud from './ViewportHud.svelte';
  import LoadState from '../../components/LoadState.svelte';
  import type { MeshTopologyStats, ShadingMode, SubMeshInfo } from './meshInspector';
  import type { Pin3dAnchor, ProjectedPin3d, SelectionKind3d } from './pin3d';
  import type { RigInfo } from './rigPlayer';

  interface Props {
    doc: Scene3dDoc;
    readonly?: boolean;
    selectedId?: string | null;
    play?: boolean;
    onchange: (doc: Scene3dDoc) => void;
    resolveAttachment: (ref: string) => Promise<string>;
    colors?: ColorResolver;
    stateId?: string | null;
    turntable?: boolean;
    viewCube?: boolean;
    /** The HUD's "View → camera" button (the 3D Studio puts it in its ⋯ menu). */
    cameraButton?: boolean;
    /** A standalone model has no authored framing: look at it and come as close as it fits. */
    frameModel?: boolean;
    /** Shown as Retry on the "3D preview unavailable" state (the host remounts). */
    onretry?: () => void;
    statusNote?: string;
    compact?: boolean;
    shading?: ShadingMode;
    wireframeOverlay?: boolean;
    scaleRef?: boolean;
    showSkeleton?: boolean;
    animClipIndex?: number;
    animPlaying?: boolean;
    pickMode?: boolean;
    pickKind?: SelectionKind3d;
    pin3dInputs?: Pin3dInput[];
    activeAnchor?: Pin3dAnchor | null;
    onpolygonhover?: (anchor: Pin3dAnchor | null) => void;
    onpolygonpick?: (anchor: Pin3dAnchor, additive: boolean) => void;
    onprojectedpins?: (pins: ProjectedPin3d[]) => void;
    ontopology?: (meshes: SubMeshInfo[], stats: MeshTopologyStats) => void;
    onriginfo?: (info: RigInfo) => void;
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
    cameraButton = true,
    frameModel = false,
    onretry,
    statusNote = '',
    compact = false,
    shading = 'shaded',
    wireframeOverlay = false,
    scaleRef = false,
    showSkeleton = false,
    animClipIndex = 0,
    animPlaying = false,
    pickMode = false,
    pickKind = 'polygon',
    pin3dInputs = [],
    activeAnchor = null,
    onpolygonhover,
    onpolygonpick,
    onprojectedpins,
    ontopology,
    onriginfo,
  }: Props = $props();

  let host = $state<HTMLDivElement | null>(null);
  let cubeEl = $state<HTMLDivElement | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let mode = $state<GizmoMode>('translate');
  let gltfErrors = $state<Record<string, string>>({});
  const status = $derived(summarize(doc));
  const canEdit = $derived(!readonly && !play);
  // Move / Rotate / Scale only mean something with an object selected.
  const gizmoModes = $derived(canEdit && !!selectedId && findNode(doc, selectedId)?.kind === 'object');
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
  let inspector: ViewportInspection | null = null;
  let env: BuiltEnvironment | null = null;
  let envKey = '';
  let setting: SettingView | null = null; // a Smart mode setting: sky, ground, fog, weather, halos
  let ro: ResizeObserver | null = null;
  let visible = true; // in (or near) the viewport, document visible
  let dirty = true;
  let destroyed = false;
  let dragging = false;
  let cameraInitialised = false;
  let userMoved = false; // the person orbited or flew: stop auto-fitting the first view
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

  // On-demand frames: `invalidate` requests one; only motion re-arms (viewportLoop.ts).
  const loop = demandLoop((now) => step(now), { canRun: () => !destroyed && visible });
  const invalidate = () => {
    dirty = true;
    loop.kick();
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
      // A driver that can't compile our shaders or drops the context would
      // otherwise leave a blank canvas under a "saved" status: say so instead.
      renderer.debug.onShaderError = () => fail('This device’s graphics driver couldn’t compile the 3D shaders.');
      onContextLost(renderer.domElement, () => fail('The graphics processor dropped the 3D view (WebGL context lost).'), () => onretry?.());
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.shadowMap.enabled = true;
      renderer.shadowMap.type = T.PCFShadowMap; // r18x folds PCFSoft into PCF (and warns)
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
      orbit.target.set(...doc.camera.target); // else the first orbit snaps to the origin
      orbit.addEventListener('change', invalidate);
      orbit.addEventListener('start', () => ((camTween = null), (userMoved = true)));

      gizmo = new tcMod.TransformControls(camera, renderer.domElement);
      gizmo.setMode(mode);
      gizmo.addEventListener('change', invalidate);
      gizmo.addEventListener('dragging-changed', (e) => {
        dragging = Boolean((e as unknown as { value: boolean }).value);
        if (orbit) orbit.enabled = !dragging;
        if (!dragging) commitGizmo(); // final, rounded write at drag end
      });
      // SD-18: during a drag three.js already moves the node; committing per
      // pointer move serialized, re-parsed, validated and reconciled the whole
      // scene 60×/s. Mid-drag only the light marker follows; the doc is written
      // once, at drag end (`dragging-changed` above).
      gizmo.addEventListener('objectChange', () => {
        if (dragging) followLightMarker();
        else commitGizmo();
      });
      gizmoHelper = gizmo.getHelper();
      scene.add(gizmoHelper);

      grid = new T.GridHelper(40, 40);
      axes = new T.AxesHelper(1.5);
      grid.renderOrder = -1;
      helpers.add(grid, axes);
      inspector = new ViewportInspection(T, helpers);

      loader = new gltfMod.GLTFLoader();
      loader.setMeshoptDecoder(meshopt.MeshoptDecoder);

      ro = new ResizeObserver(() => resize());
      ro.observe(el);
      resize();
      reconcile(doc);
      fitView();
      applyEnvironment();
      applyPlay();
      inspector.applyShading(content, shading, wireframeOverlay);
      inspector.setScaleRef(scaleRef && !play);
      emitTopology();
      loading = false;
      invalidate();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      fail(/webgl/i.test(msg) ? 'WebGL is off or unsupported in this browser, so the 3D preview can’t start.' : msg);
    }
  }

  function fail(reason: string): void {
    if (destroyed) return;
    loadError = reason;
    loading = false;
    loop.stop();
  }

  /** Dolly the first view out until the whole subject fits this viewport's aspect (the doc camera otherwise). */
  function fitView(): void {
    if (!THREE || !camera || !orbit || !content || !host || userMoved || play) return;
    const box = subjectBox(THREE, doc, nodes, content);
    if (box.isEmpty()) return;
    // Portrait: the HUD (top) and states bar / stats pill (bottom) span the width, so keep the
    // model out from under them. Landscape: they sit at the sides of a centred model. Compact hides both.
    const h = Math.max(1, host.clientHeight);
    const portrait = camera.aspect < 1.2 && !compact;
    const t = frameModel ? box.getCenter(new THREE.Vector3()) : new THREE.Vector3(...doc.camera.target);
    const from = new THREE.Vector3(...doc.camera.position).sub(new THREE.Vector3(...doc.camera.target)).add(t);
    const fit = fitCamera({ position: from.toArray(), target: t.toArray(), fov: camera.fov, aspect: camera.aspect, closer: frameModel, boxes: frameModel ? subjectBoxes(THREE, doc, nodes) : undefined,
      min: box.min.toArray(), max: box.max.toArray(), topReserve: portrait ? Math.min(0.3, 56 / h) : 0, bottomReserve: portrait ? Math.min(0.3, 60 / h) : 0 });
    camera.position.set(...fit.position);
    orbit.target.set(...fit.target);    orbit.update();
    invalidate();
  }

  function resize(): void {
    if (!host || !renderer || !camera) return;
    const w = Math.max(1, host.clientWidth);
    const h = Math.max(1, host.clientHeight);
    renderer.setSize(w, h, false);
    // A toolbar floating over the top (`--s3d-top-inset`, a phone on its side): the view is framed
    // for the canvas below it and drawn on up under the toolbar, so the model centres in the free part.
    const inset = Math.min(h / 3, parseFloat(getComputedStyle(host).getPropertyValue('--s3d-top-inset')) || 0);
    camera.aspect = w / (h - inset);
    if (inset) camera.setViewOffset(w, h - inset, 0, -inset, w, h);
    else camera.clearViewOffset();
    camera.updateProjectionMatrix();
    fitView();
    invalidate();
  }

  /** One frame; true while something still moves. */
  function step(now: number): boolean {
    const dt = lastFrame ? Math.min(0.1, (now - lastFrame) / 1000) : 0;
    lastFrame = now;
    stepTween(now);
    stepCamera(now);
    if ([inspector?.rig.step(dt), setting?.step(now)].some(Boolean)) dirty = true;
    if (turntable && !dragging && camera && orbit && !camTween) {
      const speed = doc.turntable?.speed ?? 20;
      const offset = camera.position.clone().sub(orbit.target);
      offset.applyAxisAngle(new THREE!.Vector3(0, 1, 0), speed * DEG * dt);
      camera.position.copy(orbit.target).add(offset);
      camera.lookAt(orbit.target);
      dirty = true;
    }
    if (dirty && renderer && scene && camera) {
      dirty = false;
      if (selectionBox) selectionBox.update();
      renderer.render(scene, camera);
      syncCube();
      emitProjectedPins();
    }
    return viewportHasWork({ tween: !!tween, camTween: !!camTween, turntable, dragging, animating: inspector?.rig.isAnimating || setting?.animating });
  }

  function emitTopology(): void {
    if (!inspector || !content) return;
    if (ontopology) {
      const { meshes, stats } = inspector.inspect(content);
      ontopology(meshes, stats);
    }
    onriginfo?.(inspector.syncRig(content));
  }
  function emitProjectedPins(): void {
    if (!inspector || !camera || !host || !onprojectedpins) return;
    onprojectedpins(inspector.projectPins(camera, host.clientWidth, host.clientHeight, pin3dInputs));
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
    if (content) inspector?.reapplyShading(content);
    emitTopology();
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
    invalidate();
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
      p = resolveAttachment(ref).then((url) => loadModelAsset(T, loader!, url));
      gltfCache.set(ref, p);
    }
    p.then(
      (template) => {
        if (destroyed || !holder.parent) return;
        holder.remove(placeholder);
        disposeTree(placeholder);
        const cloned = cloneModel(template);
        if (template.userData?.ottoAnimations) cloned.userData.ottoAnimations = template.userData.ottoAnimations;
        holder.add(cloned);
        if (frameModel) standOnFloor(THREE!, cloned);
        delete gltfErrors[ref];
        gltfErrors = { ...gltfErrors };
        if (content) inspector?.reapplyShading(content);
        if (content && activeAnchor) inspector?.highlightAnchor(content, activeAnchor);
        fitView();
        emitTopology();
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
    axes.visible = !play && !readonly;
    (setting ??= new SettingView(THREE, scene)).apply(e, doc.lights, doc.camera);
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
    if (axes) axes.visible = !play && !readonly;
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

  /** Keep a dragged light's marker on the light (no doc write). */
  function followLightMarker(): void {
    if (!gizmo?.object || !selectedId) return;
    const marker = lightMarkers.get(selectedId);
    if (marker) {
      marker.position.copy(gizmo.object.position);
      invalidate();
    }
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
  function onPointerMove(e: PointerEvent): void {
    if (!pickMode || dragging || play || !inspector || !renderer || !camera || !orbit || !content) return;
    const hit = inspector.pickSurface(renderer, camera, orbit.target, content, e.clientX, e.clientY, pickKind);
    onpolygonhover?.(hit);
    invalidate();
  }
  function onPointerLeave(): void {
    if (!pickMode || !inspector) return;
    inspector.clearHover();
    onpolygonhover?.(null);
    invalidate();
  }
  function onPointerUp(e: PointerEvent): void {
    const start = pointerDown;
    pointerDown = null;
    if (!start || dragging || e.button !== 0) return;
    if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > 4) return;
    if (play) return;
    if (pickMode && inspector && renderer && camera && orbit && content) {
      const hit = inspector.pickSurface(renderer, camera, orbit.target, content, e.clientX, e.clientY, pickKind);
      if (hit) onpolygonpick?.(hit, e.shiftKey || pickKind === 'region');
      invalidate();
      return;
    }
    selectedId = pick(e.clientX, e.clientY);
    syncSelection();
  }

  /** F: frame the selection (or everything) — orbit target on the centre, distance from the bounds. */
  function frame(): void {
    if (!THREE || !camera || !orbit || !content) return;
    const target = selectedId ? nodes.get(selectedId) : undefined;
    const box = new THREE.Box3();
    if (target) box.setFromObject(target);
    else box.copy(subjectBox(THREE, doc, nodes, content));
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
  export function saveViewAsCamera(): void {
    if (!camera || !orbit || readonly) return;
    onchange(setCamera(doc, currentView()));
  }

  // ── camera moves (named views, view cube) ────────────────────────────────────
  function moveCamera(pos: THREE_NS.Vector3, target: THREE_NS.Vector3, fov: number): void {
    if (!camera || !orbit) return;
    userMoved = true;
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
    invalidate();
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
    // CSS y points down: a camera ABOVE the target (pitch > 0) tips the cube's top toward us.
    cubeEl.style.transform = `translateZ(-28px) rotateX(${(-pitch).toFixed(2)}deg) rotateY(${(-yaw).toFixed(2)}deg)`;
  }

  function setMode(m: GizmoMode): void {
    mode = m;
    if (gizmo && findNode(doc, selectedId)?.kind === 'object') gizmo.setMode(m);
    invalidate();
  }

  function onKey(e: KeyboardEvent): void {
    // Focus in the viewport only (pointer-down focuses it); hovering never counts.
    if (!viewportOwnsKey(host, e.target) || loading || loadError) return;
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
    const s = shading;
    const w = wireframeOverlay;
    const sr = scaleRef && !play;
    const a = activeAnchor;
    const sk = showSkeleton && !play;
    const ci = animClipIndex;
    const ap = animPlaying;
    if (!inspector) return;
    if (content) {
      inspector.applyShading(content, s, w);
      inspector.highlightAnchor(content, a);
    }
    inspector.setScaleRef(sr);
    inspector.rig.setSkeletonVisible(sk);
    inspector.rig.setPlayback(ci, ap);
    if (!pickMode) inspector.clearHover();
    lastFrame = 0;
    invalidate();
  });
  $effect(() => {
    void pin3dInputs;
    if (renderer) emitProjectedPins();
  });
  // Off screen (a scrolled-away card, a hidden pane) or a hidden document: no frames.
  $effect(() => {
    const el = host;
    if (!el) return;
    return watchVisibility(el, (v) => {
      visible = v;
      if (!v) loop.stop();
      else invalidate();
    });
  });

  onDestroy(() => {
    destroyed = true;
    loop.stop();
    ro?.disconnect();
    orbit?.dispose();
    gizmo?.detach();
    gizmo?.dispose();
    selectionBox?.dispose();
    inspector?.dispose();
    env?.dispose(), setting?.dispose();
    if (content) disposeTree(content);
    if (lightsRoot) disposeTree(lightsRoot);
    if (helpers) disposeTree(helpers);
    grid?.dispose();
    axes?.dispose();
    for (const p of gltfCache.values()) p.then((t) => disposeTree(t)).catch(() => {});
    gltfCache.clear();
    if (renderer) releaseRenderer(renderer);
    renderer = null;
    scene = null;
  });

  export async function snapshotPng(): Promise<Blob | null> {
    if (!renderer || !scene || !camera) return null;
    return renderSnapshotPng(renderer, scene, camera);
  }
  export async function turntableSheet(frames = 8, cell = 480): Promise<Blob | null> {
    if (!THREE || !renderer || !scene || !camera || !orbit || !helpers) return null;
    return renderTurntableSheet(THREE, renderer, scene, camera, orbit, helpers, gizmoHelper, invalidate, frames, cell);
  }
  export function focusMesh(uid: string): void {
    if (!inspector || !content || !camera || !orbit) return;
    const pose = inspector.cameraForMeshUid(content, camera, orbit.target, uid);
    if (pose) goToView({ position: pose.pos, target: pose.target, fov: pose.fov ?? undefined });
  }
  export function toggleMeshVisible(uid: string): void {
    if (!inspector || !content) return;
    if (inspector.toggleMeshVisible(content, uid)) {
      emitTopology();
      invalidate();
    }
  }
  export function anchorForMesh(uid: string): Pin3dAnchor | null {
    if (!inspector || !content || !camera || !orbit) return null;
    return inspector.anchorForMeshUid(content, camera, orbit.target, uid);
  }
  export function focusAnchor(anchor: Pin3dAnchor): void {
    if (!inspector || !content) return;
    inspector.highlightAnchor(content, anchor);
    const pose = inspector.cameraForAnchor(anchor);
    if (pose) goToView({ position: pose.pos, target: pose.target, fov: pose.fov ?? undefined });
    invalidate();
  }
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
  class:picking={pickMode}
  bind:this={host}
  tabindex="-1"
  role="application"
  aria-label="3D viewport"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerleave={onPointerLeave}
  onpointerup={onPointerUp}
  {@attach (el) => (el.addEventListener('keydown', onKey), () => el.removeEventListener('keydown', onKey))}
>
  {#if loading}
    <div class="s3d-overlay center"><span class="spinner" role="img" aria-label="Loading the 3D engine" title="Loading the 3D engine"></span></div>
  {:else if loadError}
    <div class="s3d-overlay center">
      <div class="s3d-error" data-testid="s3d-unavailable">
        <LoadState what="the 3D preview" errorTitle="3D preview unavailable" error={loadError} empty {onretry} variant="compact" />
      </div>
    </div>
  {/if}

  {#if !loading && !loadError && !play}
    <ViewportHud {gizmoModes} {mode} onmode={setMode} onframe={frame} oncamera={!readonly && cameraButton ? saveViewAsCamera : undefined} />
    {#if viewCube}
      <ViewCube bind:cubeEl onsnap={snapTo} />
    {/if}
    <div class="s3d-status" class:compact>
      {#if !compact}<span>{status}</span>{/if}
      {#if Object.keys(gltfErrors).length}
        <span class="s3d-warn" title={Object.values(gltfErrors).join('\n')}>
          Couldn’t load {plural(Object.keys(gltfErrors).length, 'model')}
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
    /* A short landscape phone (≈300 px of page) must not push the stats pill off screen. */
    min-height: min(240px, 40vh);
    overflow: hidden;
    background: var(--term-bg);
    outline: none;
    border-radius: var(--radius-m);
    user-select: none;
  }
  .s3d-viewport.picking {
    cursor: crosshair;
  }
  .s3d-viewport:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
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
    border-radius: var(--radius-m);
    padding: 12px 14px;
    max-width: 360px;
    font-size: var(--fs-s);
    display: grid;
    gap: 4px;
  }
  .s3d-dim {
    color: var(--text-dim);
  }
  @media (pointer: coarse) {
    .s3d-error :global(.btn) {
      min-block-size: 40px;
    }
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
    padding: 4px 10px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text);
    font-variant-numeric: tabular-nums;
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
  @media (max-width: 640px) {
    .s3d-keys {
      display: none;
    }
  }
</style>
