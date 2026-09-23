<script lang="ts">
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
  import type { Snippet } from 'svelte';
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { registry } from '../../../lib/commands.svelte';
  import { api } from '../../../lib/api/client';
  import { ApiError } from '../../../lib/api/client';
  import { downloadText } from '../../../lib/components/exporters';
  import * as dapi from '../../../lib/api/design';
  import type { DesignArtifact, DesignAssistTurn, DesignVersion } from '../../../lib/api/types';
  import {
    ENV_PRESETS,
    Hierarchy,
    Inspector,
    Scene3DViewport,
    addCameraPreset,
    addGltfSrc,
    brandSwatches,
    budgetStatus,
    editTargetState,
    exportObjectToGlb,
    exportObjectToUsdz,
    formatBytes,
    glbFileName,
    initialState,
    parseScene,
    removeCameraPreset,
    serializeScene,
    setBrand,
    setEnvironment,
    updateCameraPreset,
    type EnvPresetId,
    type Scene3dDoc,
  } from '../../product/design/scene3d';
  import type { OptimizeResult } from '../../product/design/scene3d/optimize';
  import ArtifactStage from '../ArtifactStage.svelte';
  import StatesBar from './StatesBar.svelte';
  import GenerateModal from './GenerateModal.svelte';
  import { blenderRefinePrompt, blockoutPrompt, type Gen3dContext, type Gen3dKind } from './providers';
  import { loadBrandKit, resolveModelRef, type BrandKit } from './sources';
  import { openArtifact } from '../nav';
  import { library } from '../library.svelte';
  import { policyLabel, type LinkRow } from '../model';

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
    /** The view's Links / References panels (it owns their data). */
    links: Snippet;
    references: Snippet;
    /** The view's toolbar notices (imported / read-only / newer version). */
    notices?: Snippet;
  }
  let { artifact, source, readonly, dirty, head, usedIn, linkCount, seqOf, onchange, links, references, notices }: Props = $props();

  // ── Document ──────────────────────────────────────────────────────────────
  const parsed = $derived(source ? parseScene(source) : null);
  const doc = $derived<Scene3dDoc | null>(parsed?.ok ? parsed.doc : null);

  let selectedId = $state<string | null>(null);
  let stateId = $state<string | null>(null);
  let play = $state(false);
  /** Show the scene JSON beside a plain viewport (hand edits, agent diffs). */
  let showSource = $state(false);
  let turntable = $state(false);
  let rightTab = $state<'inspector' | 'links' | 'references'>('inspector');
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
  const swatches = $derived(brandSwatches(kit?.doc));

  // ── Agent turns (Blockout / Text→3D / Image→3D / Refine in Blender) ───────
  let agent = $state<{ turnId: string; label: string } | null>(null);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  function track(turn: DesignAssistTurn, label: string): void {
    agent = { turnId: turn.turn_id, label };
    toasts.info('Otto is working on it', `${label}. The result lands as a new agent version you can review.`);
    schedulePoll();
  }
  function schedulePoll(): void {
    if (pollTimer) clearTimeout(pollTimer);
    pollTimer = setTimeout(() => void poll(), 3000);
  }
  async function poll(): Promise<void> {
    const a = agent;
    if (!a) return;
    try {
      const turns = await dapi.listAssistTurns(artifact.id);
      const t = turns.find((x) => x.turn_id === a.turnId);
      if (!t || t.status === 'starting' || t.status === 'running') {
        if (agent?.turnId === a.turnId) schedulePoll();
        return;
      }
      agent = null;
      if (t.status === 'done') toasts.success('Otto’s version is ready', t.message ?? 'Review it in the version strip.');
      else if (t.status === 'unchanged') toasts.info('Otto made no changes', t.message ?? undefined);
      else if (t.status === 'conflict') toasts.warn('Saved as a side version', 'The scene changed while Otto worked — compare and accept it from history.');
      else toasts.error('Otto couldn’t finish', t.error ?? 'The turn failed; the scene is unchanged.');
    } catch {
      if (agent?.turnId === a.turnId) schedulePoll();
    }
  }
  $effect(() => () => {
    if (pollTimer) clearTimeout(pollTimer);
  });

  const genCtx = $derived<Gen3dContext>({ cloudKeys: {}, canEdit: !readonly, busy: !!agent, dirty });
  let genKind = $state<Gen3dKind | null>(null);

  async function startAssist(prompt: string, mode: 'generate' | 'refine', label: string): Promise<void> {
    try {
      track(await dapi.assistArtifact(artifact.id, { prompt, mode }), label);
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
      toasts.error('Export failed', e instanceof Error ? e.message : String(e));
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
      const { optimizeGlb } = await import('../../product/design/scene3d/optimize');
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
      toasts.error('Couldn’t save the GLB', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t import the model', err instanceof Error ? err.message : String(err));
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
        action: () => void navigator.clipboard?.writeText(`otto://design/${artifact.id}@approved#view:${id}`).then(() => toasts.success('Copied', `#view:${id}`)),
      },
      { separator: true },
      { label: 'Delete view', icon: 'trash', danger: true, disabled: readonly, action: () => edit(removeCameraPreset(doc!, id)) },
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
</script>

{#if !doc}
  <div class="bad" role="alert">
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
  <div class="studio3d" data-testid="studio3d">
    <aside class="left" aria-label="Scene" data-testid="s3d-left">
      <div class="hier">
        <Hierarchy {doc} bind:selectedId onchange={edit} {readonly} onimportGlb={() => void importModel()} />
      </div>
      <section class="block" aria-label="Environment">
        <div class="block-head">
          <span class="k">Environment</span>
        </div>
        <div class="env-row">
          <Icon name="globe" size={13} />
          <select
            class="env-select"
            value={env?.preset ?? 'none'}
            disabled={readonly}
            aria-label="Environment preset"
            data-testid="s3d-env"
            onchange={(e) => edit(setEnvironment(doc!, { preset: (e.currentTarget as HTMLSelectElement).value as EnvPresetId }))}
          >
            {#each ENV_PRESETS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
          </select>
        </div>
        {#if env && env.preset !== 'none'}
          <label class="env-row check">
            <input type="checkbox" checked={env.background !== false} disabled={readonly} onchange={(e) => edit(setEnvironment(doc!, { background: (e.currentTarget as HTMLInputElement).checked }))} />
            <span>Show as backdrop</span>
          </label>
        {/if}
      </section>
      <section class="block" aria-label="Views">
        <div class="block-head">
          <span class="k">Views</span>
          {#if !readonly}
            <button class="icon-btn" aria-label="Save the current view" title="Save the current view" onclick={() => void saveView()}>
              <Icon name="plus" size={13} />
            </button>
          {/if}
        </div>
        {#if doc.cameras?.length}
          <ul class="views">
            {#each doc.cameras as c (c.id)}
              <li>
                <button class="view" onclick={() => vp?.goToView(c)} oncontextmenu={(e) => { e.preventDefault(); viewMenu(e, c.id); }} title="#view:{c.id}">
                  <Icon name="eye" size={12} /> <span>{c.name ?? c.id}</span>
                </button>
                <button class="icon-btn" aria-label="View options" title="View options" onclick={(e) => viewMenu(e, c.id)}><Icon name="more" size={12} /></button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="dim">Frame a shot, then + to save it (e.g. “Hero angle”) — site embeds can use it.</p>
        {/if}
      </section>
      <section class="block card" aria-label="Scene">
        <span class="k">Scene</span>
        <p>
          {doc.objects.length} objects · {doc.lights.length} lights{doc.states?.length ? ` · ${doc.states.length} states` : ''}.
          {#if kit}Materials can use <strong>{kit.label}</strong>.{:else}No brand kit in this project yet — brand colours appear once one exists.{/if}
        </p>
      </section>
    </aside>

    <section class="center" aria-label="3D viewport">
      <div class="toolbar">
        <button class="btn small" onclick={generateMenu} disabled={readonly} aria-haspopup="menu" data-testid="s3d-generate">
          <Icon name="sparkle" size={12} /> Generate <Icon name="chevronDown" size={11} />
        </button>
        <button class="btn small" class:on={turntable} aria-pressed={turntable} onclick={() => (turntable = !turntable)} data-testid="s3d-turntable">
          <Icon name="refresh" size={12} /> Turntable
        </button>
        <button class="btn small" class:on={play} aria-pressed={play} onclick={() => (play = !play)} data-testid="s3d-play">
          <Icon name="play" size={12} /> {play ? 'Stop' : 'Play'}
        </button>
        <button class="btn small" onclick={exportMenu} aria-haspopup="menu" data-testid="s3d-export">
          <Icon name="download" size={12} /> Export <Icon name="chevronDown" size={11} />
        </button>
        <button class="btn small ghost" class:on={showSource} aria-pressed={showSource} onclick={() => (showSource = !showSource)} data-testid="design-source-toggle">
          <Icon name="file" size={12} /> Source
        </button>
        <span class="grow"></span>
        {#if agent}
          <span class="agent" role="status" data-testid="s3d-agent"><span class="pulse" aria-hidden="true"></span> Otto · {agent.label}…</span>
        {:else if agentDraft && head}
          <span class="draft-banner" role="status" data-testid="s3d-draft">
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
        <Scene3DViewport
          bind:this={vp}
          {doc}
          bind:selectedId
          readonly={readonly}
          {play}
          {turntable}
          stateId={stateId}
          brand={kit?.doc ?? null}
          viewCube
          compact
          {statusNote}
          onchange={edit}
          resolveAttachment={resolveModelRef}
        />
        <div class="states-host">
          <StatesBar {doc} {stateId} {selectedId} {readonly} onpick={(id) => (stateId = id ?? initialState(doc!))} onchange={edit} />
        </div>
      </div>
      {/if}
    </section>

    <aside class="right" aria-label="Details">
      <div class="tabs" role="tablist" aria-label="Details panel">
        <button role="tab" aria-selected={rightTab === 'inspector'} class:active={rightTab === 'inspector'} onclick={() => (rightTab = 'inspector')} data-testid="s3d-tab-inspector">Inspector</button>
        <button role="tab" aria-selected={rightTab === 'links'} class:active={rightTab === 'links'} onclick={() => (rightTab = 'links')} data-testid="design-tab-links">
          Links <span class="count">{linkCount}</span>
        </button>
        <button role="tab" aria-selected={rightTab === 'references'} class:active={rightTab === 'references'} onclick={() => (rightTab = 'references')} data-testid="design-tab-references">References</button>
      </div>
      <div class="panel" role="tabpanel">
        {#if rightTab === 'inspector'}
          <Inspector {doc} bind:selectedId onchange={edit} {readonly} {swatches} brandName={kit?.label ?? null} brand={kit?.doc ?? null} {editState} />
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
        {:else if rightTab === 'links'}
          {@render links()}
        {:else}
          {@render references()}
        {/if}
      </div>
    </aside>
  </div>
  <input class="hidden-file" type="file" accept=".glb,.gltf,model/gltf-binary,model/gltf+json" bind:this={glbInput} onchange={(e) => void uploadModel(e)} />
{/if}

{#if genKind}
  <GenerateModal kind={genKind} {artifact} ctx={genCtx} onclose={() => (genKind = null)} onstarted={track} />
{/if}

{#if opt}
  <Modal title="Optimize for web" width={500} onclose={() => (opt = null)}>
    <div class="opt" data-testid="s3d-optimize">
      {#if opt.phase === 'running'}
        <p class="dim">Exporting the scene and compressing it with gltf-transform + meshopt…</p>
      {:else if opt.phase === 'error'}
        <p class="err" role="alert">Couldn’t optimize: {opt.error}</p>
      {:else if opt.result}
        {@const r = opt.result}
        {@const b = budgetStatus(r.after)}
        <div class="sizes">
          <div><span class="k">Before</span><strong>{formatBytes(r.before)}</strong></div>
          <Icon name="chevronRight" size={14} />
          <div><span class="k">After</span><strong>{formatBytes(r.after)}</strong></div>
          <div><span class="k">Saved</span><strong>{r.before > 0 ? Math.max(0, Math.round((1 - r.after / r.before) * 100)) : 0}%</strong></div>
        </div>
        <div class="budget {b.tone}" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.min(100, b.pct)} aria-label="Web budget used">
          <span class="bar" style:width="{Math.min(100, b.pct)}%"></span>
        </div>
        <p class="dim">Web budget: {b.label}{b.tone === 'over' ? ' — simplify the scene or split it before embedding.' : '.'}</p>
        <ul class="steps">
          {#each r.steps as s (s)}<li><Icon name="check" size={11} /> {s}</li>{/each}
        </ul>
        <p class="dim">Draco isn’t offered: its encoder isn’t bundled and Otto stays offline. Every Otto viewer decodes meshopt.</p>
      {/if}
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (opt = null)}>Close</button>
      {#if opt?.phase === 'done' && opt.result}
        <button class="btn" onclick={() => opt?.result && saveBlob(new Blob([opt.result.bytes], { type: 'model/gltf-binary' }), glbFileName(`${artifact.title}-web`))}>Download</button>
        <button class="btn primary" disabled={readonly || savingGlb} onclick={() => void saveOptimized()}>{savingGlb ? 'Saving…' : 'Save as GLB design'}</button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .studio3d {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 248px minmax(0, 1fr) 320px;
    container-type: inline-size;
  }
  .left,
  .right {
    min-height: 0;
    background: var(--surface);
    display: flex;
    flex-direction: column;
  }
  .left {
    border-inline-end: 1px solid var(--border);
    overflow-y: auto;
  }
  .right {
    border-inline-start: 1px solid var(--border);
  }
  .hier {
    min-height: 200px;
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
  }
  .hier > :global(*) {
    flex: 1;
  }
  .block {
    padding: 10px 12px;
    border-block-start: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .block.card {
    margin: 10px 12px 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .block.card p {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.45;
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
  .env-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .env-row.check {
    cursor: pointer;
  }
  .env-row input {
    accent-color: var(--accent);
  }
  .env-select {
    flex: 1;
    min-width: 0;
    font: inherit;
    font-size: var(--fs-s);
    color: var(--text);
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
  }
  .views {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .views li {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .view {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
  }
  .view:hover {
    background: var(--hover);
  }
  .view span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .center {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    min-height: 40px;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    background: var(--bg);
  }
  .toolbar .on {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--accent-text);
  }
  .grow {
    flex: 1;
  }
  .agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--accent-text);
  }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
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
      inset-block-end: 64px;
    }
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
  .tabs {
    display: flex;
    gap: 16px;
    padding: 0 16px;
    border-block-end: 1px solid var(--border);
    min-height: 40px;
    align-items: stretch;
  }
  .tabs button {
    appearance: none;
    border: 0;
    border-block-end: 2px solid transparent;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-m);
    padding: 0 2px;
    cursor: pointer;
  }
  .tabs button.active {
    color: var(--text);
    border-block-end-color: var(--accent);
    font-weight: 600;
  }
  .tabs button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .count {
    color: var(--text-dim);
    font-weight: 400;
    margin-inline-start: 2px;
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
    padding: 1px 7px;
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
  .opt {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .sizes {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .sizes div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sizes strong {
    font-size: var(--fs-l);
    font-variant-numeric: tabular-nums;
  }
  .budget {
    height: 8px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .budget .bar {
    display: block;
    height: 100%;
    background: var(--success);
  }
  .budget.warn .bar {
    background: var(--warning);
  }
  .budget.over .bar {
    background: var(--danger);
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-s);
  }
  .steps li {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
  @container (max-width: 980px) {
    .studio3d {
      grid-template-columns: minmax(0, 1fr) 300px;
    }
    .left {
      display: none;
    }
  }
  @container (max-width: 720px) {
    .studio3d {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(360px, 1fr) auto;
      overflow-y: auto;
    }
    .right {
      border-inline-start: 0;
      border-block-start: 1px solid var(--border);
      max-height: 60vh;
    }
  }
</style>
