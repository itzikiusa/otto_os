<script lang="ts">
  // DesignArena — the Product → **Design** tab (design/product-design-arena.md §4).
  // ONE arena for every design artifact of a story: a Figma/Canva-style 2D side
  // (HTML screens in device frames, Excalidraw boards, Mermaid diagrams) and a
  // game-studio-style 3D side (three.js viewport + hierarchy + inspector, Track C's
  // `scene3d/` components), all file-backed `product_attachments` rows.
  //
  //   ┌ ASSETS ──────┬ toolbar: New ▾ · Import · Create with AI · … · Export ▾ ─┬ INSPECTOR ┐
  //   │ groups/rows  │                 VIEWPORT / BOARD                      │ per format │
  //   │ (Hierarchy   │   html: sandboxed iframe in a device frame              │────────────│
  //   │  when a 3D   │   excalidraw: DesignBoard island · mermaid: viewer       │ ASSISTANT  │
  //   │  scene is    │   scene3d: Scene3DViewport · image: <img>                │ (docked    │
  //   │  open)       │   status: N objects · saved 2s ago                       │  shell)    │
  //   └──────────────┴──────────────────────────────────────────────────────────┴────────────┘
  //
  // This component OWNS the in-memory `source` of the open artifact, the 600 ms
  // autosave debounce (through the product store), the dirty/conflict state, and
  // live-update handling: a `mockup_updated` for the open artifact replaces the
  // source — unless local edits are unsaved, in which case it ASKS first (§4.1,
  // "no silent clobber"). Editors (board / inspector / code view) emit every
  // change undebounced. Untrusted content stays isolated: HTML and Mermaid render
  // only inside sandboxed iframes (MockupViewer), scene3d JSON is parsed +
  // shape-checked before it reaches the viewport, and GLBs load only by
  // attachment id through the authed blob helper.
  //
  // ≤ 640 px the three columns collapse to a segmented single pane (Assets ·
  // Canvas · Inspector), like the Database Explorer's phone layout.
  import { onDestroy, untrack } from 'svelte';
  import { product, toBase64 } from '../../../lib/stores/product.svelte';
  import { mockupAssist, type LiveUpdate } from '../../../lib/stores/mockup-assist.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import MockupViewer from '../MockupViewer.svelte';
  import MockupAnnotations from '../MockupAnnotations.svelte';
  import MockupAssistPanel from '../MockupAssistPanel.svelte';
  import DesignBoard from './DesignBoard.svelte';
  import DeviceFrame, { DEVICES, type DeviceKind } from './DeviceFrame.svelte';
  import {
    Scene3DViewport,
    Hierarchy,
    Inspector,
    emptyScene,
    parseScene,
    serializeScene,
    validate,
    exportSceneToGlb,
    type Scene3dDoc,
    type Scene3dObject,
    type ValidationIssue,
  } from './scene3d';
  import {
    FORMATS,
    GROUP_ORDER,
    GLB_MIME,
    GLTF_MIME,
    IMPORT_ACCEPT,
    fileToB64,
    groupOf,
    isDesignAttachment,
    isTextKind,
    kindForUpload,
    kindOf,
    mimeForFile,
    nextFilename,
    typeLabel,
    type ArtifactKind,
  } from './format';
  import { DESIGN_TEMPLATES, blankSource, type DesignTemplate } from './templates';
  import {
    canvasToPng,
    downloadBlob,
    downloadSource,
    excalidrawToPng,
    excalidrawToSvg,
    mermaidToPng,
    mermaidToSvg,
    withExt,
  } from './exporters';
  import { downloadText } from '../../../lib/components/exporters';
  import { ApiError } from '../../../lib/api/client';
  import { DESIGN_FORMATS, type BlenderJob, type BlenderStatus, type DesignFormat, type ProductAttachment, type ProductStory } from '../types';

  // ── Story context ──────────────────────────────────────────────────────────
  const story = $derived(product.detail?.story ?? null);
  const storyId = $derived(product.selectedId);
  /** An epic's arena shows every child's artifacts too (design §3.2). */
  const children = $derived<ProductStory[]>(storyId ? product.childrenOf(storyId) : []);
  const isEpic = $derived(!!story && (story.tree_kind === 'epic' || children.length > 0));

  // ── Attachments ────────────────────────────────────────────────────────────
  let ownAtts = $state<ProductAttachment[]>([]);
  /** child story id → its attachments (epic mode only). */
  let childAtts = $state<Record<string, ProductAttachment[]>>({});
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  /** Epic mode: `''` = everything, a story id = only that story's artifacts. */
  let childFilter = $state('');
  let collapsedGroups = $state<Record<string, boolean>>({});

  /** Every listed artifact with its owning story (the epic itself or a child). */
  const rows = $derived.by(() => {
    const out: { att: ProductAttachment; owner: ProductStory | null }[] = [];
    for (const a of ownAtts) if (isDesignAttachment(a)) out.push({ att: a, owner: story });
    if (isEpic) {
      for (const c of children) {
        for (const a of childAtts[c.id] ?? []) if (isDesignAttachment(a)) out.push({ att: a, owner: c });
      }
    }
    return out;
  });
  const visibleRows = $derived(
    childFilter ? rows.filter((r) => (r.owner?.id ?? storyId) === childFilter) : rows,
  );
  /** Grouped for the asset list: known groups in GROUP_ORDER, unknown after. */
  const groups = $derived.by(() => {
    const byName = new Map<string, typeof visibleRows>();
    for (const r of visibleRows) {
      const g = groupOf(r.att);
      const list = byName.get(g) ?? [];
      list.push(r);
      byName.set(g, list);
    }
    const names = [...byName.keys()].sort((a, b) => {
      const ia = GROUP_ORDER.indexOf(a);
      const ib = GROUP_ORDER.indexOf(b);
      return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib) || a.localeCompare(b);
    });
    return names.map((name) => ({ name, rows: byName.get(name)! }));
  });

  let selectedId = $state<string | null>(null);
  const selected = $derived(rows.find((r) => r.att.id === selectedId) ?? null);
  const att = $derived(selected?.att ?? null);
  const kind = $derived<ArtifactKind>(att ? kindOf(att) : 'other');
  const isText = $derived(isTextKind(kind));

  // ── The open artifact's source (text formats) ──────────────────────────────
  let source = $state('');
  let sourceLoading = $state(false);
  let sourceError = $state<string | null>(null);
  /** Bumped when a binary artifact's bytes changed (remounts the <img>/model). */
  let binaryTick = $state(0);
  let loadToken = 0;
  /** scene3d code-view edits that fail the client validator are shown but NOT
   *  sent (the issues list in the stage explains why). */
  let localInvalid = $state<ValidationIssue[] | null>(null);

  /** scene3d: the VALIDATED doc (Track C's validator mirrors the Rust one —
   *  known types, finite numbers, bounded arrays, safe ids) or the issues. */
  const sceneParse = $derived.by<{ doc: Scene3dDoc | null; issues: ValidationIssue[] }>(() => {
    if (kind !== 'scene3d' || !source.trim()) return { doc: null, issues: [] };
    const r = parseScene(source);
    return { doc: r.doc, issues: r.issues };
  });
  const sceneDoc = $derived(sceneParse.doc);
  /** A standalone glb/gltf shows through a one-object scene (design §2.2). */
  const modelDoc = $derived.by<Scene3dDoc | null>(() => {
    if (kind !== 'model' || !att) return null;
    const base = emptyScene();
    const hero: Scene3dObject = {
      id: 'model', name: att.filename, type: 'gltf', attachment_id: att.id,
      position: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1],
    };
    return { ...base, objects: [...base.objects, hero] };
  });

  // ── View options ───────────────────────────────────────────────────────────
  let device = $state<DeviceKind>(loadPref('product.design.device', 'none') as DeviceKind);
  let scheme = $state<'light' | 'dark'>(loadPref('product.design.scheme', 'light') as 'light' | 'dark');
  let allowScripts = $state(false);
  /** Board: Annotate mode makes the board read-only and mounts the pin overlay. */
  let annotate = $state(false);
  let codeView = $state(false);
  /** Left pane while a scene is open: asset list or the scene Hierarchy. */
  let leftPane = $state<'assets' | 'hierarchy'>('assets');
  let scene3dSel = $state<string | null>(null);
  let play = $state(false);
  /** ≤ 640 px: which of the three panes is showing. */
  let mobilePane = $state<'assets' | 'canvas' | 'inspector'>('assets');
  let importing = $state(false);
  /** How the Assistant is shown: creating (full stage, own live preview) or
   *  refining the open artifact (docked in the inspector column). */
  let assistMode = $state<'create' | 'refine' | null>(null);
  let boardBox = $state<HTMLDivElement | null>(null);
  let viewportEl = $state<HTMLDivElement | null>(null);
  /** The mounted 3D viewport instance (exposes `snapshotPng()`). */
  let sceneView = $state<{ snapshotPng: () => Promise<Blob | null> } | null>(null);
  let glbInput = $state<HTMLInputElement | null>(null);

  function loadPref(key: string, dflt: string): string {
    try {
      return localStorage.getItem(key) ?? dflt;
    } catch {
      return dflt;
    }
  }
  function savePref(key: string, v: string): void {
    try {
      localStorage.setItem(key, v);
    } catch {
      /* storage unavailable — non-fatal */
    }
  }
  $effect(() => savePref('product.design.device', device));
  $effect(() => savePref('product.design.scheme', scheme));

  // ── Blender bridge (§4.4) ──────────────────────────────────────────────────
  let blender = $state<BlenderStatus | null>(null);
  let blenderChecked = false;
  let renderJob = $state<BlenderJob | null>(null);
  let rendering = $state(false);

  // ── Save status line ───────────────────────────────────────────────────────
  const saveState = $derived(att ? product.saveState[att.id] ?? 'saved' : 'saved');
  let now = $state(Date.now());
  const ticker = setInterval(() => (now = Date.now()), 5000);
  onDestroy(() => clearInterval(ticker));
  const saveLabel = $derived.by(() => {
    if (!att) return '';
    if (localInvalid) return `invalid document — not saved (${localInvalid.length} issue${localInvalid.length === 1 ? '' : 's'})`;
    switch (saveState) {
      case 'dirty': return 'unsaved';
      case 'saving': return 'saving…';
      case 'error': return 'save failed';
      case 'conflict': return 'conflict — newer version on the server';
      default: {
        const t = product.savedAt[att.id];
        if (!t) return 'saved';
        const s = Math.max(0, Math.round((now - t) / 1000));
        return s < 5 ? 'saved just now' : s < 60 ? `saved ${s}s ago` : `saved ${Math.round(s / 60)}m ago`;
      }
    }
  });
  const countLabel = $derived.by(() => {
    if (!att) return '';
    if (kind === 'scene3d' && sceneDoc) return `${sceneDoc.objects.length} objects · ${sceneDoc.lights.length} lights`;
    if (kind === 'excalidraw') {
      try {
        const n = (JSON.parse(source) as { elements?: unknown[] }).elements?.length ?? 0;
        return `${n} elements`;
      } catch {
        return 'board';
      }
    }
    if (isText) return `${source.length.toLocaleString()} chars`;
    return `${(att.size_bytes / 1024).toFixed(1)} KB`;
  });

  // ── Loading ────────────────────────────────────────────────────────────────
  // Re-run whenever the selected story changes. The Assistant is a global
  // singleton — if it's still open for a DIFFERENT story, close it so a turn
  // can't target the wrong story.
  $effect(() => {
    const sid = product.selectedId;
    if (mockupAssist.active && mockupAssist.storyId !== sid) {
      mockupAssist.close();
      assistMode = null;
    }
    childFilter = '';
    void untrack(() => loadAll());
  });
  // Children may arrive after the story (list reload) — refetch their artifacts.
  $effect(() => {
    const ids = children.map((c) => c.id).join(',');
    void untrack(() => loadChildren(ids ? children : []));
  });
  onDestroy(() => {
    mockupAssist.close();
    const id = selectedId;
    // Flush the pending edit first, THEN release every cached blob URL + base.
    void (id ? product.flushAttachmentContent(id).catch(() => null) : Promise.resolve()).finally(() =>
      product.teardown(),
    );
  });

  let listReloadTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleListReload(): void {
    if (listReloadTimer) clearTimeout(listReloadTimer);
    listReloadTimer = setTimeout(() => void loadAll(true), 300);
  }

  async function loadAll(quiet = false): Promise<void> {
    if (!product.selectedId) return;
    if (!quiet) loading = true;
    loadError = null;
    try {
      ownAtts = await product.listAttachments();
      await loadChildren(children);
      // Keep a valid selection: default to the first artifact if none chosen or
      // the previously-selected one is gone.
      if (!selectedId || !rows.some((r) => r.att.id === selectedId)) {
        selectedId = rows[0]?.att.id ?? null;
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
      if (!quiet) toasts.error('Could not load design artifacts', loadError);
    } finally {
      loading = false;
    }
  }
  async function loadChildren(list: ProductStory[]): Promise<void> {
    if (!list.length) {
      childAtts = {};
      return;
    }
    const entries = await Promise.all(
      list.map(async (c) => [c.id, await product.listAttachmentsOf(c.id).catch(() => [])] as const),
    );
    childAtts = Object.fromEntries(entries);
  }

  // Load the open artifact's bytes whenever the selection changes; flush the
  // previous artifact's pending edit first so switching never loses work.
  let prevSelected: string | null = null;
  $effect(() => {
    const a = att;
    const id = a?.id ?? null;
    if (id === prevSelected) return;
    const prev = prevSelected;
    prevSelected = id;
    untrack(() => {
      if (prev) void product.flushAttachmentContent(prev).catch(() => {});
      scene3dSel = null;
      play = false;
      annotate = false;
      codeView = false;
      allowScripts = false;
      leftPane = a && kindOf(a) === 'scene3d' ? 'hierarchy' : 'assets';
      void loadSource(a);
    });
  });

  async function loadSource(a: ProductAttachment | null): Promise<void> {
    const token = ++loadToken;
    source = '';
    sourceError = null;
    if (!a || !isTextKind(kindOf(a))) return;
    sourceLoading = true;
    localInvalid = null;
    try {
      const text = await product.attachmentText(a.id);
      if (token === loadToken) {
        source = text;
        // Optimistic-concurrency base: the row we just loaded.
        product.setContentBase(a.id, a.updated_at);
      }
    } catch (e) {
      if (token === loadToken) sourceError = e instanceof Error ? e.message : String(e);
    } finally {
      if (token === loadToken) sourceLoading = false;
    }
  }

  // ── Live updates (mockup_updated) ─────────────────────────────────────────
  $effect(() => {
    const u = mockupAssist.lastUpdate;
    if (!u) return;
    untrack(() => void handleLive(u));
  });
  async function handleLive(u: LiveUpdate): Promise<void> {
    const related = u.storyId === storyId || children.some((c) => c.id === u.storyId);
    if (!related) return;
    const known = rows.some((r) => r.att.id === u.attachmentId);
    if (!known) {
      // A brand-new artifact (assistant create, Blender output, a swarm agent's
      // otto-mockup) — refresh the list so it appears.
      scheduleListReload();
      return;
    }
    if (u.attachmentId !== att?.id) {
      scheduleListReload();
      return;
    }
    const aid = u.attachmentId;
    if (!isText) {
      // Binary bytes changed (a re-rendered PNG, a replaced GLB): drop the cached
      // blob URL and remount the viewer.
      product.invalidateBlobUrl(aid);
      binaryTick++;
      scheduleListReload();
      return;
    }
    let incoming = u.content;
    if (incoming === null) {
      try {
        incoming = await product.attachmentText(aid);
      } catch {
        return;
      }
    }
    if (incoming === source) return; // the echo of our own save
    if (product.hasUnsavedContent(aid)) {
      const ok = await confirmer.ask(
        'This artifact was just changed on the server (by the agent or another editor) while you have unsaved edits here. Replace your edits with the new version?',
        { title: 'Newer version available', confirmLabel: 'Replace mine', danger: false },
      );
      if (!ok) {
        product.markConflict(aid);
        return;
      }
      product.discardPendingContent(aid);
    }
    source = incoming;
    localInvalid = null;
    // The row's new `updated_at` becomes our base once the list refresh lands
    // (see the `$effect` on att.updated_at below).
    scheduleListReload();
  }
  // Whenever the open row's `updated_at` changes on the server AND we hold no
  // unsaved edits, that row is our new base (a save of ours also sets it).
  $effect(() => {
    const a = att;
    if (!a) return;
    const ts = a.updated_at;
    untrack(() => {
      if (!product.hasUnsavedContent(a.id)) product.setContentBase(a.id, ts);
    });
  });

  /** A save came back 409: the row moved on. Take theirs (re-fetch) or keep
   *  mine (overwrite against the fresh base) — never a silent clobber. */
  async function resolveConflict(a: ProductAttachment, mine: string): Promise<void> {
    const takeTheirs = await confirmer.ask(
      'Saving failed: this artifact was changed on the server since you loaded it (another editor or the agent). Take the server version (your edits are dropped) or keep yours and overwrite it?',
      { title: 'Save conflict', confirmLabel: 'Take theirs', danger: false },
    );
    try {
      const fresh = (await product.listAttachmentsOf(a.story_id)).find((x) => x.id === a.id);
      if (!fresh) return;
      if (takeTheirs) {
        product.discardPendingContent(a.id);
        product.setContentBase(a.id, fresh.updated_at);
        await loadSource(fresh);
      } else {
        await product.overwriteAttachmentContent(a.id, mine, fresh.updated_at);
      }
      scheduleListReload();
    } catch (e) {
      toasts.error('Could not resolve the conflict', e instanceof Error ? e.message : String(e));
    }
  }

  // ── Local edits → autosave ─────────────────────────────────────────────────
  function applyLocalEdit(next: string): void {
    if (!att) return;
    const a = att;
    source = next;
    // scene3d: run the client validator BEFORE the wire — an invalid document
    // stays local (issues shown in the stage) instead of being PUT for a 400.
    if (kind === 'scene3d') {
      const r = parseScene(next);
      if (!r.ok) {
        localInvalid = r.issues;
        return;
      }
    }
    localInvalid = null;
    void product.saveAttachmentContent(a.id, next).catch((e) => {
      if (e instanceof ApiError && e.status === 409) {
        void resolveConflict(a, next);
        return;
      }
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    });
  }
  function onBoardChange(src: string): void {
    applyLocalEdit(src);
  }
  function onSceneChange(doc: Scene3dDoc): void {
    const r = validate(doc);
    if (!r.ok) {
      // The viewport/hierarchy/inspector go through C's ops, so this is a
      // programming error rather than user input — surface it, don't save it.
      localInvalid = r.issues;
      toasts.error('Scene edit rejected by the validator', r.issues[0]?.message ?? 'invalid document');
      return;
    }
    applyLocalEdit(serializeScene(r.doc));
  }
  function onCodeInput(e: Event): void {
    applyLocalEdit((e.currentTarget as HTMLTextAreaElement).value);
  }

  // ── Create / import ────────────────────────────────────────────────────────
  async function createArtifact(format: DesignFormat, src: string, filename?: string): Promise<void> {
    if (!storyId) return;
    try {
      const created = await product.uploadAttachment({
        filename: filename ?? nextFilename(format, rows.map((r) => r.att)),
        mime: FORMATS[format].mime,
        kind: kindForUpload(FORMATS[format].mime),
        data_b64: toBase64(src),
      });
      await loadAll(true);
      selectedId = created.id;
      mobilePane = 'canvas';
    } catch (e) {
      toasts.error('Could not create the artifact', e instanceof Error ? e.message : String(e));
    }
  }
  function createBlank(format: DesignFormat): void {
    void createArtifact(format, blankSource(format, emptyScene));
  }
  function createFromTemplate(t: DesignTemplate): void {
    const names = new Set(rows.map((r) => (r.att.filename || '').toLowerCase()));
    const filename = names.has(t.filename.toLowerCase()) ? nextFilename(t.format, rows.map((r) => r.att)) : t.filename;
    void createArtifact(t.format, t.source, filename);
  }
  /** Create with AI: open the in-place design agent for a NEW artifact. */
  function createWithAi(format: DesignFormat): void {
    if (!storyId) return;
    mockupAssist.openNew(storyId, format);
    assistMode = 'create';
    mobilePane = 'canvas';
  }
  /** Manual import: upload picked file(s), then select the first. */
  async function importFiles(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const files = input.files ? Array.from(input.files) : [];
    input.value = '';
    if (!files.length) return;
    importing = true;
    let firstId: string | null = null;
    try {
      for (const f of files) {
        try {
          const mime = mimeForFile(f);
          const a = await product.uploadAttachment({
            filename: f.name,
            mime,
            kind: kindForUpload(mime, f.name),
            data_b64: await fileToB64(f),
          });
          firstId ??= a.id;
        } catch (err) {
          toasts.error(`Import failed: ${f.name}`, err instanceof Error ? err.message : String(err));
        }
      }
      await loadAll(true);
      if (firstId) selectedId = firstId;
    } finally {
      importing = false;
    }
  }
  /** Hierarchy → "import GLB": upload the model and add a `gltf` object to the doc. */
  async function importGlb(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const f = input.files?.[0];
    input.value = '';
    if (!f || !sceneDoc) return;
    try {
      const a = await product.uploadAttachment({
        filename: f.name,
        mime: f.name.toLowerCase().endsWith('.gltf') ? GLTF_MIME : GLB_MIME,
        kind: 'design',
        data_b64: await fileToB64(f),
      });
      const id = `gltf-${a.id.slice(0, 8)}`;
      const obj: Scene3dObject = {
        id, name: f.name.replace(/\.(glb|gltf)$/i, ''), type: 'gltf', attachment_id: a.id,
        position: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1],
      };
      onSceneChange({ ...sceneDoc, objects: [...sceneDoc.objects, obj] });
      scene3dSel = id;
      await loadAll(true);
    } catch (err) {
      toasts.error('GLB import failed', err instanceof Error ? err.message : String(err));
    }
  }

  // ── Menus (the global ctxMenu store — already viewport-clamped) ────────────
  function newMenu(e: MouseEvent): void {
    const items: MenuItem[] = DESIGN_FORMATS.map((f) => ({
      label: `${FORMATS[f].label}${f === 'html' ? ' (HTML screen)' : f === 'mermaid' ? ' (Mermaid)' : f === 'excalidraw' ? ' (Excalidraw)' : ''}`,
      icon: FORMATS[f].icon,
      action: () => createBlank(f),
    }));
    items.push({ separator: true });
    for (const t of DESIGN_TEMPLATES) {
      items.push({ label: `Template: ${t.name}`, icon: 'layers', action: () => createFromTemplate(t) });
    }
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'New…', maxVisible: 14 });
  }
  function aiMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      DESIGN_FORMATS.map((f) => ({
        label: f === 'html' ? 'HTML screen' : f === 'mermaid' ? 'Diagram' : FORMATS[f].label,
        icon: FORMATS[f].icon,
        action: () => createWithAi(f),
      })),
    );
  }
  function exportMenu(e: MouseEvent): void {
    if (!att) return;
    const a = att;
    const items: MenuItem[] = [];
    if (kind === 'mermaid') {
      items.push({ label: 'PNG', icon: 'image', action: () => void mermaidToPng(source, withExt(a, 'png')).catch(fail) });
      items.push({ label: 'SVG', icon: 'image', action: () => void mermaidToSvg(source).then((s) => downloadText(s, withExt(a, 'svg'), 'image/svg+xml')).catch(fail) });
    } else if (kind === 'excalidraw') {
      items.push({ label: 'PNG', icon: 'image', action: () => void excalidrawToPng(source, scheme === 'dark').then((b) => downloadBlob(b, withExt(a, 'png'))).catch(fail) });
      items.push({ label: 'SVG', icon: 'image', action: () => void excalidrawToSvg(source, scheme === 'dark').then((s) => downloadText(s, withExt(a, 'svg'), 'image/svg+xml')).catch(fail) });
    } else if (kind === 'scene3d') {
      items.push({
        label: 'PNG (viewport snapshot)', icon: 'image',
        action: () => {
          // Prefer the viewport's own snapshot (re-renders first); fall back to
          // scraping the <canvas> when the instance isn't up yet.
          if (sceneView) {
            void sceneView.snapshotPng().then((b) => {
              if (b) downloadBlob(b, withExt(a, 'png'));
              else if (!viewportEl || !canvasToPng(viewportEl, withExt(a, 'png'))) toasts.warn('No viewport to snapshot yet');
            }).catch(fail);
          } else if (!viewportEl || !canvasToPng(viewportEl, withExt(a, 'png'))) {
            toasts.warn('No viewport to snapshot yet');
          }
        },
      });
      items.push({
        label: 'GLB (three GLTFExporter)', icon: 'box',
        action: () => {
          if (!sceneDoc) return;
          void exportSceneToGlb(sceneDoc, resolveAttachment)
            .then(({ blob, skipped }) => {
              downloadBlob(blob, withExt(a, 'glb'));
              if (skipped.length) toasts.warn('Some models were skipped', skipped.join(', '));
            })
            .catch(fail);
        },
      });
      items.push({ label: 'Blender script (.py)', icon: 'file', action: () => void downloadBlenderScript() });
    }
    if (isText) items.push({ label: 'Source file', icon: 'file', action: () => downloadSource(a, source) });
    else items.push({ label: 'Download file', icon: 'file', action: () => void product.attachmentBlobUrl(a.id).then((u) => fetch(u).then((r) => r.blob())).then((b) => downloadBlob(b, a.filename)).catch(fail) });
    ctxMenu.show(e, items);
  }
  function fail(e: unknown): void {
    toasts.error('Export failed', e instanceof Error ? e.message : String(e));
  }
  function rowMenu(e: MouseEvent | KeyboardEvent, r: { att: ProductAttachment; owner: ProductStory | null }): void {
    const k = kindOf(r.att);
    const items: MenuItem[] = [
      { label: 'Open', icon: 'eye', action: () => selectArtifact(r.att.id) },
      { label: 'Rename…', icon: 'edit', action: () => void rename(r.att) },
    ];
    if (isTextKind(k)) items.push({ label: 'Refine with AI', icon: 'zap', action: () => void refine(r.att) });
    items.push({ separator: true });
    items.push({ label: 'Delete', icon: 'trash', danger: true, action: () => void remove(r.att) });
    ctxMenu.show(e, items);
  }
  function moreMenu(e: MouseEvent): void {
    if (!att) return;
    const items: MenuItem[] = [
      { label: codeView ? 'Hide source' : 'Show source', icon: 'file', disabled: !isText, action: () => (codeView = !codeView) },
      { label: 'Rename…', icon: 'edit', action: () => void rename(att!) },
      { label: 'Refresh list', icon: 'refresh', action: () => void loadAll() },
      { separator: true },
      { label: 'Delete artifact', icon: 'trash', danger: true, action: () => void remove(att!) },
    ];
    ctxMenu.show(e, items);
  }

  // ── Row actions ────────────────────────────────────────────────────────────
  function selectArtifact(id: string): void {
    selectedId = id;
    mobilePane = 'canvas';
    // Leaving the full-stage create panel: keep the agent docked when the picked
    // row IS the artifact it's working on, otherwise close it.
    if (assistMode === 'create') {
      if (mockupAssist.attachmentId === id) assistMode = 'refine';
      else closeAssist();
    }
  }
  async function rename(a: ProductAttachment): Promise<void> {
    const name = await confirmer.promptText('New file name (the extension must stay the same):', {
      title: 'Rename artifact', confirmLabel: 'Rename', initial: a.filename,
    });
    if (!name || name === a.filename) return;
    try {
      await product.patchAttachment(a.id, { filename: name });
      await loadAll(true);
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }
  async function remove(a: ProductAttachment): Promise<void> {
    const ok = await confirmer.ask(`Delete "${a.filename}"? Its pinned annotations go with it.`, {
      title: 'Delete artifact', confirmLabel: 'Delete', danger: true,
    });
    if (!ok) return;
    try {
      product.discardPendingContent(a.id);
      await product.deleteAttachment(a.id);
      if (selectedId === a.id) selectedId = null;
      await loadAll(true);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }
  /** Refine an existing artifact with the in-place agent (docked panel). */
  function refine(a: ProductAttachment): void {
    selectedId = a.id;
    void mockupAssist.openRefine(a);
    assistMode = 'refine';
    mobilePane = 'inspector';
  }
  /** A turn committed — reload the list and select the committed artifact. */
  async function onAssistCommit(a: ProductAttachment): Promise<void> {
    await loadAll(true);
    selectedId = a.id;
    if (assistMode === 'refine') void loadSource(a);
  }
  function closeAssist(): void {
    mockupAssist.close();
    assistMode = null;
    void loadAll(true);
  }
  /** Creating a NEW artifact = the full-stage panel (its own live preview) until
   *  the user opens a row or closes it; a refine is docked beside the viewport. */
  const assistCreating = $derived(mockupAssist.active && assistMode === 'create');
  const assistDocked = $derived(mockupAssist.active && assistMode === 'refine');

  // ── Blender ────────────────────────────────────────────────────────────────
  $effect(() => {
    if (kind === 'scene3d' && !blenderChecked) {
      blenderChecked = true;
      void product.blenderStatus().then((s) => (blender = s)).catch(() => (blender = { installed: false, path: null, version: null }));
    }
  });
  async function downloadBlenderScript(): Promise<void> {
    if (!att) return;
    try {
      const py = await product.blenderScript(att.story_id, att.id);
      downloadText(py, withExt(att, 'py'), 'text/x-python');
    } catch (e) {
      fail(e);
    }
  }
  async function renderInBlender(): Promise<void> {
    if (!att || rendering) return;
    rendering = true;
    try {
      await product.flushAttachmentContent(att.id);
      const { id } = await product.blenderRender(att.story_id, att.id);
      renderJob = await product.blenderJob(id);
      while (renderJob.status === 'queued' || renderJob.status === 'running') {
        await new Promise((r) => setTimeout(r, 1500));
        renderJob = await product.blenderJob(id);
      }
      if (renderJob.status === 'done') {
        toasts.success('Blender render finished', `${renderJob.outputs.length} file(s) attached`);
        await loadAll(true);
      } else {
        toasts.error('Blender render failed', renderJob.error ?? 'unknown error');
      }
    } catch (e) {
      toasts.error('Blender render failed', e instanceof Error ? e.message : String(e));
    } finally {
      rendering = false;
    }
  }

  const resolveAttachment = (aid: string) => product.attachmentBlobUrl(aid);
  function fmtBytes(n: number): string {
    return n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`;
  }
</script>

<!-- `.mockups-tab` is kept as an alias class: the product-mockups E2E + deep links target it. -->
<div
  class="design-arena mockups-tab"
  class:m-assets={mobilePane === 'assets'}
  class:m-canvas={mobilePane === 'canvas'}
  class:m-inspector={mobilePane === 'inspector'}
>
  <!-- ≤640px segmented pane switch (Database Explorer idiom) -->
  <div class="arena-seg" role="tablist" aria-label="Design panes">
    <button class="seg" class:active={mobilePane === 'assets'} role="tab" aria-selected={mobilePane === 'assets'} onclick={() => (mobilePane = 'assets')}>Assets</button>
    <button class="seg" class:active={mobilePane === 'canvas'} role="tab" aria-selected={mobilePane === 'canvas'} onclick={() => (mobilePane = 'canvas')}>Canvas</button>
    <button class="seg" class:active={mobilePane === 'inspector'} role="tab" aria-selected={mobilePane === 'inspector'} onclick={() => (mobilePane = 'inspector')}>Inspector</button>
  </div>

  <!-- ── ASSETS / HIERARCHY ─────────────────────────────────────────────── -->
  <aside class="arena-assets list-pane">
    <div class="pane-head">
      {#if kind === 'scene3d' && sceneDoc}
        <div class="pane-switch" role="tablist" aria-label="Left pane">
          <button class="ss" class:active={leftPane === 'assets'} role="tab" aria-selected={leftPane === 'assets'} onclick={() => (leftPane = 'assets')}>Assets</button>
          <button class="ss" class:active={leftPane === 'hierarchy'} role="tab" aria-selected={leftPane === 'hierarchy'} onclick={() => (leftPane = 'hierarchy')}>Hierarchy</button>
        </div>
      {:else}
        <span class="pane-title">Assets</span>
      {/if}
      <div class="list-actions">
        <button class="p-btn" onclick={newMenu} title="New artifact (blank or from a template)">
          <Icon name="plus" size={12} /> New <Icon name="chevronDown" size={10} />
        </button>
        <label class="p-btn" title="Import files: HTML, images, SVG, .mmd, .excalidraw, .glb / .gltf">
          <Icon name="arrowUp" size={12} />
          {importing ? 'Importing…' : 'Import'}
          <input type="file" multiple accept={IMPORT_ACCEPT} style="display:none" onchange={importFiles} disabled={importing} />
        </label>
        <button class="act-btn p-btn primary" onclick={aiMenu} title="Generate an artifact with AI">
          <Icon name="zap" size={12} /> Create with AI
        </button>
      </div>
    </div>

    {#if isEpic && children.length > 0 && leftPane === 'assets'}
      <!-- The epic is the single place to review the whole feature: filter by child. -->
      <label class="child-filter">
        <span>Show</span>
        <select bind:value={childFilter} aria-label="Filter artifacts by child story">
          <option value="">Epic + all children</option>
          <option value={storyId}>Epic only</option>
          {#each children as c (c.id)}
            <option value={c.id}>{c.folder ? `${c.folder} / ` : ''}{c.title}</option>
          {/each}
        </select>
      </label>
    {/if}

    <div class="pane-body">
      {#if leftPane === 'hierarchy' && sceneDoc}
        <Hierarchy
          doc={sceneDoc}
          bind:selectedId={scene3dSel}
          onchange={onSceneChange}
          onimportGlb={() => glbInput?.click()}
        />
        <input bind:this={glbInput} type="file" accept=".glb,.gltf,{GLB_MIME},{GLTF_MIME}" style="display:none" onchange={importGlb} />
      {:else if loading}
        <div class="list-empty">Loading…</div>
      {:else if loadError}
        <div class="list-empty err">{loadError}</div>
      {:else if rows.length === 0}
        <EmptyState
          icon="layers"
          title="No design artifacts yet"
          body="New ▾ for a blank screen, board, diagram or 3D scene (or a template); Import a file; or Create with AI to have a specialized agent build one right here."
        />
      {:else}
        {#each groups as g (g.name)}
          <button class="group-head" onclick={() => (collapsedGroups = { ...collapsedGroups, [g.name]: !collapsedGroups[g.name] })} aria-expanded={!collapsedGroups[g.name]}>
            <Icon name={collapsedGroups[g.name] ? 'chevronRight' : 'chevronDown'} size={11} />
            <span>{g.name}</span>
            <span class="group-count">{g.rows.length}</span>
          </button>
          {#if !collapsedGroups[g.name]}
            {#each g.rows as r (r.att.id)}
              <div
                class="mockup-row"
                class:active={selectedId === r.att.id}
                role="presentation"
                oncontextmenu={(e) => rowMenu(e, r)}
              >
                <button class="mockup-open" onclick={() => selectArtifact(r.att.id)} title={r.att.filename}>
                  <span class="mockup-type">{typeLabel(r.att)}</span>
                  <span class="mockup-name">{r.att.filename}</span>
                  {#if isEpic && r.owner && r.owner.id !== storyId}
                    <span class="owner-badge" title={`From child: ${r.owner.title}`}>{r.owner.folder || r.owner.title}</span>
                  {/if}
                  {#if r.att.source === 'agent'}<span class="agent-badge">agent</span>{/if}
                </button>
                {#if r.att.source === 'agent'}
                  <button class="refine-btn" onclick={() => refine(r.att)} title="Refine with AI" aria-label="Refine with AI">
                    <Icon name="zap" size={12} />
                  </button>
                {/if}
                <button class="row-more" onclick={(e) => rowMenu(e, r)} aria-label="Artifact menu" title="More">
                  <Icon name="grip" size={12} />
                </button>
              </div>
            {/each}
          {/if}
        {/each}
      {/if}
    </div>
  </aside>

  <!-- ── VIEWPORT / BOARD (the "stage") ─────────────────────────────────── -->
  <div class="arena-stage mockup-stage">
    {#if assistCreating}
      <MockupAssistPanel oncommit={onAssistCommit} onclose={closeAssist} />
    {:else if att}
      <div class="stage-toolbar">
        <span class="st-name" title={att.filename}>{att.filename}</span>
        <span class="st-type">{typeLabel(att)}</span>
        {#if kind === 'html'}
          <div class="seg-group" role="group" aria-label="Device frame">
            {#each DEVICES as d (d.id)}
              <button class="seg" class:active={device === d.id} onclick={() => (device = d.id)}>{d.label}</button>
            {/each}
          </div>
          <button class="tb-btn" onclick={() => (scheme = scheme === 'dark' ? 'light' : 'dark')} title="Toggle light / dark backdrop" aria-label="Toggle light / dark">
            <Icon name={scheme === 'dark' ? 'eye' : 'eyeOff'} size={12} /> {scheme === 'dark' ? 'Dark' : 'Light'}
          </button>
        {/if}
        {#if kind === 'excalidraw'}
          <button class="tb-btn" class:on={annotate} onclick={() => (annotate = !annotate)} title="Annotate: pin comments on the board (read-only while on)">
            <Icon name="pin" size={12} /> Annotate
          </button>
        {/if}
        {#if kind === 'scene3d'}
          <button class="tb-btn" class:on={play} onclick={() => (play = !play)} title="Play: presentation view (doc camera, no gizmo/grid)">
            <Icon name="play" size={12} /> Play
          </button>
        {/if}
        {#if isText}
          <button class="tb-btn" class:on={codeView} onclick={() => (codeView = !codeView)} title="Edit the source">
            <Icon name="file" size={12} /> Source
          </button>
        {/if}
        <span class="st-grow"></span>
        <button class="tb-btn" onclick={exportMenu} title="Export"><Icon name="arrowDown" size={12} /> Export <Icon name="chevronDown" size={10} /></button>
        <button class="tb-btn icon" onclick={moreMenu} aria-label="More" title="More"><Icon name="grip" size={13} /></button>
      </div>

      <div class="stage-body" class:split={codeView && isText}>
        <div class="viewport" bind:this={viewportEl}>
          {#if sourceLoading}
            <div class="stage-msg">Loading…</div>
          {:else if sourceError}
            <div class="stage-msg err">{sourceError}</div>
          {:else if kind === 'html'}
            <DeviceFrame {device} {scheme}>
              <MockupViewer attachment={att} {source} hideToolbar bind:allowScripts />
            </DeviceFrame>
          {:else if kind === 'mermaid'}
            <MockupViewer attachment={att} {source} hideToolbar />
          {:else if kind === 'image'}
            {#key `${att.id}:${binaryTick}`}
              <MockupViewer attachment={att} hideToolbar />
            {/key}
          {:else if kind === 'excalidraw'}
            <div class="render-wrap">
              <div class="render-box board-box" bind:this={boardBox}>
                <DesignBoard {source} readonly={annotate} onchange={onBoardChange} />
              </div>
              {#if annotate && boardBox}
                <MockupAnnotations attachmentId={att.id} box={boardBox} />
              {/if}
            </div>
          {:else if kind === 'scene3d'}
            {#if sceneDoc}
              <Scene3DViewport
                bind:this={sceneView}
                doc={sceneDoc}
                bind:selectedId={scene3dSel}
                {play}
                onchange={onSceneChange}
                {resolveAttachment}
              />
            {:else}
              <div class="stage-msg err">
                <p>Not a valid scene3d document — fix it in the <strong>Source</strong> view.</p>
                {#if sceneParse.issues.length}
                  <ul class="issues">
                    {#each sceneParse.issues.slice(0, 12) as i (i.path + i.message)}
                      <li><span class="mono">{i.path || '(root)'}</span> — {i.message}</li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/if}
          {:else if kind === 'model' && modelDoc}
            {#key `${att.id}:${binaryTick}`}
              <Scene3DViewport doc={modelDoc} readonly {play} onchange={() => {}} {resolveAttachment} />
            {/key}
          {:else}
            <div class="stage-msg err">Unsupported artifact type: {att.mime || 'unknown'}</div>
          {/if}
        </div>
        {#if codeView && isText}
          <!-- Code view: the raw source, autosaved like every other editor. -->
          <textarea class="code-view" spellcheck="false" value={source} oninput={onCodeInput} aria-label="Artifact source"></textarea>
        {/if}
      </div>

      <div class="stage-status" data-save-state={saveState}>
        <span>{countLabel}</span>
        <span class="dot">·</span>
        <span class="save" class:warn={saveState === 'conflict' || saveState === 'error' || !!localInvalid}>{saveLabel}</span>
        {#if saveState === 'conflict'}
          <button class="link" onclick={() => att && resolveConflict(att, source)}>Resolve…</button>
        {/if}
        {#if saveState === 'error'}
          <button class="link" onclick={() => att && applyLocalEdit(source)}>Retry save</button>
        {/if}
      </div>
    {:else if !loading}
      <div class="stage-empty">
        <Icon name="layers" size={28} />
        <p>Select an artifact to view and edit it, use <strong>New ▾</strong> for a blank one or a template, or
          <strong>Create with AI</strong> to generate one in place.</p>
      </div>
    {/if}
  </div>

  <!-- ── INSPECTOR + ASSISTANT ──────────────────────────────────────────── -->
  <aside class="arena-inspector" hidden={assistCreating}>
    <div class="insp-scroll">
      <div class="insp-head">Inspector</div>
      {#if att}
        <section class="insp-sec">
          <div class="insp-row"><span class="k">File</span><span class="v mono" title={att.filename}>{att.filename}</span></div>
          <div class="insp-row"><span class="k">Type</span><span class="v">{typeLabel(att)} <small class="mono">{att.mime}</small></span></div>
          <div class="insp-row"><span class="k">Size</span><span class="v">{fmtBytes(att.size_bytes)}</span></div>
          <div class="insp-row"><span class="k">Group</span><span class="v">{groupOf(att)}</span></div>
          {#if selected?.owner && selected.owner.id !== storyId}
            <div class="insp-row"><span class="k">Story</span><span class="v">{selected.owner.title}</span></div>
          {/if}
          <div class="insp-row"><span class="k">Source</span><span class="v">{att.source}</span></div>
        </section>

        {#if kind === 'html'}
          <section class="insp-sec">
            <div class="insp-title">Screen</div>
            <label class="insp-check">
              <input type="checkbox" bind:checked={allowScripts} />
              Enable interactivity (scripts run inside the sandboxed iframe — no same-origin access)
            </label>
            <div class="insp-row"><span class="k">Frame</span>
              <select class="v" bind:value={device} aria-label="Device frame">
                {#each DEVICES as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
              </select>
            </div>
          </section>
        {:else if kind === 'excalidraw'}
          <section class="insp-sec">
            <div class="insp-title">Board</div>
            <p class="insp-hint">Frames are artboards; keep an 8-pt grid. Toggle <strong>Annotate</strong> to pin comments (the board is read-only while annotating).</p>
          </section>
        {:else if kind === 'mermaid'}
          <section class="insp-sec">
            <div class="insp-title">Diagram</div>
            <p class="insp-hint">Edit the Mermaid text in the <strong>Source</strong> view; the diagram re-renders as you type and autosaves.</p>
          </section>
        {:else if kind === 'scene3d' && sceneDoc}
          <Inspector doc={sceneDoc} bind:selectedId={scene3dSel} onchange={onSceneChange} />
          <section class="insp-sec">
            <div class="insp-title">Blender</div>
            {#if blender === null}
              <p class="insp-hint">Checking for Blender…</p>
            {:else if blender.installed}
              <p class="insp-hint">Blender {blender.version ?? ''} <span class="mono">{blender.path}</span></p>
              <div class="insp-actions">
                <button class="p-btn primary" onclick={renderInBlender} disabled={rendering}>
                  {rendering ? `Rendering… ${renderJob?.status ?? ''}` : 'Render + export GLB'}
                </button>
                <button class="p-btn" onclick={downloadBlenderScript}>Download script</button>
              </div>
            {:else}
              <p class="insp-hint">Blender isn't installed (set <span class="mono">OTTO_BLENDER</span> or install it in /Applications). You can still download the generated script and open it in Blender by hand.</p>
              <div class="insp-actions">
                <button class="p-btn" onclick={downloadBlenderScript}>Download script</button>
              </div>
            {/if}
          </section>
        {/if}

        {#if !assistDocked && isText}
          <section class="insp-sec">
            <button class="p-btn primary wide" onclick={() => att && refine(att)}>
              <Icon name="zap" size={12} /> Refine with AI
            </button>
          </section>
        {/if}
      {:else}
        <p class="insp-hint pad">Nothing selected.</p>
      {/if}
    </div>

    {#if assistDocked}
      <div class="insp-assist">
        <MockupAssistPanel embedded oncommit={onAssistCommit} onclose={closeAssist} />
      </div>
    {/if}
  </aside>
</div>

<style>
  .design-arena {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--surface);
  }
  .arena-seg {
    display: none;
  }

  /* ── Assets pane (ListPane chrome, inlined so the header can hold a switch) ── */
  .arena-assets {
    width: 250px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .pane-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px 10px 6px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .pane-title {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .pane-switch {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .ss {
    height: 20px;
    padding: 0 7px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 10.5px;
    font-weight: 600;
    cursor: pointer;
  }
  .ss.active {
    background: var(--surface);
    color: var(--accent);
  }
  .list-actions {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-wrap: wrap;
    width: 100%;
  }
  .list-actions .primary {
    margin-inline-start: auto;
  }
  .child-filter {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    color: var(--text-dim);
  }
  .child-filter select {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    padding: 2px 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
  }
  .pane-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
  .list-empty {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 10px;
    line-height: 1.5;
  }
  .list-empty.err {
    color: #ef4444;
  }
  .group-head {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    padding: 7px 10px 3px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    cursor: pointer;
    text-align: start;
  }
  .group-count {
    margin-inline-start: auto;
    font-weight: 600;
    opacity: 0.8;
  }
  .mockup-row {
    display: flex;
    align-items: center;
    gap: 2px;
    width: 100%;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .mockup-open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 10px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .mockup-row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .mockup-row.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .refine-btn,
  .row-more {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .row-more {
    margin-inline-end: 4px;
    opacity: 0;
  }
  .mockup-row:hover .row-more,
  .mockup-row.active .row-more {
    opacity: 1;
  }
  .refine-btn:hover,
  .row-more:hover {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .mockup-type {
    flex-shrink: 0;
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .mockup-row.active .mockup-type {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .mockup-name {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .agent-badge,
  .owner-badge {
    flex-shrink: 0;
    font-size: 8.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 999px;
    max-width: 80px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .agent-badge {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .owner-badge {
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
    text-transform: none;
  }

  /* ── Stage ────────────────────────────────────────────────────────────── */
  .arena-stage {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .stage-toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    flex-wrap: wrap;
  }
  .st-name {
    font-size: 12.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 40%;
  }
  .st-type {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .st-grow {
    flex: 1;
  }
  .seg-group {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .seg {
    height: 22px;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }
  .seg.active {
    background: var(--surface);
    color: var(--accent);
  }
  .tb-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }
  .tb-btn.icon {
    padding: 0 5px;
  }
  .tb-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .tb-btn.on {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 36%, transparent);
  }
  .stage-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .viewport {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .viewport > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .stage-body.split .viewport {
    flex: 1.4;
  }
  .code-view {
    flex: 1;
    min-width: 0;
    min-height: 0;
    resize: none;
    border: none;
    border-inline-start: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    font: 12px/1.5 var(--font-mono, monospace);
    padding: 10px 12px;
    outline: none;
    tab-size: 2;
  }
  .render-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .render-box {
    position: relative;
    flex: 1;
    min-height: 320px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: #fff;
  }
  .render-box > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .stage-msg {
    padding: 24px;
    font-size: 12.5px;
    color: var(--text-dim);
    text-align: center;
    flex: none;
  }
  .stage-msg.err {
    color: #ef4444;
  }
  .stage-msg p {
    margin: 0 0 8px;
  }
  .issues {
    margin: 0 auto;
    padding: 0;
    list-style: none;
    text-align: start;
    max-width: 560px;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .stage-status {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .stage-status .save.warn {
    color: #f59e0b;
    font-weight: 600;
  }
  .stage-status .link {
    border: none;
    background: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 11px;
    padding: 0;
    margin-inline-start: 6px;
  }
  .stage-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 20px;
  }
  .stage-empty p {
    margin: 0;
    font-size: 13px;
    max-width: 360px;
    line-height: 1.5;
  }

  /* ── Inspector + assistant ────────────────────────────────────────────── */
  .arena-inspector {
    width: 300px;
    flex-shrink: 0;
    border-inline-start: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .arena-inspector[hidden] {
    display: none;
  }
  .insp-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }
  .insp-head {
    padding: 8px 12px 6px;
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .insp-sec {
    padding: 10px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .insp-title {
    font-size: 11px;
    font-weight: 700;
    color: var(--text);
  }
  .insp-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
  }
  .insp-row .k {
    width: 48px;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .insp-row .v {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .insp-row select.v {
    font-size: 11px;
    padding: 2px 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
  }
  .insp-row small {
    color: var(--text-dim);
    font-size: 10px;
  }
  .insp-check {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.4;
    cursor: pointer;
  }
  .insp-hint {
    margin: 0;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .insp-hint.pad {
    padding: 12px;
  }
  .insp-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .wide {
    justify-content: center;
  }
  .insp-assist {
    flex: 1 1 55%;
    min-height: 260px;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }

  /* ── Phone: segmented single pane ─────────────────────────────────────── */
  @media (max-width: 640px) {
    .design-arena {
      flex-direction: column;
    }
    .arena-seg {
      display: flex;
      gap: 2px;
      padding: 6px 8px;
      border-bottom: 1px solid var(--border);
      flex-shrink: 0;
    }
    .arena-seg .seg {
      flex: 1;
      height: 32px;
      font-size: 13px;
    }
    .arena-assets,
    .arena-stage,
    .arena-inspector {
      display: none;
      width: 100%;
      border: none;
    }
    .m-assets .arena-assets,
    .m-canvas .arena-stage,
    .m-inspector .arena-inspector:not([hidden]) {
      display: flex;
      flex: 1;
      min-height: 0;
    }
    /* Creating with AI (full-stage panel) always shows the canvas pane. */
    .m-assets .arena-stage:has(:global(.mockup-assist:not(.embedded))),
    .m-inspector .arena-stage:has(:global(.mockup-assist:not(.embedded))) {
      display: flex;
      flex: 1;
    }
    .st-name {
      max-width: 100%;
    }
    .code-view {
      min-height: 160px;
      border-inline-start: none;
      border-top: 1px solid var(--border);
    }
    .stage-body.split {
      flex-direction: column;
    }
  }
</style>
