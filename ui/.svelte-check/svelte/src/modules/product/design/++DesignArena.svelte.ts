///<reference types="svelte" />
;
import { formatBytes } from '../../../lib/metric-format';
import { rowMenu as rowMenuKeys } from '../../../lib/rowMenu';
import { plural } from '../../../lib/plural';
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
import { onTabKey } from '../../../lib/tabKeys';
import { toastError } from '../../../lib/toastError';
import { onDestroy, untrack } from 'svelte';
import { product, toBase64 } from '../../../lib/stores/product.svelte';
import { mockupAssist, type LiveUpdate } from '../../../lib/stores/mockup-assist.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { rel } from '../../../lib/stores/now.svelte';
import MockupViewer from '../MockupViewer.svelte';
import MockupAnnotations from '../MockupAnnotations.svelte';
import MockupAssistPanel from '../MockupAssistPanel.svelte';
import DesignBoard from '../../../lib/design/DesignBoard.svelte';
import DeviceFrame, { DEVICES, type DeviceKind } from '../../../lib/design/DeviceFrame.svelte';
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
  } from '../../../lib/design/scene3d';
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
  } from '../../../lib/design/format';
import { viewport } from '../../../lib/stores/viewport.svelte';
import { DESIGN_TEMPLATES, blankSource, type DesignTemplate } from '../../../lib/design/templates';
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
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── Story context ──────────────────────────────────────────────────────────
  const story = $derived(product.detail?.story ?? null);
  const storyId = $derived(product.selectedId);
  const contentContext = product.contentContext;
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
  let blenderProbeError = $state<string | null>(null); // a failed probe ≠ "not installed"
  let blenderChecked = $state(false);
  let renderJob = $state<BlenderJob | null>(null);
  let rendering = $state(false);

  // ── Save status line ───────────────────────────────────────────────────────
  const saveState = $derived(att ? product.saveState[att.id] ?? 'saved' : 'saved');
  const savedAt = $derived(att ? product.savedAt[att.id] : undefined); // absolute on hover
  const saveLabel = $derived.by(() => {
    if (!att) return '';
    if (localInvalid) return `invalid document — not saved (${plural(localInvalid.length, 'issue')})`;
    switch (saveState) {
      case 'dirty': return 'unsaved';
      case 'saving': return 'saving…';
      case 'error': return 'save failed';
      case 'conflict': return 'conflict — newer version on the server';
      default: return savedAt ? `saved ${rel(savedAt)}` : 'saved';
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
  let lastChildIds = '';
  // Tracks ONLY the selected story: reading `mockupAssist.active` / `storyId`
  // here made every Assistant open/close reload the whole list (1 + N GETs)
  // and wipe the child filter — and closed a just-opened "Refine" on a child
  // story's artifact (its storyId differs from the epic's).
  $effect(() => {
    const sid = product.selectedId;
    untrack(() => {
      if (mockupAssist.active && mockupAssist.storyId !== sid) {
        mockupAssist.close();
        assistMode = null;
      }
      childFilter = '';
      // Drop the previous story's artifacts NOW: until this story's list
      // lands they would show (and autosave / delete) under the new story.
      // Clearing the selection flushes its pending edit (the selection effect).
      ownAtts = [];
      childAtts = {};
      selectedId = null;
      // loadAll() loads this story's children: effect (b) needn't again.
      lastChildIds = children.map((c) => c.id).join(',');
      void loadAll();
    });
  });
  // Children may arrive after the story (list reload) — refetch their
  // artifacts, but only when the child SET changed: `childrenOf` is a fresh
  // array on every stories write. Effect (a) records the set it loaded, so
  // the first run and a story switch skip — `loadAll()` already loads them.
  $effect(() => {
    const ids = children.map((c) => c.id).join(',');
    if (ids === lastChildIds) return;
    lastChildIds = ids;
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

  /** Overlapping list loads (story switch, list reload) race: only the newest
   *  one, for the story still selected, commits — a slow answer for story A
   *  must not land as story B's artifacts. */
  let loadSeq = 0;
  async function loadAll(quiet = false): Promise<void> {
    if (!product.selectedId) return;
    const my = ++loadSeq;
    const owned = product.captureSelection();
    const current = () => my === loadSeq && owned();
    if (!quiet) loading = true;
    loadError = null;
    try {
      const own = await product.listAttachments();
      if (!current()) return;
      ownAtts = own;
      await loadChildren(children);
      if (!current()) return;
      // Keep a valid selection: default to the first artifact if none chosen or
      // the previously-selected one is gone.
      if (!selectedId || !rows.some((r) => r.att.id === selectedId)) {
        selectedId = rows[0]?.att.id ?? null;
        if (selectedId && !viewport.isPhone) mobilePane = 'canvas';
      }
    } catch (e) {
      // A load failure is shown inline in the assets pane (with Retry), not toasted.
      if (current()) loadError = loadErrorText(e);
    } finally {
      if (my === loadSeq) loading = false;
    }
  }
  /** Overlapping child loads (story switch, list reload) race: only the
   *  newest one commits. */
  let childSeq = 0;
  async function loadChildren(list: ProductStory[]): Promise<void> {
    const my = ++childSeq;
    if (!list.length) {
      childAtts = {};
      return;
    }
    const entries = await Promise.all(
      list.map(async (c) => [c.id, await product.listAttachmentsOf(c.id).catch(() => [])] as const),
    );
    if (my !== childSeq) return;
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
      const text = await product.attachmentText(a.id, true);
      if (token === loadToken) {
        source = text;
        // Optimistic-concurrency base: the row we just loaded.
        if (!product.hasUnsavedContent(a.id)) product.setContentBase(a.id, a.updated_at);
      }
    } catch (e) {
      if (token === loadToken) sourceError = loadErrorText(e);
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
    if (contentContext !== product.contentContext || aid !== att?.id) return;
    if (incoming === source) return; // the echo of our own save
    if (product.hasUnsavedContent(aid)) {
      const ok = await confirmer.ask(
        'This artifact was just changed on the server (by the agent or another editor) while you have unsaved edits here. Replace your edits with the new version?',
        { title: 'Newer version available', confirmLabel: 'Replace mine', danger: false },
      );
      if (contentContext !== product.contentContext || aid !== att?.id) return;
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
   *  mine (overwrite against the fresh base) — never a silent clobber. Two
   *  explicit buttons: a yes/no here once mapped Cancel/Esc to "overwrite
   *  theirs", so dismissing the dialog destroyed the other version. Dismiss
   *  now does nothing — the edits stay pending and the next save asks again. */
  async function resolveConflict(a: ProductAttachment, mine: string): Promise<void> {
    const { value } = await confirmer.choose(
      'Saving failed: this artifact was changed on the server since you loaded it (another editor or the agent). Your edits aren’t saved yet.',
      {
        title: 'Save conflict',
        options: [
          { label: 'Keep mine (overwrite theirs)', value: 'mine', kind: 'primary' },
          { label: 'Take theirs (discard my edits)', value: 'theirs', kind: 'danger' },
        ],
      },
    );
    if (contentContext !== product.contentContext || (value !== 'mine' && value !== 'theirs')) return;
    const takeTheirs = value === 'theirs';
    try {
      const fresh = (await product.listAttachmentsOf(a.story_id)).find((x) => x.id === a.id);
      if (!fresh || contentContext !== product.contentContext) return;
      if (takeTheirs) {
        product.discardPendingContent(a.id);
        product.setContentBase(a.id, fresh.updated_at);
        await loadSource(fresh);
      } else {
        await product.overwriteAttachmentContent(a.id, mine, fresh.updated_at);
      }
      scheduleListReload();
    } catch (e) {
      toastError('Couldn’t resolve the conflict', e);
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
    void product.saveAttachmentContent(a.id, next, contentContext).catch((e) => {
      if (e instanceof ApiError && e.status === 409) {
        if (product.saveState[a.id] === 'conflict') void resolveConflict(a, next);
        return;
      }
      toastError('Couldn’t save the design', e);
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
      toasts.error('Couldn’t apply that scene edit', r.issues[0]?.message ?? 'The edit would make the scene invalid, so it wasn’t saved.');
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
      toastError('Couldn’t create the artifact', e);
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
          toastError(`Couldn’t import ${f.name}`, err);
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
      toastError('Couldn’t import the GLB', err);
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
    toastError('Couldn’t export the design', e);
  }
  function rowMenu(e: MouseEvent | KeyboardEvent, r: { att: ProductAttachment; owner: ProductStory | null }): void {
    const k = kindOf(r.att);
    const items: MenuItem[] = [
      { label: 'Open', icon: 'eye', action: () => selectArtifact(r.att.id) },
      { label: 'Rename…', icon: 'edit', action: () => void rename(r.att) },
    ];
    if (isTextKind(k)) items.push({ label: 'Refine with Otto', icon: 'zap', action: () => void refine(r.att) });
    items.push({ separator: true });
    items.push({ label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(r.att) });
    ctxMenu.show(e, items);
  }
  function moreMenu(e: MouseEvent): void {
    if (!att) return;
    const items: MenuItem[] = [
      { label: codeView ? 'Hide source' : 'Show source', icon: 'file', disabled: !isText, action: () => (codeView = !codeView) },
      { label: 'Rename…', icon: 'edit', action: () => void rename(att!) },
      { label: 'Refresh list', icon: 'refresh', action: () => void loadAll() },
      { separator: true },
      { label: 'Delete artifact…', icon: 'trash', danger: true, action: () => void remove(att!) },
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
      toastError('Couldn’t rename the design', e);
    }
  }
  async function remove(a: ProductAttachment): Promise<void> {
    const ok = await confirmer.ask(`Delete “${a.filename}”? Its pinned annotations go with it.`, {
      title: 'Delete artifact', confirmLabel: 'Delete', danger: true,
    });
    if (!ok) return;
    try {
      product.discardPendingContent(a.id);
      await product.deleteAttachment(a.id);
      if (selectedId === a.id) selectedId = null;
      await loadAll(true);
    } catch (e) {
      toastError('Couldn’t delete the design', e);
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
      blenderProbeError = null;
      void product.blenderStatus().then((s) => (blender = s)).catch((e: unknown) => (blenderProbeError = loadErrorText(e)));
    }
  });
  const recheckBlender = (): void => { blender = null; blenderChecked = false; }; // the effect re-probes
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
        toasts.success('Blender render finished', `${plural(renderJob.outputs.length, 'file')} attached`);
        await loadAll(true);
      } else {
        toasts.error('Couldn’t render with Blender', renderJob.error ?? 'The job ended without a reason. Check the Blender log.');
      }
    } catch (e) {
      toastError('Couldn’t render with Blender', e);
    } finally {
      rendering = false;
    }
  }

  const resolveAttachment = (aid: string) => product.attachmentBlobUrl(aid);
;
async () => {

 { svelteHTML.createElement("div", { "class":`arena-container`,});

  { svelteHTML.createElement("div", {     "class":`design-arena mockups-tab`,});mobilePane === 'assets';mobilePane === 'canvas';mobilePane === 'inspector';
  
   { svelteHTML.createElement("div", {     "class":`arena-seg`,"role":`tablist`,"aria-label":`Design panes`,});
     { svelteHTML.createElement("button", {            "class":`seg`,"role":`tab`,"aria-selected":mobilePane === 'assets',"tabindex":mobilePane === 'assets' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (mobilePane = 'assets'),});mobilePane === 'assets';  }
     { svelteHTML.createElement("button", {            "class":`seg`,"role":`tab`,"aria-selected":mobilePane === 'canvas',"tabindex":mobilePane === 'canvas' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (mobilePane = 'canvas'),});mobilePane === 'canvas';  }
     { svelteHTML.createElement("button", {            "class":`seg`,"role":`tab`,"aria-selected":mobilePane === 'inspector',"tabindex":mobilePane === 'inspector' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (mobilePane = 'inspector'),});mobilePane === 'inspector';  }
   }

  
   { svelteHTML.createElement("aside", { "class":`arena-assets list-pane`,});
     { svelteHTML.createElement("div", { "class":`pane-head`,});
      if(kind === 'scene3d' && sceneDoc){
         { svelteHTML.createElement("div", {     "class":`pane-switch`,"role":`tablist`,"aria-label":`Left pane`,});
           { svelteHTML.createElement("button", {            "class":`ss`,"role":`tab`,"aria-selected":leftPane === 'assets',"tabindex":leftPane === 'assets' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (leftPane = 'assets'),});leftPane === 'assets';  }
           { svelteHTML.createElement("button", {            "class":`ss`,"role":`tab`,"aria-selected":leftPane === 'hierarchy',"tabindex":leftPane === 'hierarchy' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (leftPane = 'hierarchy'),});leftPane === 'hierarchy';  }
         }
      }else{
         { svelteHTML.createElement("span", { "class":`pane-title`,});  }
      }
       { svelteHTML.createElement("div", { "class":`list-actions`,});
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":newMenu,"title":`New artifact (blank or from a template)`,});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
         }
         { svelteHTML.createElement("label", {   "class":`btn small`,"title":`Import files: HTML, images, SVG, .mmd, .excalidraw, .glb / .gltf`,});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}
          importing ? 'Importing…' : 'Import';
           { svelteHTML.createElement("input", {          "type":`file`,"multiple":true,"accept":IMPORT_ACCEPT,"style":`display:none`,"onchange":importFiles,"disabled":importing,});}
         }
         { svelteHTML.createElement("button", {     "class":`act-btn btn small primary`,"onclick":aiMenu,"title":`Generate an artifact with AI`,});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}   
         }
       }
     }

    if(isEpic && children.length > 0 && leftPane === 'assets'){
      
       { svelteHTML.createElement("label", { "class":`child-filter`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", {   "bind:value":childFilter,"aria-label":`Filter artifacts by child story`,});/*Ωignore_startΩ*/() => childFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});    }
           { svelteHTML.createElement("option", { "value":storyId,});  }
             for(let c of __sveltets_2_ensureArray(children)){c.id;
             { svelteHTML.createElement("option", { "value":c.id,});c.folder ? `${c.folder} / ` : '';c.title; }
          }
         }
       }
    }

     { svelteHTML.createElement("div", { "class":`pane-body`,});
      if(leftPane === 'hierarchy' && sceneDoc){
         { const $$_yhcrareiH4C = __sveltets_2_ensureComponent(Hierarchy); const $$_yhcrareiH4 = new $$_yhcrareiH4C({ target: __sveltets_2_any(), props: {         "doc":sceneDoc,selectedId:scene3dSel,"onchange":onSceneChange,"onimportGlb":() => glbInput?.click(),}});/*Ωignore_startΩ*/() => scene3dSel = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_yhcrareiH4.$$bindings = 'selectedId';}
         { const $$_input4 = svelteHTML.createElement("input", {          "type":`file`,"accept":`.glb,.gltf,${GLB_MIME},${GLTF_MIME}`,"style":`display:none`,"onchange":importGlb,});glbInput = $$_input4;}
      } else if ((loading || loadError) && rows.length === 0){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {          "what":`design artifacts`,"variant":`compact`,"loading":loading,"error":loadError,"empty":true,"onretry":() => void loadAll(),}});}
      } else if (rows.length === 0){
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {       "icon":`layers`,"title":`No design artifacts yet`,"body":`New ▾ for a blank screen, board, diagram or 3D scene (or a template); Import a file; or Create with AI to have a specialized agent build one right here.`,}});}
      }else{
           for(let g of __sveltets_2_ensureArray(groups)){g.name;
           { svelteHTML.createElement("button", {     "class":`group-head`,"onclick":() => (collapsedGroups = { ...collapsedGroups, [g.name]: !collapsedGroups[g.name] }),"aria-expanded":!collapsedGroups[g.name],});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":collapsedGroups[g.name] ? 'chevronRight' : 'chevronDown',"size":12,}});}
             { svelteHTML.createElement("span", {});g.name; }
             { svelteHTML.createElement("span", { "class":`group-count`,});g.rows.length; }
           }
          if(!collapsedGroups[g.name]){
               for(let r of __sveltets_2_ensureArray(g.rows)){r.att.id;
               {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {         "class":`mockup-row`,"role":`presentation`,"oncontextmenu":(e) => rowMenu(e, r),});selectedId === r.att.id;
                 { svelteHTML.createElement("button", {     "class":`mockup-open`,"onclick":() => selectArtifact(r.att.id),"title":r.att.filename,});
                   { svelteHTML.createElement("span", { "class":`mockup-type`,});typeLabel(r.att); }
                   { svelteHTML.createElement("span", { "class":`mockup-name`,});r.att.filename; }
                  if(isEpic && r.owner && r.owner.id !== storyId){
                     { svelteHTML.createElement("span", {   "class":`owner-badge`,"title":`From child: ${r.owner.title}`,});r.owner.folder || r.owner.title; }
                  }
                  if(r.att.source === 'agent'){ { svelteHTML.createElement("span", { "class":`agent-badge`,});  }}
                 }
                if(r.att.source === 'agent'){
                   { svelteHTML.createElement("button", {       "class":`refine-btn`,"onclick":() => refine(r.att),"title":`Refine with Otto`,"aria-label":`Refine with Otto`,});
                     { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}
                   }
                }
                 { svelteHTML.createElement("button", {       "class":`row-more reveal-on-hover`,"onclick":(e) => rowMenu(e, r),"aria-label":`Artifact actions`,"title":`Artifact actions`,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});}
                 }
               }}
            }
          }
        }
      }
     }
   }

  
   { svelteHTML.createElement("div", { "class":`arena-stage mockup-stage`,});
    if(assistCreating){
       { const $$_lenaPtsissApukcoM3C = __sveltets_2_ensureComponent(MockupAssistPanel); new $$_lenaPtsissApukcoM3C({ target: __sveltets_2_any(), props: {    "oncommit":onAssistCommit,"onclose":closeAssist,}});}
    } else if (att){
       { svelteHTML.createElement("div", { "class":`stage-toolbar`,});
         { svelteHTML.createElement("span", {   "class":`st-name`,"title":att.filename,});att.filename; }
         { svelteHTML.createElement("span", { "class":`st-type`,});typeLabel(att); }
        if(kind === 'html'){
           { svelteHTML.createElement("div", {     "class":`seg-group`,"role":`group`,"aria-label":`Device frame`,});
               for(let d of __sveltets_2_ensureArray(DEVICES)){d.id;
               { svelteHTML.createElement("button", {      "class":`seg`,"aria-pressed":device === d.id,"onclick":() => (device = d.id),});device === d.id;d.label; }
            }
           }
           { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => (scheme = scheme === 'dark' ? 'light' : 'dark'),"title":`Toggle light / dark backdrop`,"aria-label":`Toggle light / dark`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":scheme === 'dark' ? 'eye' : 'eyeOff',"size":12,}});} scheme === 'dark' ? 'Dark' : 'Light';
           }
        }
        if(kind === 'excalidraw'){
           { svelteHTML.createElement("button", {        "class":`btn small da-toggle`,"aria-pressed":annotate,"onclick":() => (annotate = !annotate),"title":`Annotate: pin comments on the board (read-only while on)`,});annotate;
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`pin`,"size":12,}});} 
           }
        }
        if(kind === 'scene3d'){
           { svelteHTML.createElement("button", {        "class":`btn small da-toggle`,"aria-pressed":play,"onclick":() => (play = !play),"title":`Play: presentation view (doc camera, no gizmo/grid)`,});play;
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} 
           }
        }
        if(isText){
           { svelteHTML.createElement("button", {        "class":`btn small da-toggle`,"aria-pressed":codeView,"onclick":() => (codeView = !codeView),"title":`Edit the source`,});codeView;
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} 
           }
        }
         { svelteHTML.createElement("span", { "class":`st-grow`,}); }
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":exportMenu,"title":`Export`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":moreMenu,"aria-label":`More actions`,"title":`More actions`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
       }

       { svelteHTML.createElement("div", {  "class":`stage-body`,});codeView && isText;
         { const $$_div4 = svelteHTML.createElement("div", {  "class":`viewport`,});viewportEl = $$_div4;
          if(sourceLoading || sourceError){
             { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {        "what":att.filename,"loading":sourceLoading,"error":sourceError,"empty":true,"onretry":() => void loadSource(att),}});}
          } else if (kind === 'html'){
             { const $$_emarFeciveD5C = __sveltets_2_ensureComponent(DeviceFrame); new $$_emarFeciveD5C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },device,scheme,}});
                { const $$_reweiVpukcoM6C = __sveltets_2_ensureComponent(MockupViewer); const $$_reweiVpukcoM6 = new $$_reweiVpukcoM6C({ target: __sveltets_2_any(), props: {    "attachment":att,source,"hideToolbar":true,allowScripts,}});/*Ωignore_startΩ*/() => allowScripts = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_reweiVpukcoM6.$$bindings = 'allowScripts';}
             DeviceFrame}
          } else if (kind === 'mermaid'){
              { const $$_reweiVpukcoM5C = __sveltets_2_ensureComponent(MockupViewer); new $$_reweiVpukcoM5C({ target: __sveltets_2_any(), props: {   "attachment":att,source,"hideToolbar":true,}});}
          } else if (kind === 'image'){
            `${att.id}:${binaryTick}`; {
                { const $$_reweiVpukcoM5C = __sveltets_2_ensureComponent(MockupViewer); new $$_reweiVpukcoM5C({ target: __sveltets_2_any(), props: {  "attachment":att,"hideToolbar":true,}});}
            }
          } else if (kind === 'excalidraw'){
             { svelteHTML.createElement("div", { "class":`render-wrap`,});
               { const $$_div6 = svelteHTML.createElement("div", {  "class":`render-box board-box`,});boardBox = $$_div6;
                 { const $$_draoBngiseD7C = __sveltets_2_ensureComponent(DesignBoard); new $$_draoBngiseD7C({ target: __sveltets_2_any(), props: {     source,"readonly":annotate,"onchange":onBoardChange,}});}
               }
              if(annotate && boardBox){
                 { const $$_snoitatonnApukcoM6C = __sveltets_2_ensureComponent(MockupAnnotations); new $$_snoitatonnApukcoM6C({ target: __sveltets_2_any(), props: {    "attachmentId":att.id,"box":boardBox,}});}
              }
             }
          } else if (kind === 'scene3d'){
            if(sceneDoc){
               { const $$_tropweiVD3enecS5C = __sveltets_2_ensureComponent(Scene3DViewport); const $$_tropweiVD3enecS5 = new $$_tropweiVD3enecS5C({ target: __sveltets_2_any(), props: {          "doc":sceneDoc,selectedId:scene3dSel,play,"onchange":onSceneChange,resolveAttachment,}});sceneView = $$_tropweiVD3enecS5;/*Ωignore_startΩ*/() => scene3dSel = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_tropweiVD3enecS5.$$bindings = 'selectedId';}
            }else{
               { svelteHTML.createElement("div", { "class":`stage-msg err`,});
                 { svelteHTML.createElement("p", {});           { svelteHTML.createElement("strong", {});  }  }
                if(sceneParse.issues.length){
                   { svelteHTML.createElement("ul", { "class":`issues`,});
                       for(let i of __sveltets_2_ensureArray((sceneParse.issues.slice(0, 12)))){i.path + i.message;
                       { svelteHTML.createElement("li", {}); { svelteHTML.createElement("span", { "class":`mono`,});i.path || '(root)'; }  i.message; }
                    }
                   }
                }
               }
            }
          } else if (kind === 'model' && modelDoc){
            `${att.id}:${binaryTick}`; {
               { const $$_tropweiVD3enecS5C = __sveltets_2_ensureComponent(Scene3DViewport); new $$_tropweiVD3enecS5C({ target: __sveltets_2_any(), props: {       "doc":modelDoc,"readonly":true,play,"onchange":() => {},resolveAttachment,}});}
            }
          }else{
             { svelteHTML.createElement("div", { "class":`stage-msg err`,});   att.mime || 'unknown'; }
          }
         }
        if(codeView && isText){
          
           { svelteHTML.createElement("textarea", {           "dir":`ltr`,"class":`code-view`,"spellcheck":`false`,"value":source,"oninput":onCodeInput,"aria-label":`Artifact source`,}); }
        }
       }

       { svelteHTML.createElement("div", {   "class":`stage-status`,...__sveltets_2_empty({"data-save-state":saveState}),});
         { svelteHTML.createElement("span", {});countLabel; }
         { svelteHTML.createElement("span", { "class":`dot`,});  }
         { svelteHTML.createElement("span", {    "class":`save`,"title":savedAt ? `Saved ${new Date(savedAt).toLocaleString()}` : undefined,});saveState === 'conflict' || saveState === 'error' || !!localInvalid;saveLabel; }
        if(saveState === 'conflict'){
           { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => att && resolveConflict(att, source),});  }
        }
        if(saveState === 'error'){
           { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => att && applyLocalEdit(source),});  }
        }
       }
    } else if (!loading){
       { svelteHTML.createElement("div", { "class":`stage-empty`,});
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {      "icon":`layers`,"title":`No artifact selected`,"body":`Select an artifact to view and edit it, use New ▾ for a blank one or a template, or Create with AI to generate one in place.`,}});}
       }
    }
   }

  
   { svelteHTML.createElement("aside", {   "class":`arena-inspector`,"hidden":assistCreating,});
     { svelteHTML.createElement("div", { "class":`insp-scroll`,});
       { svelteHTML.createElement("div", { "class":`insp-head`,});  }
      if(att){
         { svelteHTML.createElement("section", { "class":`insp-sec`,});
           { svelteHTML.createElement("div", { "class":`insp-row`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", {   "class":`v mono`,"title":att.filename,});att.filename; } }
           { svelteHTML.createElement("div", { "class":`insp-row`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v`,});typeLabel(att);  { svelteHTML.createElement("small", { "class":`mono`,});att.mime; } } }
           { svelteHTML.createElement("div", { "class":`insp-row`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v`,});formatBytes(att.size_bytes); } }
           { svelteHTML.createElement("div", { "class":`insp-row`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v`,});groupOf(att); } }
          if(selected?.owner && selected.owner.id !== storyId){
             { svelteHTML.createElement("div", { "class":`insp-row`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v`,});selected.owner.title; } }
          }
           { svelteHTML.createElement("div", { "class":`insp-row`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v`,});att.source; } }
         }

        if(kind === 'html'){
           { svelteHTML.createElement("section", { "class":`insp-sec`,});
             { svelteHTML.createElement("div", { "class":`insp-title`,});  }
             { svelteHTML.createElement("label", { "class":`insp-check`,});
               { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":allowScripts,});/*Ωignore_startΩ*/() => allowScripts = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                         
             }
             { svelteHTML.createElement("div", { "class":`insp-row`,}); { svelteHTML.createElement("span", { "class":`k`,});  }
               { svelteHTML.createElement("select", {     "class":`v`,"bind:value":device,"aria-label":`Device frame`,});/*Ωignore_startΩ*/() => device = __sveltets_2_any(null);/*Ωignore_endΩ*/
                   for(let d of __sveltets_2_ensureArray(DEVICES)){d.id; { svelteHTML.createElement("option", { "value":d.id,});d.label; }}
               }
             }
           }
        } else if (kind === 'excalidraw'){
           { svelteHTML.createElement("section", { "class":`insp-sec`,});
             { svelteHTML.createElement("div", { "class":`insp-title`,});  }
             { svelteHTML.createElement("p", { "class":`insp-hint`,});         { svelteHTML.createElement("strong", {});  }          }
           }
        } else if (kind === 'mermaid'){
           { svelteHTML.createElement("section", { "class":`insp-sec`,});
             { svelteHTML.createElement("div", { "class":`insp-title`,});  }
             { svelteHTML.createElement("p", { "class":`insp-hint`,});       { svelteHTML.createElement("strong", {});  }          }
           }
        } else if (kind === 'scene3d' && sceneDoc){
           { const $$_rotcepsnI4C = __sveltets_2_ensureComponent(Inspector); const $$_rotcepsnI4 = new $$_rotcepsnI4C({ target: __sveltets_2_any(), props: {      "doc":sceneDoc,selectedId:scene3dSel,"onchange":onSceneChange,}});/*Ωignore_startΩ*/() => scene3dSel = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rotcepsnI4.$$bindings = 'selectedId';}
           { svelteHTML.createElement("section", { "class":`insp-sec`,});
             { svelteHTML.createElement("div", { "class":`insp-title`,});  }
            if(blenderProbeError){
               { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {        "what":`Blender status`,"variant":`compact`,"error":blenderProbeError,"empty":true,"onretry":recheckBlender,}});}
            } else if (blender === null){
               { svelteHTML.createElement("p", { "class":`insp-hint`,});   }
            } else if (blender.installed){
               { svelteHTML.createElement("p", { "class":`insp-hint`,}); blender.version ?? '';  { svelteHTML.createElement("span", { "class":`mono`,});blender.path; } }
            }else{
               { svelteHTML.createElement("p", { "class":`insp-hint`,});     { svelteHTML.createElement("span", { "class":`mono`,});  }                    }
            }
            if(blender || blenderProbeError){
               { svelteHTML.createElement("div", { "class":`insp-actions`,});
                if(blender?.installed){
                   { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":renderInBlender,"disabled":rendering,});
                    rendering ? `Rendering… ${renderJob?.status ?? ''}` : 'Render + export GLB';
                   }
                }
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":downloadBlenderScript,});  }
               }
            }
           }
        }

        if(!assistDocked && isText){
           { svelteHTML.createElement("section", { "class":`insp-sec`,});
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => att && refine(att),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}   
             }
           }
        }
      }else{
         { svelteHTML.createElement("p", { "class":`insp-hint pad`,});  }
      }
     }

    if(assistDocked){
       { svelteHTML.createElement("div", { "class":`insp-assist`,});
         { const $$_lenaPtsissApukcoM4C = __sveltets_2_ensureComponent(MockupAssistPanel); new $$_lenaPtsissApukcoM4C({ target: __sveltets_2_any(), props: {    "embedded":true,"oncommit":onAssistCommit,"onclose":closeAssist,}});}
       }
    }
   }
 }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DesignArena__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DesignArena__SvelteComponent_ = ReturnType<typeof DesignArena__SvelteComponent_>;
/*Ωignore_endΩ*/export default DesignArena__SvelteComponent_;