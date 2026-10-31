///<reference types="svelte" />
;
import { announceShown } from '../../../lib/announce.svelte';
import { rowMenu } from '../../../lib/rowMenu';
// 3D Studio 1.5 — the Design Hall layout for a `scene3d` artifact (mockup S4):
//
//   ┌ HIERARCHY ─────┬ toolbar: ✨ Generate ▾ · Turntable · Play · Export ▾ ───┬ Inspector · Links · References ┐
//   │ tree (+ add)   │ VIEWPORT (three.js)             view cube ⌝          │ transform · material presets  │
//   │ Environment    │   "Showing v8 draft by Otto · not approved"          │ brand swatches · sliders      │
//   │ Views          │ ⌞ States [Idle | Hover | Flipped]   12 objects · … ⌟  │ notes · Used in               │
//   └────────────────┴──────────────────────────────────────────────────────┴───────────────────────────────┘
//
// It edits the SAME working copy the artifact view owns (`source` →
// `onchange(source)`): the view keeps versions, saving, conflicts and the
// header. Everything here is a pure `ops.ts` edit of the scene3d v2 doc, an
// agent turn through the unified design assist route (Blockout / Text→3D /
// Image→3D / Refine in Blender — each lands as a reviewable `agent` version),
// or a local export. Cloud 3D providers are opt-in and disabled without a
// Keychain key; nothing is sent anywhere without an explicit click.
import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
import { toastError } from '../../../lib/toastError';
import { copyWithToast } from '../../../lib/clipboard';
import { untrack, type Snippet } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import Modal from '../../../lib/components/Modal.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { registry } from '../../../lib/commands.svelte';
import { api, ApiError } from '../../../lib/api/client';
import { downloadText } from '../../../lib/components/exporters';
import * as dapi from '../../../lib/api/design';
import { listTurns, startAssist as startTurn } from '../assist/api';
import type { DesignArtifact, DesignAssistTurn, DesignVersion } from '../../../lib/api/types';
import {
    ENV_PRESETS,
    Hierarchy,
    Inspector,
    Scene3DViewport,
    addCameraPreset,
    addGltfSrc,
    budgetStatus,
    editTargetState,
    exportObjectToGlb,
    exportObjectToUsdz,
    formatBytes,
    glbFileName,
    initialState,
    parseScene,
    resolveColor,
    removeCameraPreset,
    serializeScene,
    setBrand,
    setEnvironment,
    updateCameraPreset,
    type BrandSwatch,
    type EnvPresetId,
    type Scene3dDoc,
  } from '../../../lib/design/scene3d';
