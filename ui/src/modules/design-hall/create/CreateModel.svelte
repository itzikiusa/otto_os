<script lang="ts" module>
  import type { Scene3dCamera } from '../../../lib/design/scene3d/types';
  /** Where the camera last was on a scene, with the scene's own camera then. */
  const lastViews = new Map<string, { view: Scene3dCamera; cam: Scene3dCamera }>();
</script>

<script lang="ts">
  // The thing itself, large: one made model (a GLB artifact, at a version) or
  // a whole scene (a scene3d artifact), on the shared Scene3DViewport —
  // orbit by drag or touch, framed on the subject, moves played from the
  // model's own clips, and taps turned into numbered pins when `pickable`.
  // Every studio control (tool bar, stats, grid) stays hidden: nothing here
  // needs a word of 3D vocabulary.
  import { onDestroy, untrack } from 'svelte';
  import Scene3DViewport from '../../../lib/design/scene3d/Scene3DViewport.svelte';
  import { emptyScene, type Scene3dDoc } from '../../../lib/design/scene3d/types';
  import { parseScene } from '../../../lib/design/scene3d/validate';
  import type { Pin3dAnchor, ProjectedPin3d, RigInfo } from '../../../lib/design/scene3d';
  import type { Pin3dInput } from '../../../lib/design/scene3d/viewportInspection';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { fetchContent, putThumbnail } from '../../../lib/api/design';
  import { loadErrorText } from '../../../lib/loadError';
  import { modelScope } from '../studio3d/sources';

  interface Props {
    artifactId: string;
    /** `model` = one GLB; `scene` = a scene3d document of made models. */
    as?: 'model' | 'scene';
    /** Version sequence to show (default: the newest). */
    seq?: number | null;
    label: string;
    pickable?: boolean;
    pins?: Pin3dInput[];
    clipIndex?: number;
    playing?: boolean;
    /** Draw the skeleton's bones over the model (they follow the pose while a move plays). */
    skeleton?: boolean;
    onpick?: (anchor: Pin3dAnchor) => void;
    onrig?: (info: RigInfo) => void;
    /** The parsed scene (so a caller can map a tapped object to its item). */
    onscene?: (doc: Scene3dDoc) => void;
    /** Save a picture of the view as this artifact's thumbnail once everything has loaded. */
    thumbnail?: boolean;
    /** What the remembered camera of a scene belongs to (default: the artifact). */
    viewKey?: string;
    /** A scene object to outline (the one speaking while the scene plays). */
    highlight?: string | null;
  }
  let { artifactId, as = 'model', seq = null, label, pickable = false, pins = [], clipIndex = 0, playing = false, skeleton = false, onpick, onrig, onscene, thumbnail = false, viewKey, highlight = null }: Props = $props();

  const scope = modelScope();
  onDestroy(() => scope.release());
  let doc = $state<Scene3dDoc | null>(null);
  let error = $state<string | null>(null);
  let projected = $state<ProjectedPin3d[]>([]);
  let selectedId = $state<string | null>(null);
  let attempt = $state(0);
  /** Where each pin was tapped (as a share of the picture), for when the 3D point can't be placed on screen. */
  let taps = $state<Record<string, { x: number; y: number }>>({});
  let lastTap: { x: number; y: number } | null = null;
  function onDown(e: PointerEvent): void {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    lastTap = { x: (e.clientX - r.left) / Math.max(1, r.width), y: (e.clientY - r.top) / Math.max(1, r.height) };
  }
  /** A pin is always a whole marker inside the picture (CSS clamps it in): its 3D place when that is in view, else where it was tapped. */
  const shown = $derived(
    pins.map((pin) => {
      const p = projected.find((x) => x.id === pin.id);
      const t = taps[pin.id];
      const x = p?.visible ? `${p.x.toFixed(1)}px` : `${((t?.x ?? 0.5) * 100).toFixed(1)}%`;
      const y = p?.visible ? `${p.y.toFixed(1)}px` : `${((t?.y ?? 0.5) * 100).toFixed(1)}%`;
      return { id: pin.id, n: pin.n, x: `clamp(24px, ${x}, calc(100% - 24px))`, y: `clamp(40px, ${y}, calc(100% - 10px))` };
    }),
  );
  // Remember where a new pin was tapped.
  $effect(() => {
    for (const pin of pins) if (!taps[pin.id] && lastTap) taps = { ...taps, [pin.id]: lastTap };
  });
  let viewport = $state<{ snapshotPng: () => Promise<Blob | null>; currentView: () => Scene3dCamera; goToView: (c: Scene3dCamera) => void } | null>(null);

  // A scene shown again (after a change: a new version, its pieces re-laid) opens where the
  // camera was, stepped back only as much as the group grew, so a bigger monster looks bigger
  // instead of everything else looking smaller.
  let restored = false;
  /** The view is still the viewport's own framing (no remembered view was put back). */
  let auto = true;
  function remember(): void {
    if (as === 'scene' && doc && viewport) lastViews.set(viewKey ?? artifactId, { view: viewport.currentView(), cam: doc.camera });
  }
  function restore(): void {
    if (restored || as !== 'scene' || !doc || !viewport) return;
    restored = true;
    const last = lastViews.get(viewKey ?? artifactId);
    if (!last) return;
    auto = false;
    const len = (c: Scene3dCamera) => Math.hypot(...c.position.map((v, k) => v - c.target[k])) || 1;
    const k = len(doc.camera) / len(last.cam);
    const target = last.view.target.map((v, i) => v + doc!.camera.target[i] - last.cam.target[i]) as Scene3dCamera['target'];
    const position = last.view.position.map((v, i) => target[i] + (v - last.view.target[i]) * k) as Scene3dCamera['position'];
    viewport.goToView({ position, target, fov: last.view.fov });
    lastViews.set(viewKey ?? artifactId, { view: { position, target, fov: last.view.fov }, cam: doc.camera });
  }

  /** The page's own surface colour, so the picture sits in the theme (light or dark). */
  function surface(): string {
    const v = getComputedStyle(document.documentElement).getPropertyValue('--surface-2').trim();
    return /^#[0-9a-f]{3,8}$/i.test(v) ? v : '#f0f0f4';
  }

  // The viewport frames the subject itself (frameModel: every piece stands on
  // the ground and the camera fits them all). Once the last piece has loaded,
  // save a picture of the view as the thumbnail.
  let seenPieces = 0;
  let shotTimer: ReturnType<typeof setTimeout> | null = null;
  function loaded(pieces: number): void {
    restore();
    if (auto) remember();
    if (!thumbnail || pieces <= seenPieces) return;
    seenPieces = pieces;
    if (shotTimer) clearTimeout(shotTimer);
    shotTimer = setTimeout(() => void saveThumb(), 1500);
  }
  async function saveThumb(): Promise<void> {
    const png = await viewport?.snapshotPng().catch(() => null);
    if (png && png.size < 2_000_000) await putThumbnail(artifactId, png).catch(() => undefined);
  }
  onDestroy(() => shotTimer && clearTimeout(shotTimer));

  /** A lone model on a soft studio floor, framed by the viewport. */
  function modelDoc(): Scene3dDoc {
    return {
      ...emptyScene(),
      background: surface(),
      grid: false,
      environment: { preset: 'studio-soft', background: false },
      camera: { position: [2.4, 1.6, 3.2], target: [0, 0.8, 0], fov: 35 },
      lights: [{ id: 'key', type: 'directional', position: [3, 5, 2], intensity: 1.2, shadow: true }],
      objects: [
        {
          id: 'model',
          name: label,
          type: 'gltf',
          src: `otto://design/${artifactId}@${seq ? `v${seq}` : 'latest'}`,
          position: [0, 0, 0],
          rotation: [0, 0, 0],
          scale: [1, 1, 1],
        },
      ],
    } as Scene3dDoc;
  }

  $effect(() => {
    void artifactId;
    void seq;
    void as;
    void attempt;
    untrack(() => void load());
  });

  async function load(): Promise<void> {
    error = null;
    // A new scene or version is a new picture: its thumbnail is taken again.
    seenPieces = 0;
    if (as === 'model') {
      doc = modelDoc();
      return;
    }
    try {
      const c = await fetchContent(artifactId, { version: seq ? `v${seq}` : undefined, asText: true });
      const r = parseScene(c.text ?? '');
      if (!r.ok) throw new Error('The scene could not be read');
      // A scene with a setting shows its own sky; any other sits on the page's surface.
      doc = r.doc.environment?.sky ? { ...r.doc, grid: false } : { ...r.doc, grid: false, background: surface(), environment: { ...(r.doc.environment ?? { preset: 'studio-soft' }), background: false } };
      onscene?.(doc);
    } catch (e) {
      error = loadErrorText(e);
    }
  }

  // A plain look has no selection outline (a tap that selected something is undone): only the
  // one speaking while the scene plays is outlined.
  $effect(() => {
    if (selectedId !== highlight) selectedId = highlight;
  });
