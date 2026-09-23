// 3D embed runtime — render a scene3d (or GLB/glTF) Design Hall artifact
// inside any element: the Site Studio's "3D embed" block, cards, previews.
//
//   import { mountScene3dEmbed } from '../studio3d/embed';
//   const h = await mountScene3dEmbed(el, {
//     src: 'otto://design/<id>@approved#view:hero',   // a scene3d or glb artifact
//     autoRotate: true, drag: true, tiltOnHover: true, poster: thumbUrl,
//   });
//   h.setState('flipped'); … h.destroy();
//
// • Version: the URI selector (`@approved` — the default for render links —
//   `@latest`, `@vN`) picks the bytes; `version` overrides it.
// • Camera: `view` (or `#view:<id>` in `src`) picks a named camera preset of
//   the scene (`cameras[]`, e.g. "Hero angle"); else the document camera.
// • States: `hoverState` (default: a state with id `hover`, if the scene has
//   one) is tweened to on pointer-enter and back on leave — the Spline-style
//   "tilt on hover"; `tiltOnHover` also leans the model toward the pointer.
// • Poster: shown until the first frame, and kept when WebGL / loading fails.
// • Motion: `prefers-reduced-motion` turns off auto-rotate and tilt and makes
//   state changes instant. Off-screen embeds stop rendering.
//
// `three` (and the loaders) are lazy-loaded on first mount — the host bundle
// only carries this small module. Every fetch goes through the authed Design
// Hall client; a document can never make the embed fetch an arbitrary URL.
// Exported (static) sites will ship their own resolver via `resolve`.
import type * as THREE_NS from 'three';
import { fetchContent, getArtifact } from '../../../lib/api/design';
import { parseOttoUri } from '../model';
import {
  applyTransform,
  buildLight,
  buildMesh,
  disposeTree,
  DEG,
  loadThree,
  type ColorResolver,
  type Three,
} from '../../product/design/scene3d/build';
import { buildEnvironment, type BuiltEnvironment } from '../../product/design/scene3d/environment';
import { parseScene } from '../../product/design/scene3d/validate';
import { emptyScene, type Easing, type Scene3dDoc } from '../../product/design/scene3d/types';
import { resolveColor } from '../../product/design/scene3d/tokens';
import { resolveMaterial, type ResolvedMaterial } from '../../product/design/scene3d/presets';
import { findState, initialState, lerpPoses, stateTiming, statePoses, transitionProgress, type Pose } from '../../product/design/scene3d/states';
import { loadBrandKit, resolveModelRef, versionFor } from './sources';

export interface Scene3dEmbedOptions {
  /** `otto://design/<id>[@approved|@latest|@vN][#view:<camera>]`. */
  src: string;
  /** Explicit version (`v12` or a version id) — overrides the URI selector. */
  version?: string | null;
  /** Named camera id (overrides `#view:` in `src`). */
  view?: string | null;
  /** Slow turntable (ignored under reduced motion). Default false. */
  autoRotate?: boolean;
  /** Orbit on drag / wheel zoom off. Default true. */
  drag?: boolean;
  /** Lean toward the pointer and tween to `hoverState` on hover. Default false. */
  tiltOnHover?: boolean;
  /** State shown while hovered (default: the scene's `hover` state, if any). */
  hoverState?: string | null;
  /** Image URL shown until the first frame (and on failure). */
  poster?: string | null;
  /** `transparent` lets the page show through (no environment backdrop). */
  background?: 'scene' | 'transparent';
  /** Accessible label for the canvas (default: the artifact title). */
  label?: string;
  /** Injected loaders (exported sites / tests). Default: the authed Design Hall client. */
  resolve?: {
    /** Text of a scene3d document, or null when `src` is a model. */
    scene?: (src: string, version: string | null) => Promise<{ doc: Scene3dDoc; title: string; brand: unknown } | null>;
    model?: (ref: string) => Promise<string>;
  };
}