import type { ColorResolver } from '../../../lib/design/scene3d/build';
import { colorTokens, resolveToken, tokenRef } from '../brand/tokens';
import type { OptimizeResult } from '../../../lib/design/scene3d/optimize';
import ArtifactStage from '../ArtifactStage.svelte';
import StatesBar from './StatesBar.svelte';
import GenerateModal from './GenerateModal.svelte';
import { blenderRefinePrompt, blockoutPrompt, type Gen3dContext, type Gen3dKind } from './providers';
import { loadBrandKit, modelScope, type BrandKit } from './sources';
import { openArtifact } from '../nav';
import { library } from '../library.svelte';
import { policyLabel, type LinkRow } from '../model';
import { turnOutcome, type AssistSelection } from '../assist/model';
;

  interface Props {
    artifact: DesignArtifact;
    /** The working copy (scene3d JSON). */
    source: string | null;
    readonly: boolean;
    dirty: boolean;
    head: DesignVersion | null;
    usedIn: LinkRow[];
    linkCount: number;
    seqOf: (versionId: string) => number | null;
    onchange: (source: string) => void;
    /** The view's Otto / Links / References panels (it owns their data); Otto
     *  gets the selected object so a turn can focus on it. */
    otto: Snippet<[AssistSelection | null]>;
    links: Snippet;
    references: Snippet;
    /** Open on the Otto tab (the lobby's Generate hand-off, `#/design/a/<id>/otto`). */
    openOtto?: boolean;
    /** The view's toolbar notices (imported / read-only / newer version). */
    notices?: Snippet;
  };;
  type RightTab = 'inspector' | 'otto' | 'links' | 'references';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { artifact, source, readonly, dirty, head, usedIn, linkCount, seqOf, onchange, otto, links, references, notices, openOtto = false }: Props = $props();

  // ── Document ──────────────────────────────────────────────────────────────
  const parsed = $derived(source ? parseScene(source) : null);
  const doc = $derived<Scene3dDoc | null>(parsed?.ok ? parsed.doc : null);

  let selectedId = $state<string | null>(null);
  let stateId = $state<string | null>(null);
  let play = $state(false);
  /** Show the scene JSON beside a plain viewport (hand edits, agent diffs). */
  let showSource = $state(false);
  let showSceneTools = $state(false);
  let turntable = $state(false);

  const rightTabs = $derived<TabItem<RightTab>[]>([
    { id: 'inspector', label: 'Inspector', testid: 's3d-tab-inspector' },
    { id: 'otto', label: 'Otto', testid: 'design-tab-otto' },
    { id: 'links', label: 'Links', count: linkCount, testid: 'design-tab-links' },
    { id: 'references', label: 'References', testid: 'design-tab-references' },
  ]);
  let rightTab = $state<RightTab>(untrack(() => (openOtto ? 'otto' : 'inspector')));
  $effect(() => {
    if (openOtto) untrack(() => (rightTab = 'otto'));
  });
  let vp = $state<ReturnType<typeof Scene3DViewport> | null>(null);

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
  let agent = $state<{ turnId: string; label: string } | null>(null);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  // Set on unmount: a poll in flight must not re-arm a timer (or toast) from a dead view.
  let destroyed = false;
  function track(turn: DesignAssistTurn, label: string): void {
    agent = { turnId: turn.turn_id, label };
    toasts.info('Otto is working on it', `${label}. The result lands as a new agent version you can review.`);
    schedulePoll();
  }
  function schedulePoll(): void {
    if (destroyed) return;
    if (pollTimer) clearTimeout(pollTimer);
    pollTimer = setTimeout(() => void poll(), 3000);
  }
  async function poll(): Promise<void> {
    const a = agent;
    if (!a) return;
    try {
      const turns = await listTurns(artifact.id);
      if (destroyed) return;
      const out = turnOutcome(turns.find((x) => x.turn_id === a.turnId));
      if (agent?.turnId !== a.turnId) return;
      if (out.kind === 'wait') return schedulePoll();
      agent = null;
      toasts[out.kind](out.title, out.body);
    } catch {
      if (destroyed) return;
      if (agent?.turnId === a.turnId) schedulePoll();
    }
  }
  $effect(() => () => {
    destroyed = true;
    if (pollTimer) clearTimeout(pollTimer);
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
      const msg = e instanceof ApiError && e.status === 409 ? 'Otto is already working on this design.' : e instanceof Error ? e.message : String(e);
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
    const items: MenuItem[] = [
      { label: 'Text → 3D model…', icon: 'sparkle', disabled: readonly, action: () => (genKind = 'text') },
      { label: 'Image → 3D model…', icon: 'image', disabled: readonly, action: () => (genKind = 'image') },
      { label: 'Blockout from prompt (scene JSON)…', icon: 'box', disabled: readonly, action: () => void blockout() },
      { separator: true },
      { label: 'Refine in Blender (MCP)…', icon: 'layers', disabled: readonly, action: () => void refineInBlender() },
    ];
    ctxMenu.show(e, items);
  }

  // ── Export ────────────────────────────────────────────────────────────────
  function saveBlob(blob: Blob, name: string): void {
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 5000);
  }
  async function exportAs(kind: 'glb' | 'usdz' | 'png' | 'sheet'): Promise<void> {
    if (!vp) return;
    try {
      if (kind === 'png') {
        const b = await vp.snapshotPng();
        if (b) saveBlob(b, glbFileName(artifact.title, 'png'));
      } else if (kind === 'sheet') {
        const b = await vp.turntableSheet(8);
        if (b) saveBlob(b, glbFileName(`${artifact.title}-turntable`, 'png'));
      } else {
        const root = vp.contentRoot();
        if (!root) return;
        const b = kind === 'glb' ? await exportObjectToGlb(root) : await exportObjectToUsdz(root);
        saveBlob(b, glbFileName(artifact.title, kind));
      }
    } catch (e) {
      toastError('Couldn’t export the model', e);
    }
  }
  function exportMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'GLB (.glb)', icon: 'download', action: () => void exportAs('glb') },
      { label: 'USDZ (.usdz) — AR Quick Look', icon: 'download', action: () => void exportAs('usdz') },
      { label: 'PNG snapshot', icon: 'image', action: () => void exportAs('png') },
      { label: 'PNG turntable (8 frames)', icon: 'image', action: () => void exportAs('sheet') },
      { label: 'Scene JSON', icon: 'file', disabled: !source, action: () => source && downloadText(source, `${glbFileName(artifact.title).replace(/\.glb$/, '')}.scene3d.json`, 'application/json') },
      { separator: true },
      { label: 'Optimize for web (meshopt)…', icon: 'zap', action: () => void optimize() },
      { label: 'Blender script — Product arena only', icon: 'file', disabled: true },
    ];
    ctxMenu.show(e, items);
  }

  // Optimize for web: GLB of the live scene → gltf-transform (lazy chunk) → budget readout.
  let opt = $state<{ phase: 'running' | 'done' | 'error'; result?: OptimizeResult; error?: string } | null>(null);
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
      opt = { phase: 'error', error: e instanceof Error ? e.message : String(e) };
    }
  }
  function b64(bytes: Uint8Array): string {
    let s = '';
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(s);
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
        content_b64: b64(r.bytes),
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
    let models: DesignArtifact[] = [];
    try {
      const f = artifact.project_id ? { project_id: artifact.project_id } : { workspace_id: artifact.workspace_id };
      const [glb, gltf] = await Promise.all([dapi.listArtifacts({ ...f, format: 'glb', limit: 50 }), dapi.listArtifacts({ ...f, format: 'gltf', limit: 50 })]);
      models = [...glb, ...gltf];
    } catch {
      /* offer the upload only */
    }
    const items: MenuItem[] = [{ label: 'Upload GLB / glTF…', icon: 'arrowUp', pinned: true, action: () => glbInput?.click() }];
    if (models.length) items.push({ separator: true });
    for (const m of models) {
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
    const anchor = document.querySelector<HTMLElement>('[data-testid="s3d-left"]');
    const rect = anchor?.getBoundingClientRect();
    // Opened from the Hierarchy's Add menu (already closed): anchor near the panel.
    const at = { clientX: (rect?.left ?? 200) + 40, clientY: (rect?.top ?? 120) + 40, preventDefault() {}, stopPropagation() {} };
    ctxMenu.show(at as unknown as MouseEvent, items, { filter: models.length > 8, maxVisible: 12 });
  }
  async function uploadModel(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const f = input.files?.[0];
    input.value = '';
    if (!f || !doc) return;
    const ext = f.name.toLowerCase().endsWith('.gltf') ? 'gltf' : 'glb';
    try {
      const bytes = new Uint8Array(await f.arrayBuffer());
      const res = await dapi.createArtifact({
        workspace_id: artifact.workspace_id,
        project_id: artifact.project_id ?? undefined,
        studio: '3d',
        format: ext,
        title: f.name.replace(/\.(glb|gltf)$/i, ''),
        content_b64: b64(bytes),
        message: `Imported for ${artifact.title}`,
      });
      const r = addGltfSrc(doc, `otto://design/${res.artifact.id}@latest`, { name: res.artifact.title, position: [1.4, 0, 0] });
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
    thumbTimer = setTimeout(() => void storeThumbnail(), 1200);
  });
  $effect(() => () => {
    if (thumbTimer) clearTimeout(thumbTimer);
  });
  async function storeThumbnail(): Promise<void> {
    const b = await vp?.snapshotPng();
    if (!b) return;
    try {
      const img = await createImageBitmap(b);
      const w = Math.min(640, img.width);
      const h = Math.round((img.height / img.width) * w);
      const c = document.createElement('canvas');
      c.width = w;
      c.height = h;
      c.getContext('2d')?.drawImage(img, 0, 0, w, h);
      const small = await new Promise<Blob | null>((res) => c.toBlob(res, 'image/png'));
      if (small) await dapi.putThumbnail(artifact.id, small);
    } catch {
      /* a thumbnail is a cache — never interrupt editing over it */
    }
  }

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
  const env = $derived(doc?.environment);

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
;
async () => {

if(!doc){
   { svelteHTML.createElement("div", { "class":`bad`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":16,}});}
     { svelteHTML.createElement("div", {});
       { svelteHTML.createElement("strong", {});       }
      if(parsed && !parsed.ok){
         { svelteHTML.createElement("ul", {});
             for(let i of __sveltets_2_ensureArray((parsed.issues.slice(0, 8)))){i.path + i.message;
             { svelteHTML.createElement("li", {}); { svelteHTML.createElement("span", { "class":`mono`,});i.path || '(root)'; }  i.message; }
          }
         }
      }
     }
   }
}else{
   { svelteHTML.createElement("div", {     "class":`studio3d`,...__sveltets_2_empty({"data-testid":`studio3d`}),});rightTab === 'otto';showSceneTools;
     { svelteHTML.createElement("aside", {       "class":`left`,"id":`s3d-scene-tools`,"aria-label":`Scene`,...__sveltets_2_empty({"data-testid":`s3d-left`}),});
       { svelteHTML.createElement("div", { "class":`hier`,});
         { const $$_yhcrareiH3C = __sveltets_2_ensureComponent(Hierarchy); const $$_yhcrareiH3 = new $$_yhcrareiH3C({ target: __sveltets_2_any(), props: {      doc,selectedId,"onchange":edit,readonly,"onimportGlb":() => void importModel(),}});/*Ωignore_startΩ*/() => selectedId = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_yhcrareiH3.$$bindings = 'selectedId';}
       }
       { svelteHTML.createElement("section", {   "class":`block`,"aria-label":`Environment`,});
         { svelteHTML.createElement("div", { "class":`block-head`,});
           { svelteHTML.createElement("span", { "class":`k`,});  }
         }
         { svelteHTML.createElement("div", { "class":`env-row`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":13,}});}
           { svelteHTML.createElement("select", {             "class":`env-select`,"value":env?.preset ?? 'none',"disabled":readonly,"aria-label":`Environment preset`,...__sveltets_2_empty({"data-testid":`s3d-env`}),"onchange":(e) => edit(setEnvironment(doc!, { preset: (e.currentTarget as HTMLSelectElement).value as EnvPresetId })),});
               for(let p of __sveltets_2_ensureArray(ENV_PRESETS)){p.id; { svelteHTML.createElement("option", { "value":p.id,});p.label; }}
           }
         }
        if(env && env.preset !== 'none'){
           { svelteHTML.createElement("label", { "class":`env-row check`,});
             { svelteHTML.createElement("input", {        "type":`checkbox`,"checked":env.background !== false,"disabled":readonly,"onchange":(e) => edit(setEnvironment(doc!, { background: (e.currentTarget as HTMLInputElement).checked })),});}
             { svelteHTML.createElement("span", {});   }
           }
        }
       }
       { svelteHTML.createElement("section", {   "class":`block`,"aria-label":`Views`,});
         { svelteHTML.createElement("div", { "class":`block-head`,});
           { svelteHTML.createElement("span", { "class":`k`,});  }
          if(!readonly){
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Save the current view`,"title":`Save the current view`,"onclick":() => void saveView(),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}
             }
          }
         }
        if(doc.cameras?.length){
           { svelteHTML.createElement("ul", { "class":`views`,});
               for(let c of __sveltets_2_ensureArray(doc.cameras)){c.id;
               { svelteHTML.createElement("li", {});
                 {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {        "class":`view`,"onclick":() => vp?.goToView(c),"oncontextmenu":(e) => { e.preventDefault(); viewMenu(e, c.id); },"title":`#view:${c.id}`,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}  { svelteHTML.createElement("span", {});c.name ?? c.id; }
                 }}
                 { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`View options`,"title":`View options`,"onclick":(e) => viewMenu(e, c.id),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});} }
               }
            }
           }
        }else{
           { svelteHTML.createElement("p", { "class":`dim`,});                 }
        }
       }
       { svelteHTML.createElement("section", {   "class":`block card`,"aria-label":`Scene`,});
         { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("p", {});
          doc.objects.length;   doc.lights.length; doc.states?.length ? ` · ${doc.states.length} states` : '';
          if(kit){    { svelteHTML.createElement("strong", {});kit.label; } }else{             }
         }
       }
     }

     { svelteHTML.createElement("section", {   "class":`center`,"aria-label":`3D viewport`,});
       { svelteHTML.createElement("div", { "class":`toolbar`,});
         { svelteHTML.createElement("button", {       "class":`btn small scene-toggle`,"aria-expanded":showSceneTools,"aria-controls":`s3d-scene-tools`,"onclick":() => (showSceneTools = !showSceneTools),});  }
         { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":generateMenu,"disabled":readonly,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`s3d-generate`}),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}   { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {        "class":`btn small`,"aria-pressed":turntable,"onclick":() => (turntable = !turntable),...__sveltets_2_empty({"data-testid":`s3d-turntable`}),});turntable;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} 
         }
         { svelteHTML.createElement("button", {        "class":`btn small`,"aria-pressed":play,"onclick":() => (play = !play),...__sveltets_2_empty({"data-testid":`s3d-play`}),});play;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} 
         }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":exportMenu,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`s3d-export`}),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});}   { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {        "class":`btn small ghost`,"aria-pressed":showSource,"onclick":() => (showSource = !showSource),...__sveltets_2_empty({"data-testid":`design-source-toggle`}),});showSource;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} 
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(agent){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('span'),(`Otto · ${agent.label}…`)));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {     "class":`agent`,...__sveltets_2_empty({"data-testid":`s3d-agent`}),}); { svelteHTML.createElement("span", {   "class":`pulse`,"aria-hidden":`true`,}); }   agent.label;  }}
        } else if (agentDraft && head){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('span'),(`Showing v${head.seq} draft by Otto, not approved`)));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {     "class":`draft-banner`,...__sveltets_2_empty({"data-testid":`s3d-draft`}),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}   { svelteHTML.createElement("strong", {}); head.seq;  }     
           }}
        }
        if(notices){;__sveltets_2_ensureSnippet(notices());}
       }
      if(showSource){
         { svelteHTML.createElement("div", { "class":`stage`,});
           { const $$_egatStcafitrA3C = __sveltets_2_ensureComponent(ArtifactStage); const $$_egatStcafitrA3 = new $$_egatStcafitrA3C({ target: __sveltets_2_any(), props: {      artifact,source,readonly,"showSource":true,selectedId,"onchange":(s) => !readonly && onchange(s),}});/*Ωignore_startΩ*/() => selectedId = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_egatStcafitrA3.$$bindings = 'selectedId';}
         }
      }else{
       { svelteHTML.createElement("div", {       "class":`stage`,"role":`presentation`,"onpointerenter":() => onPlayHover(true),"onpointerleave":() => onPlayHover(false),});
         { const $$_tropweiVD3enecS3C = __sveltets_2_ensureComponent(Scene3DViewport); const $$_tropweiVD3enecS3 = new $$_tropweiVD3enecS3C({ target: __sveltets_2_any(), props: {                  doc,selectedId,"readonly":readonly,play,turntable,"stateId":stateId,colors,"viewCube":true,"compact":true,statusNote,"onchange":edit,"resolveAttachment":models.resolve,}});vp = $$_tropweiVD3enecS3;/*Ωignore_startΩ*/() => selectedId = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_tropweiVD3enecS3.$$bindings = 'selectedId';}
         { svelteHTML.createElement("div", { "class":`states-host`,});
           { const $$_raBsetatS4C = __sveltets_2_ensureComponent(StatesBar); new $$_raBsetatS4C({ target: __sveltets_2_any(), props: {        doc,stateId,selectedId,readonly,"onpick":(id) => (stateId = id ?? initialState(doc!)),"onchange":edit,}});}
         }
       }
      }
     }

     { svelteHTML.createElement("aside", {   "class":`right`,"aria-label":`Details`,});
       { const $$_sbaT2C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT2C({ target: __sveltets_2_any(), props: {          "label":`Details panel`,"idBase":`s3d`,"tabs":rightTabs,"value":rightTab,"onchange":(id) => (rightTab = id),}});}
       { svelteHTML.createElement("div", {       "class":`panel`,"role":`tabpanel`,"id":`s3d-panel-${rightTab}`,"aria-labelledby":`s3d-tab-${rightTab}`,});
        if(rightTab === 'inspector'){
           { const $$_rotcepsnI3C = __sveltets_2_ensureComponent(Inspector); const $$_rotcepsnI3 = new $$_rotcepsnI3C({ target: __sveltets_2_any(), props: {         doc,selectedId,"onchange":edit,readonly,swatches,"brandName":kit?.label ?? null,colors,editState,}});/*Ωignore_startΩ*/() => selectedId = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rotcepsnI3.$$bindings = 'selectedId';}
          if(usedIn.length){
             { svelteHTML.createElement("section", {     "class":`usedin`,"aria-label":`Used in`,...__sveltets_2_empty({"data-testid":`s3d-usedin`}),});
               { svelteHTML.createElement("div", { "class":`block-head`,});
                 { svelteHTML.createElement("span", { "class":`k`,});  }
                 { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => (rightTab = 'links'),});  }
               }
               { svelteHTML.createElement("ul", {});
                   for(let r of __sveltets_2_ensureArray((usedIn.slice(0, 6)))){r.link.id;
                   { svelteHTML.createElement("li", {});
                     { svelteHTML.createElement("button", {     "class":`linkbtn row`,"disabled":!r.other,"onclick":() => r.other && openArtifact(r.other.id),});
                       { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`name`,});r.label;r.link.src_node ? ` › ${r.link.src_node}` : ''; }
                     }
                     { svelteHTML.createElement("span", {  "class":`ver`,});r.link.policy === 'pinned';usedInBadge(r); }
                   }
                }
               }
             }
          }
        } else if (rightTab === 'otto'){
          ;__sveltets_2_ensureSnippet(otto(assistSelection));
        } else if (rightTab === 'links'){
          ;__sveltets_2_ensureSnippet(links());
        }else{
          ;__sveltets_2_ensureSnippet(references());
        }
       }
     }
   }
   { const $$_input0 = svelteHTML.createElement("input", {         "class":`hidden-file`,"type":`file`,"accept":`.glb,.gltf,model/gltf-binary,model/gltf+json`,"onchange":(e) => void uploadModel(e),});glbInput = $$_input0;}
}