</script>

<div class="cm" class:pickable data-testid="create-model" onpointerdowncapture={onDown} onpointerupcapture={remember}>
  {#if error}
    <div class="cm-error"><LoadState what="the picture" errorTitle="I couldn’t show this right now" {error} onretry={() => attempt++} variant="compact" /></div>
  {:else if doc}
    <!-- Retry (or the graphics processor giving the view back) builds the view again. -->
    {#key attempt}
    <Scene3DViewport
      bind:this={viewport}
      {doc}
      readonly
      compact
      frameModel
      bind:selectedId
      cameraButton={false}
      onchange={() => {}}
      resolveAttachment={scope.resolve}
      pickMode={pickable}
      pickKind="point"
      pin3dInputs={pins}
      animClipIndex={clipIndex}
      animPlaying={playing}
      showSkeleton={skeleton}
      onpolygonpick={(a) => onpick?.(a)}
      onprojectedpins={(p) => (projected = p)}
      onriginfo={(i) => onrig?.(i)}
      ontopology={(meshes) => loaded(meshes.length)}
      onretry={() => attempt++}
    />
    {/key}
    <div class="cm-pins" aria-hidden="true">
      {#each shown as p (p.id)}
        <span class="cm-pin" data-testid="create-pin" style:inset-inline-start={p.x} style:inset-block-start={p.y}><b>{p.n}</b></span>
      {/each}
    </div>
  {/if}
  <span class="sr">{label}</span>
</div>

<style>
  .cm {
    position: relative;
    inline-size: 100%;
    block-size: 100%;
    min-block-size: 240px;
    border-radius: var(--radius-l);
    overflow: hidden;
    background: var(--surface-2);
  }
  .cm :global(.s3d-hud),
  .cm :global(.s3d-status) {
    display: none;
  }
  .cm :global(.s3d-viewport) {
    border-radius: 0;
  }
  .cm-pins {
    position: absolute;
    inset: 0;
    z-index: 2;
    pointer-events: none;
    direction: ltr;
  }
  .cm-pin {
    position: absolute;
    translate: -15px -30px;
    inline-size: 30px;
    block-size: 30px;
    display: grid;
    place-items: center;
    border-radius: 50% 50% 50% 0;
    rotate: -45deg;
    background: var(--danger-solid);
    color: var(--danger-contrast);
    font-weight: 600;
    font-size: var(--fs-m);
    box-shadow: var(--shadow-card);
  }
  .cm-pin b {
    rotate: 45deg;
  }
  .cm-error {
    display: grid;
    place-items: center;
    gap: var(--sp-3);
    block-size: 100%;
    padding: var(--sp-6);
    text-align: center;
  }
  .sr {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
