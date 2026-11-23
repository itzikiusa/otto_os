<script lang="ts">
  import { announceShown } from '../../../lib/announce.svelte';
  import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
  import { toastError } from '../../../lib/toastError';
  import { copyWithToast } from '../../../lib/clipboard';
  import { untrack, type Snippet } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { registry } from '../../../lib/commands.svelte';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { api, ApiError } from '../../../lib/api/client';
  import { downloadText } from '../../../lib/components/exporters';
  import * as dapi from '../../../lib/api/design';
  import * as mapi from '../../../lib/api/media';
  import { listTurns, startAssist as startTurn } from '../assist/api';
  import { designAssistBus } from '../../../lib/events.svelte';
  import { loadErrorText as errText } from '../../../lib/loadError';
  import { blenderSourceOf, downloadBlenderScript } from '../exportMenu';
  import type {
    CommentPin3d,
    DesignArtifact,
    DesignAssistTurn,
    DesignVersion,
    Media3dPipelineStage,
    Media3dProvider,
  } from '../../../lib/api/types';
  import {
    EMPTY_RIG_INFO,
    Inspector,
    Scene3DViewport,
    addCameraPreset,
    addGltfSrc,
    editTargetState,
    emptyScene,
    exportObjectToGlb,
    glbFileName,
    initialState,
    mergeFaceIntoSelection,
    parseScene,
    resolveColor,
    removeCameraPreset,
    serializeScene,
    setBrand,
    updateCameraPreset,
    type BrandSwatch,
    type EnvPresetId,
    type MeshTopologyStats,
    type Pin3dAnchor,
    type ProjectedPin3d,
    type RigInfo,
    type Scene3dDoc,
    type SelectionKind3d,
    type ShadingMode,
    type SubMeshInfo,
  } from '../../../lib/design/scene3d';
  import type { ColorResolver } from '../../../lib/design/scene3d/build';
  import { colorTokens, resolveToken, tokenRef } from '../brand/tokens';
  import ArtifactStage from '../ArtifactStage.svelte';
  import { isTypingTarget, pins3dFor } from '../comments/pins';
  import type { CommentsStore } from '../comments/commentsStore.svelte';
  import AnimationBar from './AnimationBar.svelte';
  import StatesBar from './StatesBar.svelte';
  import type { OptimizeState } from './OptimizeModal.svelte';
  import Pin3DLayer from './Pin3DLayer.svelte';
  import Studio3dLeftPanel from './Studio3dLeftPanel.svelte';
  import Studio3dModals from './Studio3dModals.svelte';
  import Studio3dToolbar from './Studio3dToolbar.svelte';
  import { anchorEventNearLeftPanel, bytesToB64, exportViewportAs, listImportable3dModels, renderAndPutThumbnail, saveBlob, uploadGlbFileArtifact } from './studio3dExport';
  import { blenderRefinePrompt, blockoutPrompt, type Gen3dContext, type Gen3dKind } from './providers';
  import { loadBrandKit, modelScope, type BrandKit } from './sources';
  import { openArtifact } from '../nav';
  import { library } from '../library.svelte';
  import { policyLabel, type LinkRow } from '../model';
  import { turnOutcome, type AssistSelection } from '../assist/model';

  interface Props {
    artifact: DesignArtifact;
    source: string | null;
    /** Object URL when viewing a standalone `.glb`/`.gltf`/`.obj` model artifact. */
    modelBlobUrl?: string | null;
    readonly: boolean;
    dirty: boolean;
    head: DesignVersion | null;
    shownVersionId?: string | null;
    usedIn: LinkRow[];
    linkCount: number;
    seqOf: (versionId: string) => number | null;
    onchange: (source: string) => void;
    comments?: CommentsStore;
    commentCount?: number | null;
    commentsPanel?: Snippet<[{ onfocus3d: (pin: CommentPin3d) => void; onregenerate3d: (prompt: string) => void }]>;
    otto: Snippet<[AssistSelection | null]>;
    links: Snippet;
    versions: Snippet;
    openOtto?: boolean;
    notices?: Snippet;
  }
  let {
    artifact,
    source,
    modelBlobUrl = null,
    readonly,
    dirty,
    head,
    shownVersionId = null,
    usedIn,
    linkCount,
    seqOf,
    onchange,
    comments,
    commentCount = null,
    commentsPanel,
    otto,
    links,
    versions,
    notices,
    openOtto = false,
  }: Props = $props();

  // ── Document (scene3d JSON or synthetic wrapper for standalone GLB/glTF/OBJ) ─
  const isModelArtifact = $derived(artifact.format === 'glb' || artifact.format === 'gltf' || artifact.format === 'obj');
  let modelEnvPreset = $state<EnvPresetId>('studio-soft');
  const parsed = $derived(!isModelArtifact && source ? parseScene(source) : null);
  const doc = $derived.by<Scene3dDoc | null>(() => {
    if (isModelArtifact) {
      const base = emptyScene();
      return {
        ...base,
        environment: { preset: modelEnvPreset, background: true, intensity: 1 },
        // No 20 m floor slab under a standalone model: the grid marks the ground and the slab
        // hid whatever of the model (an animated character's legs) dipped below it.
        objects: [
          { id: 'model', name: artifact.title, type: 'gltf', attachment_id: artifact.id, position: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1] },
        ],
      };
    }
    return parsed?.ok ? parsed.doc : null;
  });

  let selectedId = $state<string | null>(null);
  let stateId = $state<string | null>(null);
  let play = $state(false);
  let showSource = $state(false);
  let showSceneTools = $state(false);
  let showDetailsDrawer = $state(false);
  let turntable = $state(false);
  // ── 3D Inspection & Polygon Commenting ────────────────────────────────────
  let shading = $state<ShadingMode>('shaded');
  let wireframeOverlay = $state(false);
  let scaleRef = $state(false);
  let pickMode = $state(false);
  let pickKind = $state<SelectionKind3d>('polygon');
  let hoveredAnchor = $state<Pin3dAnchor | null>(null);
  let draftAnchor = $state<Pin3dAnchor | null>(null);
  let projectedPins = $state<ProjectedPin3d[]>([]);
  let subMeshes = $state<SubMeshInfo[]>([]);
  let topologyStats = $state<MeshTopologyStats>({ triangles: 0, vertices: 0, subMeshes: 0, materials: 0, bounds: null });
  let activeMeshUid = $state<string | null>(null);
  let providersOpen = $state(false);
  let exportWsOpen = $state(false);
  let genInitialPrompt = $state('');
  let genStage = $state<Media3dPipelineStage>('generate');
  let rigInfo = $state<RigInfo>(EMPTY_RIG_INFO);
  let showSkeleton = $state(false);
  let animClipIndex = $state(0);
  let animPlaying = $state(false);
  let mediaProviders = $state<Media3dProvider[]>([]);
  const stageProviders = $derived(
    mediaProviders.filter(
      (p) => p.enabled && p.ready && p.capabilities?.supported_stages?.some((s) => s !== 'generate'),
    ),
  );
  $effect(() => {
    if (!isModelArtifact) return;
    untrack(() => {
      void mapi.list3dProviders().then((ps) => { if (!destroyed) mediaProviders = ps; }, () => {});
    });
  });

  const canComment = $derived(auth.can('design', 'edit'));
  const canAdmin3d = $derived(auth.can('design', 'admin'));
  const effectiveVersionId = $derived(shownVersionId ?? head?.id ?? null);

  type RightTab = 'inspector' | 'comments' | 'otto' | 'versions' | 'links';
  const rightTabs = $derived<TabItem<RightTab>[]>([
    ...(!isModelArtifact ? [{ id: 'inspector' as const, label: 'Inspector', testid: 's3d-tab-inspector' }] : []),
    ...(commentsPanel ? [{ id: 'comments' as const, label: 'Comments', count: commentCount ?? undefined, testid: 'design-tab-comments' }] : []),
    { id: 'otto', label: 'Otto', testid: 'design-tab-otto' },
    { id: 'versions', label: 'Versions', testid: 'design-tab-versions' },
    { id: 'links', label: 'Links', count: linkCount, testid: 'design-tab-links' },
  ]);
  let rightTab = $state<RightTab>(untrack(() => (openOtto ? 'otto' : isModelArtifact && commentsPanel ? 'comments' : 'inspector')));
  $effect(() => {
    if (openOtto) untrack(() => { rightTab = 'otto'; showDetailsDrawer = true; });
  });
  let vp = $state<ReturnType<typeof Scene3DViewport> | null>(null);
  let vpKey = $state(0); // bumped by the viewport's Retry: a fresh renderer
  let studioW = $state(0); // ≤ 980: one side pane at a time (with both the canvas would be a sliver)

  const pin3dInputs = $derived.by(() => {
    if (!comments) return [];
    return pins3dFor(comments.threads, effectiveVersionId).map(({ thread, muted }) => ({
      id: thread.root.id,
      n: thread.n,
      body: thread.root.body,
      muted,
      anchor: thread.root.pin_3d!,
    }));
  });
  const activeAnchor = $derived.by<Pin3dAnchor | null>(() => {
    if (draftAnchor) return draftAnchor;
    if (!comments?.selectedId) return null;
    const hit = comments.threads.find((t) => t.root.id === comments.selectedId);
    return hit?.root.pin_3d ?? null;
  });

  function onPolygonPick(hit: Pin3dAnchor, additive: boolean): void {
    draftAnchor = additive ? mergeFaceIntoSelection(draftAnchor, hit) : hit;
  }
  function onSelectPin3d(id: string): void {
    if (!comments) return;
    comments.selectedId = id;
    rightTab = 'comments';
    showDetailsDrawer = true;
    const hit = comments.threads.find((t) => t.root.id === id);
    if (hit?.root.pin_3d) vp?.focusAnchor(hit.root.pin_3d);
  }
  function onFocus3d(pin: CommentPin3d): void {
    draftAnchor = null;
    vp?.focusAnchor(pin);
  }
  function onRegenerate3d(feedbackPrompt: string): void {
    genStage = 'generate';
    genInitialPrompt = feedbackPrompt;
    genKind = 'text';
  }
  function openStage(stage: Media3dPipelineStage): void {
    genStage = stage;
    genInitialPrompt = '';
    genKind = 'text';
  }
  /** Pipeline stages for a model artifact (in the toolbar's ⋯), when a ready provider offers one. */
  const pipelineItems = $derived.by<MenuItem[]>(() => {
    if (!isModelArtifact || !stageProviders.length) return [];
    const has = (s: Media3dPipelineStage) => stageProviders.some((p) => p.capabilities.supported_stages.includes(s));
    const off = !auth.can('design', 'edit');
    return [
      { label: 'Make game-ready (retopo / bake)…', icon: 'zap', disabled: off || !has('gameready'), action: () => openStage('gameready') },
      { label: 'Rig skeleton…', icon: 'layers', disabled: off || !has('rig'), action: () => openStage('rig') },
      { label: 'Animate model…', icon: 'play', disabled: off || !has('animate'), action: () => openStage('animate') },
    ];
  });
  function onCommentSubMesh(m: SubMeshInfo): void {
    if (!canComment) return;
    activeMeshUid = m.uid;
    vp?.focusMesh(m.uid);
    const a = vp?.anchorForMesh(m.uid);
    if (a) {
      pickMode = true;
      draftAnchor = a;
    }
  }
  // C toggles 3D Polygon Comment mode; Esc exits it.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || isTypingTarget(e.target)) return;
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      if ((e.key === 'c' || e.key === 'C') && canComment && !e.shiftKey) {
        e.preventDefault();
        pickMode = !pickMode;
        if (!pickMode) draftAnchor = null;
      } else if (e.key === 'Escape' && (pickMode || draftAnchor)) {
        pickMode = false;
        draftAnchor = null;
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function resolveModelRef(ref: string): Promise<string> {
    if (isModelArtifact && ref === artifact.id && modelBlobUrl) return Promise.resolve(modelBlobUrl);
    return models.resolve(ref);
  }

  // Keep the state picker on a real state (the doc may gain/lose states).
  $effect(() => {
    const d = doc;
    if (!d) return;
    untrack(() => {
      if (!stateId || !d.states?.some((s) => s.id === stateId)) stateId = initialState(d);
    });
  });

  function edit(next: Scene3dDoc): void {
    if (readonly) return;
    // Brand colours need a kit to resolve against: the first token pins the
    // scene to the kit it was picked from (a `uses_tokens` link on save).
    let d = next;
    if (!d.brand && kit && JSON.stringify(d.objects).includes('"token:color.')) d = setBrand(d, kit.uri);
    onchange(serializeScene(d));
  }

  // ── Brand kit (token colours + swatches) ──────────────────────────────────
  let kit = $state<BrandKit | null>(null);
  let kitKey = '';
  $effect(() => {
    const key = `${artifact.id}|${doc?.brand ?? ''}`;
    if (!doc || key === kitKey) return;
    kitKey = key;
    const brandUri = doc.brand;
    void loadBrandKit(brandUri, artifact, library.projectOf(artifact.project_id)?.brand_kit_id ?? null).then(
      (k) => {
        if (kitKey === key) kit = k;
      },
      () => {
        if (kitKey === key) kit = null;
      },
    );
  });
  // Brand Kit owns token resolution: `token:color.<name>` → the kit's hex.
  const swatches = $derived<BrandSwatch[]>(
    colorTokens(kit?.doc).map((c) => ({ name: c.name, value: c.hex.toLowerCase(), ref: tokenRef('color', c.name) })),
  );
  const colors = $derived.by<ColorResolver>(() => {
    const kd = kit?.doc;
    return (ref, fallback) => resolveColor(ref, kd ? (r) => resolveToken(kd, r) : null, fallback);
  });

  // ── Agent turns (Blockout / Text→3D / Image→3D / Refine in Blender) ───────
  let agent = $state<{ turnId: string; label: string; fails: number; error: string | null } | null>(null);
  // Set on unmount: a read in flight must not toast from a dead view.
  let destroyed = false;
  function track(turn: DesignAssistTurn, label: string): void {
    agent = { turnId: turn.turn_id, label, fails: 0, error: null };
    toasts.info('Otto is working on it', `${label}. The result lands as a new agent version you can review.`);
  }
  // Pushed, not polled: the Otto panel's `designAssistBus` says when the turn
  // ends (or the socket reconnects); the outcome is then read once.
  let assistSeen = designAssistBus.seq;
  $effect(() => {
    const now = designAssistBus.seq;
    void designAssistBus.resyncTick;
    untrack(() => {
      const evs = designAssistBus.since(assistSeen);
      assistSeen = now;
      if (agent && (evs.length === 0 || evs.some((ev) => ev.type === 'design_assist_updated' && ev.turn_id === agent!.turnId))) void poll();
    });
  });
  async function poll(): Promise<void> {
    const a = agent;
    if (!a) return;
    try {
      const turns = await listTurns(artifact.id);
      if (destroyed || agent?.turnId !== a.turnId) return;
      const out = turnOutcome(turns.find((x) => x.turn_id === a.turnId));
      if (out.kind === 'wait') return void (agent = { ...a, fails: 0, error: null });
      agent = null;
      toasts[out.kind](out.title, out.body);
    } catch (e) {
      // A failed read is retried on the next event; after two in a row say so inline.
      if (!destroyed && agent?.turnId === a.turnId) agent = { ...a, fails: a.fails + 1, error: a.fails + 1 >= 2 ? errText(e) : null };
    }
  }
  $effect(() => () => {
    destroyed = true;
    models.release();
  });
  /** This editor’s hold on its model blobs (released on unmount, above). */
  const models = modelScope();

  const genCtx = $derived<Gen3dContext>({ cloudKeys: {}, canEdit: !readonly, busy: !!agent, dirty });
  let genKind = $state<Gen3dKind | null>(null);

  async function startAssist(prompt: string, mode: 'generate' | 'refine', label: string): Promise<void> {
    try {
      track(await startTurn(artifact.id, { prompt, mode }), label);
    } catch (e) {
      const msg = e instanceof ApiError && e.status === 409 ? 'Otto is already working on this design.' : errText(e);
      toasts.error('Couldn’t start Otto', msg);
    }
  }

  function guardAgent(): boolean {
    if (readonly) return false;
    if (agent) {
      toasts.info('Otto is already working on this design', agent.label);
      return false;
    }
    if (dirty) {
      toasts.warn('Save your edits first', 'Otto’s result lands as a new version on top of the saved scene.');
      return false;
    }
    return true;
  }

  async function blockout(): Promise<void> {
    if (!guardAgent()) return;
    const p = await confirmer.promptText(
      'Describe the scene. Otto writes a fresh scene3d blockout as a new agent version (the current one stays in history).',
      { title: 'Blockout from prompt', confirmLabel: 'Generate blockout', placeholder: 'A rewards card on a plinth, soft studio light, hero angle' },
    );
    if (p) await startAssist(blockoutPrompt(p), 'generate', 'Blocking out the scene');
  }

  async function refineInBlender(): Promise<void> {
    if (!guardAgent()) return;
    let blender = 'Blender detection unavailable';
    try {
      const s = await api.get<{ installed: boolean; version: string | null }>('/product/design/blender');
      blender = s.installed ? `Blender ${s.version ?? ''} is installed.`.replace('  ', ' ') : 'Blender isn’t installed — Otto will refine the scene file directly.';
    } catch {
      /* keep the neutral line */
    }
    const p = await confirmer.promptText(
      `${blender} Otto runs one agent turn that may drive Blender through the Blender MCP server (MCP → Blender template). ` +
        'Every Blender MCP tool call asks for your approval first, and the Blender Foundation notes it runs model-generated code without guards. ' +
        'The result lands as a new agent version. What should change?',
      { title: 'Refine in Blender (MCP)', confirmLabel: 'Start refine', placeholder: 'Softer edges on the card, add a subtle chip emboss' },
    );
    if (p) await startAssist(blenderRefinePrompt(p), 'refine', 'Refining in Blender');
  }

  function generateMenu(e: MouseEvent): void {
    const canGen = auth.can('design', 'edit');
    const items: MenuItem[] = [
      { label: 'Generate 3D asset…', title: 'Otto Forge on this Mac; other generators are under Advanced', icon: 'sparkle', disabled: !canGen, action: () => { genInitialPrompt = ''; genKind = 'text'; } },
      { label: 'Image → 3D model…', icon: 'image', disabled: !canGen, action: () => { genInitialPrompt = ''; genKind = 'image'; } },
      ...(!isModelArtifact ? [{ label: 'Blockout from prompt (scene JSON)…', icon: 'box' as const, disabled: readonly, action: () => void blockout() }] : []),
      { separator: true },
      { label: 'Manage 3D providers…', icon: 'gear', action: () => (providersOpen = true) },
      ...(!isModelArtifact ? [{ label: 'Refine in Blender (MCP)…', icon: 'layers' as const, disabled: readonly, action: () => void refineInBlender() }] : []),
    ];
    ctxMenu.show(e, items);
  }

  // ── Export ────────────────────────────────────────────────────────────────
  async function exportAs(kind: 'glb' | 'usdz' | 'png' | 'sheet'): Promise<void> {
    if (!vp) return;
    try {
      await exportViewportAs(vp, artifact.title, kind);
    } catch (e) {
      toastError('Couldn’t export the model', e);
    }
  }
  function exportItems(): MenuItem[] {
    return [
      ...(isModelArtifact ? [{ label: 'Export to Workspace…', icon: 'folder' as const, disabled: !auth.can('design', 'edit'), action: () => (exportWsOpen = true) }, { separator: true as const }] : []),
      { label: 'GLB (.glb)', icon: 'download', action: () => void exportAs('glb') },
      { label: 'USDZ (.usdz) — AR Quick Look', icon: 'download', action: () => void exportAs('usdz') },
      { label: 'PNG snapshot', icon: 'image', action: () => void exportAs('png') },
      { label: 'PNG turntable (8 frames)', icon: 'image', action: () => void exportAs('sheet') },
      ...(!isModelArtifact ? [{ label: 'Scene JSON', icon: 'file' as const, disabled: !source, action: () => source && downloadText(source, `${glbFileName(artifact.title).replace(/\.glb$/, '')}.scene3d.json`, 'application/json') }] : []),
      { separator: true },
      { label: 'Optimize for web (meshopt)…', icon: 'zap', action: () => void optimize() },
      ...(blenderSourceOf(artifact) ? [{ label: 'Blender script (.py)', icon: 'file' as const, action: () => void downloadBlenderScript(artifact) }] : []),
    ];
  }

  let opt = $state<OptimizeState | null>(null);
  let savingGlb = $state(false);
  async function optimize(): Promise<void> {
    if (!vp) return;
    opt = { phase: 'running' };
    try {
      const root = vp.contentRoot();
      if (!root) throw new Error('The viewport isn’t ready yet');
      const glb = await exportObjectToGlb(root);
      const { optimizeGlb } = await import('../../../lib/design/scene3d/optimize');
      const result = await optimizeGlb(await glb.arrayBuffer());
      opt = { phase: 'done', result };
    } catch (e) {
      opt = { phase: 'error', error: errText(e) };
    }
  }
  async function saveOptimized(): Promise<void> {
    const r = opt?.result;
    if (!r || savingGlb) return;
    savingGlb = true;
    try {
      const res = await dapi.createArtifact({
        workspace_id: artifact.workspace_id,
        project_id: artifact.project_id ?? undefined,
        studio: '3d',
        format: 'glb',
        title: `${artifact.title} (web)`,
        content_b64: bytesToB64(r.bytes),
        derived_from: head ? { artifact_id: artifact.id, version_id: head.id } : { artifact_id: artifact.id },
        message: `Optimized for web from v${head?.seq ?? '?'} (${r.compression})`,
      });
      toasts.success('Saved the web GLB', `${res.artifact.title} — embed it as otto://design/${res.artifact.id}@approved`);
      opt = null;
    } catch (e) {
      toastError('Couldn’t save the GLB', e);
    } finally {
      savingGlb = false;
    }
  }

  // ── Models (Hierarchy → Import) ───────────────────────────────────────────
  let glbInput = $state<HTMLInputElement | null>(null);
  async function importModel(): Promise<void> {
    if (!doc || readonly) return;
    const list = await listImportable3dModels(artifact);
    const items: MenuItem[] = [{ label: 'Upload GLB / glTF…', icon: 'arrowUp', pinned: true, action: () => glbInput?.click() }];
    if (list.length) items.push({ separator: true });
    for (const m of list) {
      items.push({
        label: m.title,
        icon: 'box',
        action: () => {
          const r = addGltfSrc(doc!, `otto://design/${m.id}@${m.approved_version_id ? 'approved' : 'latest'}`, { name: m.title, position: [1.4, 0, 0] });
          edit(r.doc);
          selectedId = r.id;
        },
      });
    }
    ctxMenu.show(anchorEventNearLeftPanel(), items, { filter: list.length > 8, maxVisible: 12 });
  }
  async function uploadModel(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const f = input.files?.[0];
    input.value = '';
    if (!f || !doc) return;
    try {
      const created = await uploadGlbFileArtifact(f, artifact);
      const r = addGltfSrc(doc, `otto://design/${created.id}@latest`, { name: created.title, position: [1.4, 0, 0] });
      edit(r.doc);
      selectedId = r.id;
    } catch (err) {
      toastError('Couldn’t import the model', err);
    }
  }

  // ── Views (named cameras) ─────────────────────────────────────────────────
  async function saveView(): Promise<void> {
    if (!doc || !vp) return;
    const name = await confirmer.promptText('Name this camera view. Site embeds can ask for it as #view:<id>.', {
      title: 'Save view',
      confirmLabel: 'Save view',
      placeholder: doc.cameras?.length ? 'Detail angle' : 'Hero angle',
    });
    if (!name) return;
    edit(addCameraPreset(doc, name, vp.currentView()).doc);
  }
  function viewMenu(e: MouseEvent, id: string): void {
    if (!doc) return;
    const cam = doc.cameras?.find((c) => c.id === id);
    if (!cam) return;
    ctxMenu.show(e, [
      { label: 'Go to view', icon: 'eye', action: () => vp?.goToView(cam) },
      { label: 'Update to the current view', icon: 'refresh', disabled: readonly, action: () => vp && edit(updateCameraPreset(doc!, id, vp.currentView())) },
      {
        label: 'Rename…',
        icon: 'edit',
        disabled: readonly,
        action: async () => {
          const n = await confirmer.promptText('View name', { title: 'Rename view', confirmLabel: 'Rename', initial: cam.name ?? cam.id });
          if (n) edit(updateCameraPreset(doc!, id, { name: n }));
        },
      },
      {
        label: 'Copy embed reference',
        icon: 'link',
        action: () => void copyWithToast(`otto://design/${artifact.id}@approved#view:${id}`, 'embed reference'),
      },
      { separator: true },
      {
        label: 'Delete view…',
        icon: 'trash',
        danger: true,
        disabled: readonly,
        action: async () => {
          const ok = await confirmer.ask(`Delete the view “${cam.name ?? cam.id}”? Embeds that point at #view:${id} fall back to the default camera.`, {
            title: 'Delete view',
            confirmLabel: 'Delete',
            danger: true,
          });
          if (ok) edit(removeCameraPreset(doc!, id));
        },
      },
    ]);
  }

  // ── Thumbnails: a rendered frame after each save keeps Design Hall cards real ──
  let thumbFor = '';
  let thumbTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const hid = head?.id ?? '';
    const want = !readonly && !dirty && !!hid && !!vp;
    if (!want || hid === thumbFor) return;
    // First open without a stored thumbnail, or a fresh save: render + store once.
    const missing = !artifact.thumb_blob;
    const fresh = thumbFor !== '';
    thumbFor = hid;
    if (!missing && !fresh) return;
    if (thumbTimer) clearTimeout(thumbTimer);
    thumbTimer = setTimeout(() => { if (vp) void renderAndPutThumbnail(vp, artifact.id); }, 1200);
  });
  $effect(() => () => {
    if (thumbTimer) clearTimeout(thumbTimer);
  });

  // ── Derived view bits ─────────────────────────────────────────────────────
  const agentDraft = $derived(!!head && head.author_kind === 'agent' && artifact.approved_version_id !== head.id);
  const statusNote = $derived(dirty ? 'edited' : agentDraft && head ? `draft v${head.seq}` : head ? 'saved' : '');
  const editState = $derived(doc && selectedId ? editTargetState(doc, stateId, selectedId) : null);
  const hoverState = $derived(doc?.states?.find((s) => s.id === 'hover')?.id ?? null);
  /** What an Otto turn focuses on: the selected object (or light / group). */
  const assistSelection = $derived.by<AssistSelection | null>(() => {
    if (!doc || !selectedId) return null;
    const n = doc.objects.find((o) => o.id === selectedId) ?? doc.groups.find((g) => g.id === selectedId) ?? doc.lights.find((l) => l.id === selectedId);
    return { node_id: selectedId, label: n?.name || selectedId };
  });

  function onPlayHover(inside: boolean): void {
    if (!play || !doc || !hoverState) return;
    stateId = inside ? hoverState : initialState(doc);
  }

  // ⌘K verbs while the 3D studio is open.
  $effect(() => {
    return registry.register('design-3d', [
      { id: 'design3d.generate', title: 'Generate blockout from prompt…', group: 'Design Hall', keywords: '3d scene agent ai', run: () => void blockout() },
      { id: 'design3d.text3d', title: 'Text → 3D model…', group: 'Design Hall', keywords: '3d generate', run: () => (genKind = 'text') },
      { id: 'design3d.turntable', title: 'Toggle turntable', group: 'Design Hall', keywords: '3d orbit rotate', run: () => (turntable = !turntable) },
      { id: 'design3d.play', title: 'Play scene', group: 'Design Hall', keywords: '3d present', run: () => (play = !play) },
      { id: 'design3d.optimize', title: 'Optimize 3D for web…', group: 'Design Hall', keywords: 'glb meshopt compress export', run: () => void optimize() },
    ]);
  });

  function usedInBadge(r: LinkRow): string {
    const pinned = r.link.pinned_version_id ? seqOf(r.link.pinned_version_id) : null;
    if (r.link.policy === 'pinned') return pinned != null ? `pinned v${pinned}` : 'pinned';
    const target = r.link.policy === 'follow_latest' ? artifact.head_seq : seqOf(artifact.approved_version_id ?? '') ?? artifact.head_seq;
    return target != null ? `v${target}` : policyLabel(r.link.policy, pinned);
  }