export interface Scene3dEmbedHandle {
  /** Resolves when the first frame rendered (rejects on failure — the poster stays). */
  ready: Promise<void>;
  /** Tween to a state (null = the scene's initial state). */
  setState(id: string | null): void;
  /** Fly to a named camera. */
  setView(id: string | null): void;
  /** Named cameras + states of the loaded scene (for block inspectors). */
  info(): { title: string; views: { id: string; name: string }[]; states: { id: string; name: string }[] };
  destroy(): void;
}

/** `otto://design/<id>@approved#view:hero` from parts (what a site block stores). */
export function embedSrc(artifactId: string, opts: { selector?: 'approved' | 'latest' | number; view?: string | null } = {}): string {
  const sel = opts.selector === undefined ? '@approved' : typeof opts.selector === 'number' ? `@v${opts.selector}` : `@${opts.selector}`;
  return `otto://design/${artifactId}${sel}${opts.view ? `#view:${opts.view}` : ''}`;
}

/** Load the scene behind `src` through the authed client (scene3d doc, or a model wrapped in one). */
async function defaultScene(src: string, version: string | null): Promise<{ doc: Scene3dDoc; title: string; brand: unknown }> {
  const uri = parseOttoUri(src);
  if (!uri) throw new Error('Not an otto://design reference');
  const d = await getArtifact(uri.artifactId);
  const a = d.artifact;
  const v = version ?? versionFor(a, uri.selector);
  if (a.format === 'glb' || a.format === 'gltf') {
    const base = emptyScene();
    const doc: Scene3dDoc = {
      ...base,
      background: '#eceaf3',
      grid: false,
      environment: { preset: 'studio-soft' },
      camera: { position: [2.4, 1.6, 3.2], target: [0, 0.5, 0], fov: 35 },
      lights: [{ id: 'key', type: 'directional', position: [3, 5, 2], intensity: 1.2, shadow: true }],
      objects: [
        {
          id: 'model', name: a.title, type: 'gltf',
          src: `otto://design/${a.id}${modelSelector(uri.selector, a, version)}`,
          position: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1],
        },
      ],
    };
    return { doc, title: a.title, brand: null };
  }
  if (a.format !== 'scene3d') throw new Error(`Can't embed a ${a.format} design in 3D`);
  const c = await fetchContent(a.id, { version: v ?? undefined, asText: true });
  const r = parseScene(c.text ?? '');
  if (!r.ok) throw new Error('The scene document is not valid');
  const kit = await loadBrandKit(r.doc.brand, a).catch(() => null);
  return { doc: r.doc, title: a.title, brand: kit?.doc ?? null };
}

/** The selector a wrapped model reference keeps: an explicit `vN`, else the URI's, else approved-or-latest. */
function modelSelector(sel: NonNullable<ReturnType<typeof parseOttoUri>>['selector'], a: { approved_version_id: string | null }, version: string | null): string {
  if (version && /^v\d+$/.test(version)) return `@${version}`;
  if (sel.kind === 'version') return `@v${sel.seq}`;
  if (sel.kind === 'latest') return '@latest';
  return a.approved_version_id ? '@approved' : '@latest';
}

const reducedMotion = (): boolean =>
  typeof window !== 'undefined' && !!window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