if(genKind){
   { const $$_ladoMetareneG0C = __sveltets_2_ensureComponent(GenerateModal); new $$_ladoMetareneG0C({ target: __sveltets_2_any(), props: {         "kind":genKind,artifact,"ctx":genCtx,"onclose":() => (genKind = null),"onstarted":track,}});}
}

if(opt){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Optimize for web`,"width":500,"onclose":() => (opt = null),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (opt = null),});  }
      if(opt?.phase === 'done' && opt.result){
         { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => opt?.result && saveBlob(new Blob([opt.result.bytes], { type: 'model/gltf-binary' }), glbFileName(`${artifact.title}-web`)),});  }
         { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":readonly || savingGlb,"onclick":() => void saveOptimized(),});savingGlb ? 'Saving…' : 'Save as GLB design'; }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", {   "class":`opt`,...__sveltets_2_empty({"data-testid":`s3d-optimize`}),});
      if(opt.phase === 'running'){
         { svelteHTML.createElement("p", { "class":`dim`,});          }
      } else if (opt.phase === 'error'){
         { svelteHTML.createElement("p", {   "class":`err`,"role":`alert`,});  opt.error; }
      } else if (opt.result){
        const r = opt.result;
        const b = budgetStatus(r.after);
         { svelteHTML.createElement("div", { "class":`sizes`,});
           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("strong", {});formatBytes(r.before); } }
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":14,}});}
           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("strong", {});formatBytes(r.after); } }
           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("strong", {});r.before > 0 ? Math.max(0, Math.round((1 - r.after / r.before) * 100)) : 0;  } }
         }
         { svelteHTML.createElement("div", {           "class":`budget ${b.tone}`,"role":`meter`,"aria-valuemin":0,"aria-valuemax":100,"aria-valuenow":Math.min(100, b.pct),"aria-label":`Web budget used`,});
           { svelteHTML.createElement("span", {  "class":`bar`,});__sveltets_2_ensureType(String, Number, `${Math.min(100, b.pct)} `); }
         }
         { svelteHTML.createElement("p", { "class":`dim`,});  b.label;b.tone === 'over' ? ' — simplify the scene or split it before embedding.' : '.'; }
         { svelteHTML.createElement("ul", { "class":`steps`,});
             for(let s of __sveltets_2_ensureArray(r.steps)){s; { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} s; }}
         }
         { svelteHTML.createElement("p", { "class":`dim`,});                }
      }
     }
    
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Studio3D__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Studio3D__SvelteComponent_ = ReturnType<typeof Studio3D__SvelteComponent_>;
/*Ωignore_endΩ*/export default Studio3D__SvelteComponent_;