</script>

{#if !doc}
  <div class="bad">
    <Icon name="warning" size={16} />
    <div>
      <strong>This isn’t a valid 3D scene document.</strong>
      {#if parsed && !parsed.ok}
        <ul>
          {#each parsed.issues.slice(0, 8) as i (i.path + i.message)}
            <li><span class="mono">{i.path || '(root)'}</span> — {i.message}</li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>
{:else}
  <div class="studio3d" class:wide-right={rightTab === 'otto'} class:scene-open={showSceneTools} class:details-open={showDetailsDrawer} bind:clientWidth={studioW} data-testid="studio3d">
    <Studio3dLeftPanel
      {doc}
      {isModelArtifact}
      {readonly}
      {canComment}
      {selectedId}
      {modelEnvPreset}
      {subMeshes}
      {topologyStats}
      {activeMeshUid}
      {kit}
      onselectid={(id) => (selectedId = id)}
      onedit={edit}
      onmodelenv={(p) => (modelEnvPreset = p)}
      onimportglb={() => void importModel()}
      onfocusmesh={(uid) => { activeMeshUid = uid; vp?.focusMesh(uid); }}
      ontogglevis={(uid) => vp?.toggleMeshVisible(uid)}
      oncommentmesh={onCommentSubMesh}
      onsaveview={() => void saveView()}
      ongotoview={(c) => vp?.goToView(c)}
      onviewmenu={viewMenu}
    />

    <section class="center" aria-label="3D viewport">
      <Studio3dToolbar
        {isModelArtifact}
        canEdit={auth.can('design', 'edit')}
        {canComment}
        sceneOpen={showSceneTools}
        detailsOpen={showDetailsDrawer}
        onscene={() => { showSceneTools = !showSceneTools; if (studioW <= 980) showDetailsDrawer = false; }}
        ondetails={() => { showDetailsDrawer = !showDetailsDrawer; if (studioW <= 980) showSceneTools = false; }}
        ongenerate={generateMenu}
        {exportItems}
        {pipelineItems}
        oncamera={readonly || isModelArtifact ? undefined : () => vp?.saveViewAsCamera()}
        {pickMode}
        onpickmode={(on) => { pickMode = on; if (!on) draftAnchor = null; }}
        bind:shading
        bind:wireframeOverlay
        bind:scaleRef
        bind:pickKind
        bind:turntable
        bind:play
        bind:showSource
      />
      <div class="s3d-notices">
        {#if agent}
          <span class="agent" use:announceShown={`Otto · ${agent.label}…`} data-testid="s3d-agent"><span class="pulse" aria-hidden="true"></span> Otto · {agent.label}…</span>
          {#if agent.error}<LoadState what="Otto’s progress" errorTitle="Can’t read Otto’s progress" variant="compact" error={agent.error} empty onretry={() => void poll()} />{/if}
        {:else if agentDraft && head}
          <span class="draft-banner" data-testid="s3d-draft" use:announceShown={`Showing v${head.seq} draft by Otto, not approved`}>
            <Icon name="sparkle" size={12} /> Showing <strong>v{head.seq} draft</strong> by Otto · not approved
          </span>
        {/if}
        {#if notices}{@render notices()}{/if}
      </div>
      {#if showSource}
        <div class="stage">
          <ArtifactStage {artifact} {source} {readonly} showSource bind:selectedId onchange={(s) => !readonly && onchange(s)} />
        </div>
      {:else}
      <div class="stage" role="presentation" onpointerenter={() => onPlayHover(true)} onpointerleave={() => onPlayHover(false)}>
        {#key vpKey}
        <Scene3DViewport
          bind:this={vp}
          {doc}
          cameraButton={false}
          frameModel={isModelArtifact}
          onretry={() => vpKey++}
          bind:selectedId
          readonly={readonly || isModelArtifact}
          {play}
          {turntable}
          stateId={stateId}
          {colors}
          viewCube
          compact
          {statusNote}
          {shading}
          {wireframeOverlay}
          {scaleRef}
          {showSkeleton}
          {animClipIndex}
          {animPlaying}
          {pickMode}
          {pickKind}
          {pin3dInputs}
          {activeAnchor}
          onpolygonhover={(a) => (hoveredAnchor = a)}
          onpolygonpick={onPolygonPick}
          onprojectedpins={(p) => (projectedPins = p)}
          ontopology={(ms, st) => { subMeshes = ms; topologyStats = st; }}
          onriginfo={(r) => { rigInfo = r; if (animClipIndex >= r.clips.length) animClipIndex = 0; }}
          onchange={edit}
          resolveAttachment={resolveModelRef}
        />
        {/key}
        {#if comments}
          <Pin3DLayer
            store={comments}
            pins={projectedPins}
            hovered={hoveredAnchor}
            {draftAnchor}
            {pickMode}
            versionId={effectiveVersionId}
            onselectpin={onSelectPin3d}
            oncleardraft={() => (draftAnchor = null)}
          />
        {/if}
        {#if rigInfo.hasSkeleton || rigInfo.clips.length > 0}
          <div class="states-host">
            <AnimationBar
              rig={rigInfo}
              {showSkeleton}
              clipIndex={animClipIndex}
              playing={animPlaying}
              onskeleton={(on) => (showSkeleton = on)}
              onclip={(i) => (animClipIndex = i)}
              onplay={(on) => (animPlaying = on)}
            />
          </div>
        {:else if !isModelArtifact}
          <div class="states-host">
            <StatesBar {doc} {stateId} {selectedId} {readonly} onpick={(id) => (stateId = id ?? initialState(doc!))} onchange={edit} />
          </div>
        {/if}
      </div>
      {/if}
    </section>

    <aside class="right" id="s3d-details-panel" aria-label="Details">
      <Tabs label="Details panel" idBase="s3d" tabs={rightTabs} value={rightTab} onchange={(id) => (rightTab = id)} />
      <div class="panel" role="tabpanel" id="s3d-panel-{rightTab}" aria-labelledby="s3d-tab-{rightTab}">
        {#if rightTab === 'inspector'}
          <Inspector {doc} bind:selectedId onchange={edit} {readonly} {swatches} brandName={kit?.label ?? null} {colors} {editState} />
          {#if usedIn.length}
            <section class="usedin" aria-label="Used in" data-testid="s3d-usedin">
              <div class="block-head">
                <span class="k">Used in</span>
                <button class="linkbtn" onclick={() => (rightTab = 'links')}>See links</button>
              </div>
              <ul>
                {#each usedIn.slice(0, 6) as r (r.link.id)}
                  <li>
                    <button class="linkbtn row" disabled={!r.other} onclick={() => r.other && openArtifact(r.other.id)}>
                      <Icon name="link" size={12} /> <span class="name">{r.label}{r.link.src_node ? ` › ${r.link.src_node}` : ''}</span>
                    </button>
                    <span class="ver" class:pinned={r.link.policy === 'pinned'}>{usedInBadge(r)}</span>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}
        {:else if rightTab === 'comments' && commentsPanel}
          {@render commentsPanel({ onfocus3d: onFocus3d, onregenerate3d: onRegenerate3d })}
        {:else if rightTab === 'otto'}
          {@render otto(assistSelection)}
        {:else if rightTab === 'versions'}
          {@render versions()}
        {:else}
          {@render links()}
        {/if}
      </div>
    </aside>
  </div>
  <input class="hidden-file" type="file" accept=".glb,.gltf,model/gltf-binary,model/gltf+json" bind:this={glbInput} onchange={(e) => void uploadModel(e)} />
{/if}

<Studio3dModals
  {artifact}
  {genKind}
  {genStage}
  {genInitialPrompt}
  {genCtx}
  canEdit={auth.can('design', 'edit')}
  {canAdmin3d}
  {readonly}
  {effectiveVersionId}
  {providersOpen}
  {exportWsOpen}
  {opt}
  {savingGlb}
  onclosegen={() => { genKind = null; genStage = 'generate'; }}
  onstarted={track}
  oncloseproviders={() => (providersOpen = false)}
  onproviderschanged={() => { void mapi.list3dProviders().then((ps) => { if (!destroyed) mediaProviders = ps; }, () => {}); }}
  oncloseexportws={() => (exportWsOpen = false)}
  oncloseopt={() => (opt = null)}
  ondownloadopt={(bytes, name) => saveBlob(new Blob([bytes.slice()], { type: 'model/gltf-binary' }), name)}
  onsaveopt={() => void saveOptimized()}
/>

<style>
  .studio3d {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 248px minmax(0, 1fr) 320px;
    container-type: inline-size;
  }
  /* The Otto tab holds a conversation and a variants tray: a little wider. */
  .studio3d.wide-right {
    grid-template-columns: 248px minmax(0, 1fr) 360px;
  }
  .right {
    min-height: 0;
    background: var(--surface);
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid var(--border);
  }
  .block-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .center {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .s3d-notices {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding-block: 4px;
    padding-inline: 12px;
    border-block-end: 1px solid var(--border);
    background: var(--bg);
  }
  .s3d-notices:not(:has(*)) {
    display: none;
  }
  .agent { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-s); color: var(--accent-text); }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse { animation: none; }
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    container: s3dstage / inline-size;
  }
  /* A narrow viewport stacks the stats pill above the states bar. */
  @container s3dstage (max-width: 600px) {
    .stage :global(.s3d-status.compact) {
      inset-block-end: 76px; /* the bar's 12 px inset + 54 px touch height + a clear 10 px gap */
    }
  }
  /* A phone on its side has ≈300 px of page: the toolbar floats over the top of the
     canvas (same grid cell) instead of taking a row, so the model gets the screen. */
  @media (max-height: 500px) {
    .center { display: grid; grid-template-rows: auto minmax(0, 1fr); }
    .center > :global(.toolbar), .stage { grid-row: 2; grid-column: 1; }
    .center > :global(.toolbar) { align-self: start; z-index: 2; background: none; border-block-end: 0; pointer-events: none; }
    .center > :global(.toolbar > *) { pointer-events: auto; }
    .center > :global(.toolbar .grow) { display: none; }
    .center > :global(.toolbar .more) { background: var(--surface); border: 1px solid var(--border); }
    .center > :global(.toolbar .on) { background: linear-gradient(var(--accent-soft), var(--accent-soft)), var(--surface); }
    .stage { --s3d-top-inset: 52px; }
    .stage :global(.s3d-hud) { inset-block-start: 58px; }
  }
  .stage > :global(.s3d-viewport) {
    border-radius: 0;
  }
  .stage > :global(.stage) {
    flex: 1;
  }
  .draft-banner {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 10px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    background: color-mix(in srgb, var(--surface) 92%, transparent);
    color: var(--accent-text);
    font-size: var(--fs-s);
    white-space: nowrap;
  }
  .states-host {
    position: absolute;
    inset-block-end: 12px;
    inset-inline-start: 12px;
    max-width: calc(100% - 24px);
  }
  @media (pointer: coarse) {
    .right :global(.otab) { min-block-size: 40px; }
  }
  .panel {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .panel > :global(.s3d-inspector) {
    height: auto;
    overflow: visible;
  }
  .usedin {
    margin: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
  }
  .usedin ul {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .usedin li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .linkbtn.row {
    flex: 1;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
    text-align: start;
  }
  .linkbtn.row .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .linkbtn:disabled {
    cursor: default;
  }
  .ver {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--success-soft);
    color: var(--success);
    white-space: nowrap;
  }
  .ver.pinned {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .bad {
    margin: 20px;
    display: flex;
    gap: 10px;
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .bad ul {
    color: var(--text);
    padding-inline-start: 18px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .hidden-file {
    display: none;
  }
  /* Narrow panes put the side panes behind the toolbar's Scene / Details toggles. */
  @container (max-width: 980px) {
    .studio3d :global(.scene-toggle) { display: inline-flex; }
    .studio3d,
    .studio3d.wide-right {
      grid-template-columns: minmax(0, 1fr) 300px;
    }
    .studio3d > :global(.left) {
      display: none;
    }
    .studio3d.scene-open :global(.left) { display: flex; grid-column: 1 / -1; max-block-size: 320px; border-block-end: 1px solid var(--border); }
    .studio3d.scene-open { grid-template-rows: auto minmax(360px, 1fr); }
  }
  /* Tablets and phone landscape: the canvas comes first. Both panes collapse to
     toggles and open as side columns, so the model never shrinks to a sliver
     between two docked panes. */
  @container (max-width: 1180px) {
    @media (pointer: coarse), (max-height: 500px) {
      .studio3d :global(.scene-toggle),
      .studio3d :global(.details-toggle) { display: inline-flex; }
      .studio3d,
      .studio3d.wide-right,
      .studio3d.scene-open { grid-template-columns: minmax(0, 1fr); grid-template-rows: minmax(0, 1fr); }
      .studio3d > :global(.left),
      .right { display: none; }
      .studio3d.scene-open :global(.left) { display: flex; grid-column: auto; max-block-size: none; border-block-end: 0; }
      .studio3d.scene-open { grid-template-columns: 248px minmax(0, 1fr); }
      .studio3d.details-open .right { display: flex; }
      .studio3d.details-open { grid-template-columns: minmax(0, 1fr) 320px; }
      .studio3d.scene-open.details-open { grid-template-columns: 248px minmax(0, 1fr) 320px; }
    }
  }
  /* Phone and tablet portrait: one column. An open pane takes the lower half
     under the canvas (one at a time), so the toolbar never moves and the model
     stays in view while you edit it. */
  @container (max-width: 768px) {
    .studio3d :global(.details-toggle) { display: inline-flex; }
    .studio3d,
    .studio3d.wide-right,
    .studio3d.scene-open,
    .studio3d.details-open,
    .studio3d.scene-open.details-open {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 1fr);
    }
    .right { display: none; }
    .studio3d.scene-open,
    .studio3d.details-open {
      grid-template-rows: minmax(0, 1fr) minmax(0, 1fr);
    }
    .studio3d.scene-open :global(.left),
    .studio3d.details-open .right {
      display: flex;
      order: 2;
      grid-column: auto;
      max-block-size: none;
      border-inline: 0;
      border-block-start: 1px solid var(--border);
    }
    /* A jump to a comment or Otto opens Details while Scene is open: Details wins. */
    .studio3d.scene-open.details-open :global(.left) { display: none; }
  }
</style>