export async function mountScene3dEmbed(el: HTMLElement, opts: Scene3dEmbedOptions): Promise<Scene3dEmbedHandle> {
  const reduced = reducedMotion();
  let destroyed = false;
  let raf = 0;
  let visible = true;

  // Poster first — the page never shows an empty box.
  el.style.position ||= 'relative';
  let poster: HTMLImageElement | null = null;
  if (opts.poster) {
    poster = document.createElement('img');
    poster.src = opts.poster;
    poster.alt = '';
    poster.setAttribute('aria-hidden', 'true');
    Object.assign(poster.style, { position: 'absolute', inset: '0', width: '100%', height: '100%', objectFit: 'contain' });
    el.appendChild(poster);
  }

  let THREE: Three | null = null;
  let renderer: THREE_NS.WebGLRenderer | null = null;
  let scene: THREE_NS.Scene | null = null;
  let camera: THREE_NS.PerspectiveCamera | null = null;
  let controls: { update(): void; dispose(): void; target: THREE_NS.Vector3; enabled: boolean } | null = null;
  let env: BuiltEnvironment | null = null;
  let content: THREE_NS.Group | null = null;
  let io: IntersectionObserver | null = null;
  let ro: ResizeObserver | null = null;
  let doc: Scene3dDoc | null = null;
  let title = opts.label ?? '';
  const nodes = new Map<string, THREE_NS.Object3D>();
  const baseMat = new Map<string, ResolvedMaterial>();
  let color: ColorResolver = (r, fb) => resolveColor(r, null, fb);
  let shown: string | null = null;
  let poses: Map<string, Pose> | null = null;
  let tween: { from: Map<string, Pose>; to: Map<string, Pose>; start: number; duration: number; easing: Easing } | null = null;
  let tilt = { x: 0, y: 0, tx: 0, ty: 0 };
  let last = 0;
  let needsFrame = true;
  const cleanups: (() => void)[] = [];

  function applyPoses(p: Map<string, Pose>): void {
    for (const [id, pose] of p) {
      const n = nodes.get(id);
      if (!n) continue;
      n.position.set(...pose.position);
      n.rotation.set(pose.rotation[0] * DEG, pose.rotation[1] * DEG, pose.rotation[2] * DEG);
      n.scale.set(pose.scale[0] || 1e-4, pose.scale[1] || 1e-4, pose.scale[2] || 1e-4);
      n.visible = pose.visible;
      const base = baseMat.get(id);
      const mat = (n as THREE_NS.Mesh).material as THREE_NS.MeshPhysicalMaterial | undefined;
      if (base && mat && 'clearcoat' in mat && !mat.map) {
        mat.color.set(pose.color ?? base.color);
        mat.emissive.set(pose.emissive ?? base.emissive);
        mat.opacity = pose.opacity ?? base.opacity;
        const t = mat.opacity < 1;
        if (mat.transparent !== t) {
          mat.transparent = t;
          mat.needsUpdate = true;
        }
      }
    }
    needsFrame = true;
  }

  function goToState(id: string | null): void {
    if (!doc) return;
    const want = id && findState(doc, id) ? id : initialState(doc);
    if (want === shown) return;
    const to = statePoses(doc, want, (c) => color(c, '#94a3b8'));
    const { duration, easing } = stateTiming(findState(doc, want), reduced);
    const from = poses ?? statePoses(doc, shown, (c) => color(c, '#94a3b8'));
    shown = want;
    if (duration <= 0) {
      tween = null;
      poses = to;
      applyPoses(to);
    } else tween = { from, to, start: performance.now(), duration, easing };
  }

  function viewCamera(id: string | null): { position: THREE_NS.Vector3Tuple; target: THREE_NS.Vector3Tuple; fov: number } | null {
    if (!doc) return null;
    const preset = id ? doc.cameras?.find((c) => c.id === id) : undefined;
    const cam = preset ?? doc.camera;
    return { position: [...cam.position], target: [...cam.target], fov: cam.fov ?? doc.camera.fov };
  }

  function setView(id: string | null): void {
    const v = viewCamera(id);
    if (!v || !camera) return;
    camera.position.set(...v.position);
    camera.fov = v.fov;
    camera.updateProjectionMatrix();
    if (controls) {
      controls.target.set(...v.target);
      controls.update();
    } else camera.lookAt(...v.target);
    needsFrame = true;
  }

  const hoverState = (): string | null => {
    if (!doc) return null;
    if (opts.hoverState !== undefined) return opts.hoverState;
    return findState(doc, 'hover') ? 'hover' : null;
  };

  const ready = (async () => {
    const [three, orbitMod, gltfMod, meshopt] = await Promise.all([
      loadThree(),
      import('three/examples/jsm/controls/OrbitControls.js'),
      import('three/examples/jsm/loaders/GLTFLoader.js'),
      import('three/examples/jsm/libs/meshopt_decoder.module.js'),
    ]);
    if (destroyed) return;
    THREE = three;
    const loaded = opts.resolve?.scene ? await opts.resolve.scene(opts.src, opts.version ?? null) : await defaultScene(opts.src, opts.version ?? null);
    if (!loaded || destroyed) throw new Error('Nothing to embed');
    doc = loaded.doc;
    title = opts.label ?? loaded.title;
    color = (r, fb) => resolveColor(r, loaded.brand, fb);
    const T = THREE;

    renderer = new T.WebGLRenderer({ antialias: true, alpha: opts.background === 'transparent' });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    renderer.outputColorSpace = T.SRGBColorSpace;
    renderer.toneMapping = T.ACESFilmicToneMapping;
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = T.PCFSoftShadowMap;
    const canvas = renderer.domElement;
    Object.assign(canvas.style, { display: 'block', width: '100%', height: '100%', touchAction: opts.drag === false ? 'auto' : 'none' });
    canvas.setAttribute('role', 'img');
    canvas.setAttribute('aria-label', title ? `3D view: ${title}` : '3D view');
    el.appendChild(canvas);

    scene = new T.Scene();
    content = new T.Group();
    scene.add(content);
    if (doc.environment && doc.environment.preset !== 'none') {
      env = buildEnvironment(T, renderer, doc.environment);
      scene.environment = env.envMap;
      scene.environmentIntensity = doc.environment.intensity ?? 1;
      renderer.toneMappingExposure = env.preset.exposure;
    }
    if (opts.background !== 'transparent') {
      scene.background = doc.environment?.background !== false && env?.backdrop ? env.backdrop : new T.Color(doc.background ?? '#0f172a');
    }

    // Content: groups (organisational) + objects + lights.
    const parentOf = new Map<string, string>();
    for (const g of doc.groups) for (const c of g.children) parentOf.set(c, g.id);
    for (const g of doc.groups) {
      const gn = new T.Group();
      gn.visible = g.visible !== false;
      nodes.set(g.id, gn);
    }
    const loader = new gltfMod.GLTFLoader();
    loader.setMeshoptDecoder(meshopt.MeshoptDecoder);
    const resolveModel = opts.resolve?.model ?? resolveModelRef;
    const pending: Promise<void>[] = [];
    for (const o of doc.objects) {
      if (o.type === 'gltf') {
        const holder = new T.Group();
        applyTransform(holder, o);
        holder.visible = o.visible !== false;
        nodes.set(o.id, holder);
        const ref = o.src ?? o.attachment_id;
        if (ref) {
          pending.push(
            resolveModel(ref)
              .then((url) => loader.loadAsync(url))
              .then((g) => {
                if (destroyed) return;
                g.scene.traverse((n) => {
                  const m = n as THREE_NS.Mesh;
                  if (m.isMesh) m.castShadow = m.receiveShadow = true;
                });
                holder.add(g.scene);
                needsFrame = true;
              })
              .catch(() => {
                /* a missing model leaves a gap; the rest still renders */
              }),
          );
        }
      } else {
        nodes.set(o.id, buildMesh(T, o, color));
        baseMat.set(o.id, resolveMaterial(o.material, color));
      }
    }
    for (const [id, n] of nodes) {
      const pid = parentOf.get(id);
      (pid && nodes.get(pid) ? nodes.get(pid)! : content).add(n);
    }
    for (const l of doc.lights) {
      const { light, target } = buildLight(T, l);
      scene.add(light);
      if (target) scene.add(target);
    }

    camera = new T.PerspectiveCamera(doc.camera.fov, 1, doc.camera.near ?? 0.1, doc.camera.far ?? 500);
    if (opts.drag !== false) {
      const oc = new orbitMod.OrbitControls(camera, canvas);
      oc.enableZoom = false;
      oc.enablePan = false;
      oc.enableDamping = !reduced;
      oc.addEventListener('change', () => (needsFrame = true));
      controls = oc;
    }
    const hash = parseOttoUri(opts.src)?.node ?? null;
    setView(opts.view ?? (hash?.startsWith('view:') ? hash.slice(5) : null));
    goToState(hash?.startsWith('state:') ? hash.slice(6) : null);
    poses = statePoses(doc, shown, (c) => color(c, '#94a3b8'));
    applyPoses(poses);

    const resize = () => {
      if (!renderer || !camera) return;
      const w = Math.max(1, el.clientWidth);
      const h = Math.max(1, el.clientHeight);
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      needsFrame = true;
    };
    ro = new ResizeObserver(resize);
    ro.observe(el);
    resize();
    io = new IntersectionObserver((es) => {
      visible = es.some((e) => e.isIntersecting);
      if (visible) needsFrame = true;
    });
    io.observe(el);

    // Hover: tween to the hover state, lean toward the pointer.
    const onEnter = () => {
      const hs = hoverState();
      if (hs) goToState(hs);
    };
    const onLeave = () => {
      goToState(null);
      tilt.tx = tilt.ty = 0;
    };
    const onMove = (e: PointerEvent) => {
      if (!opts.tiltOnHover || reduced) return;
      const r = el.getBoundingClientRect();
      tilt.tx = ((e.clientY - r.top) / r.height - 0.5) * 0.25;
      tilt.ty = ((e.clientX - r.left) / r.width - 0.5) * 0.35;
    };
    if (opts.tiltOnHover || opts.hoverState) {
      el.addEventListener('pointerenter', onEnter);
      el.addEventListener('pointerleave', onLeave);
      el.addEventListener('pointermove', onMove);
      cleanups.push(() => {
        el.removeEventListener('pointerenter', onEnter);
        el.removeEventListener('pointerleave', onLeave);
        el.removeEventListener('pointermove', onMove);
      });
    }

    await Promise.race([Promise.all(pending), new Promise((r) => setTimeout(r, 4000))]);
    if (destroyed) return;
    const frame = (now: number) => {
      if (destroyed) return;
      raf = requestAnimationFrame(frame);
      const dt = last ? Math.min(0.1, (now - last) / 1000) : 0;
      last = now;
      if (!visible || !renderer || !scene || !camera || !content) return;
      if (tween) {
        const t = transitionProgress(now - tween.start, tween.duration, tween.easing);
        const done = now - tween.start >= tween.duration;
        poses = done ? tween.to : lerpPoses(tween.from, tween.to, t);
        applyPoses(poses);
        if (done) tween = null;
      }
      if (opts.autoRotate && !reduced && doc) {
        content.rotation.y += (doc.turntable?.speed ?? 20) * DEG * dt;
        needsFrame = true;
      }
      if (opts.tiltOnHover && !reduced) {
        tilt.x += (tilt.tx - tilt.x) * Math.min(1, dt * 8);
        tilt.y += (tilt.ty - tilt.y) * Math.min(1, dt * 8);
        if (Math.abs(tilt.x - content.rotation.x) > 1e-4 || Math.abs(tilt.tx) > 1e-4) {
          content.rotation.x = tilt.x;
          if (!opts.autoRotate) content.rotation.y = tilt.y;
          needsFrame = true;
        }
      }
      if (controls && !reduced) controls.update();
      if (!needsFrame) return;
      needsFrame = false;
      renderer.render(scene, camera);
      if (poster) {
        poster.remove();
        poster = null;
      }
    };
    raf = requestAnimationFrame(frame);
  })();
  // Failure keeps the poster; the caller decides whether to show an error.
  ready.catch(() => {});

  function destroy(): void {
    destroyed = true;
    cancelAnimationFrame(raf);
    for (const c of cleanups) c();
    io?.disconnect();
    ro?.disconnect();
    controls?.dispose();
    env?.dispose();
    if (content) disposeTree(content);
    if (renderer) {
      renderer.dispose();
      renderer.forceContextLoss();
      renderer.domElement.remove();
    }
    poster?.remove();
    renderer = null;
  }

  return {
    ready,
    setState: goToState,
    setView,
    info: () => ({
      title,
      views: (doc?.cameras ?? []).map((c) => ({ id: c.id, name: c.name ?? c.id })),
      states: (doc?.states ?? []).map((s) => ({ id: s.id, name: s.name ?? s.id })),
    }),
    destroy,
  };
}
