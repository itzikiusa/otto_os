<script lang="ts">
  // Per-session Canvas references: the scenes attached to the focused agent
  // session (via GET/POST/DELETE /sessions/{id}/canvas-refs). Each row shows a
  // format chip + an expandable inline SVG preview (mermaid/d2 — rendered from
  // the scene's live source; Excalidraw rows show a static "board" card
  // instead of mounting the heavy editor here). Live updates arrive via
  // canvasRefsBus (attach/detach) and canvasDocBus (an open scene's source
  // changing while an agent edits it).
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { router } from '../../lib/router.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { canvasRefsBus, canvasDocBus } from '../../lib/events.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { renderMermaid } from '../canvas/mermaid';
  import { renderD2 } from '../canvas/d2';
  import type { CanvasDoc, CanvasFormat, CanvasScene, CanvasSceneSummary } from '../canvas/types';

  const session = $derived(ws.activeSession);
  const sessionId = $derived(session?.id ?? null);

  let refs = $state<CanvasSceneSummary[]>([]);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  // Which session `refs` belongs to. On a session switch the previous
  // session's rows are dropped at once (not shown under the new session until
  // the fetch lands), and a slower response for an older session is ignored.
  let refsFor: string | null = null;
  let loadSeq = 0;

  async function load(): Promise<void> {
    const sid = sessionId;
    const seq = ++loadSeq;
    if (!sid) {
      refs = [];
      refsFor = null;
      return;
    }
    if (refsFor !== sid) {
      refs = [];
      refsFor = sid;
    }
    loading = true;
    loadError = null;
    try {
      const rows = await api.get<CanvasSceneSummary[]>(`/sessions/${sid}/canvas-refs`);
      if (seq === loadSeq) refs = rows;
    } catch (e) {
      if (seq === loadSeq) loadError = e instanceof Error ? e.message : String(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  // Load whenever the focused session changes.
  $effect(() => {
    const sid = sessionId;
    if (sid) void load();
  });

  // Reload on attach/detach for THIS session (from anywhere: this panel, the
  // Canvas module's "Attach to session" flow, or an MCP write tool).
  $effect(() => {
    const t = canvasRefsBus.tick;
    if (t > 0 && canvasRefsBus.sessionId === sessionId) void load();
  });

  // -- expandable inline preview -----------------------------------------

  let expanded = $state<Record<string, boolean>>({});
  interface Preview {
    key: string;
    svg?: string;
    error?: string;
    loading?: boolean;
  }
  let previews = $state<Record<string, Preview>>({});

  async function renderFromDoc(sceneId: string, doc: CanvasDoc, key: string): Promise<void> {
    const format: CanvasFormat = doc.format ?? 'mermaid';
    if (format === 'excalidraw') return; // static card — nothing to render
    previews = { ...previews, [sceneId]: { key, loading: true } };
    const renderId = `canvas-panel-${sceneId}`;
    const source = doc.source ?? '';
    const out =
      format === 'd2'
        ? await renderD2(renderId, source, { sketch: doc.sketch, dark: ui.resolvedScheme === 'dark' })
        : await renderMermaid(renderId, source);
    previews = { ...previews, [sceneId]: { key, svg: out.svg, error: out.error } };
  }

  async function togglePreview(ref: CanvasSceneSummary): Promise<void> {
    const open = !expanded[ref.id];
    expanded = { ...expanded, [ref.id]: open };
    if (!open || ref.format === 'excalidraw') return;

    const key = `${ref.id}:${ref.updated_at}`;
    if (previews[ref.id]?.key === key) return; // cached — same scene revision

    previews = { ...previews, [ref.id]: { key, loading: true } };
    try {
      const scene = await api.get<CanvasScene>(`/canvas/scenes/${ref.id}`);
      let doc: CanvasDoc | null = null;
      try {
        doc = JSON.parse(scene.doc_json) as CanvasDoc;
      } catch {
        doc = null;
      }
      if (!doc || typeof doc.source !== 'string') {
        previews = { ...previews, [ref.id]: { key, error: 'No renderable source' } };
        return;
      }
      await renderFromDoc(ref.id, doc, key);
    } catch (e) {
      previews = { ...previews, [ref.id]: { key, error: e instanceof Error ? e.message : String(e) } };
    }
  }

  // Live re-render: an open, expanded, referenced scene's source changed
  // while an agent edits it (canvas_updated). Render straight from the pushed
  // doc — no refetch needed.
  $effect(() => {
    const sceneId = canvasDocBus.sceneId;
    const tick = canvasDocBus.tick;
    if (tick === 0 || !sceneId) return;
    if (!expanded[sceneId] || !refs.some((r) => r.id === sceneId)) return;
    const doc = canvasDocBus.doc as CanvasDoc | null;
    if (!doc || typeof doc.source !== 'string') return;
    void renderFromDoc(sceneId, doc, `live:${tick}`);
  });

  // -- row actions ---------------------------------------------------------

  function openInCanvas(sceneId: string): void {
    canvas.pendingOpenId = sceneId;
    router.go('canvas');
  }

  async function detach(sceneId: string): Promise<void> {
    const sid = sessionId;
    if (!sid) return;
    try {
      await api.del(`/sessions/${sid}/canvas-refs/${sceneId}`);
      refs = refs.filter((r) => r.id !== sceneId);
    } catch (e) {
      toasts.error('Detach failed', e instanceof Error ? e.message : String(e));
    }
  }

  // -- footer: attach existing / create new --------------------------------

  let attachOpen = $state(false);
  let attachQuery = $state('');
  let allScenes = $state<CanvasSceneSummary[]>([]);
  let attachLoading = $state(false);
  let attachError = $state<string | null>(null);
  let creating = $state(false);

  // Re-list on every open: scenes created since the last open (here, on the
  // Canvas page or by an agent) must show up, and a failed fetch must not
  // read as "no scenes".
  async function loadAllScenes(): Promise<void> {
    attachLoading = true;
    attachError = null;
    try {
      allScenes = await api.get<CanvasSceneSummary[]>('/canvas/scenes');
    } catch (e) {
      attachError = e instanceof Error ? e.message : String(e);
    } finally {
      attachLoading = false;
    }
  }

  async function toggleAttachPicker(): Promise<void> {
    attachOpen = !attachOpen;
    if (attachOpen) await loadAllScenes();
  }

  const attachCandidates = $derived(
    allScenes
      .filter((s) => s.workspace_id === session?.workspace_id)
      .filter((s) => !refs.some((r) => r.id === s.id))
      .filter((s) => s.title.toLowerCase().includes(attachQuery.trim().toLowerCase())),
  );

  async function attachExisting(sceneId: string): Promise<void> {
    const sid = sessionId;
    if (!sid) return;
    try {
      await api.post(`/sessions/${sid}/canvas-refs`, { scene_id: sceneId });
      attachOpen = false;
      attachQuery = '';
      await load();
    } catch (e) {
      toasts.error('Attach failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function createNewScene(): Promise<void> {
    const sid = sessionId;
    const wsId = session?.workspace_id;
    if (!sid || !wsId || creating) return;
    creating = true;
    try {
      const doc: CanvasDoc = { type: 'otto-canvas', version: 1, format: 'mermaid', source: '' };
      const created = await api.post<CanvasScene>(`/workspaces/${wsId}/canvas/scenes`, {
        title: 'Untitled canvas',
        doc,
      });
      await api.post(`/sessions/${sid}/canvas-refs`, { scene_id: created.id });
      openInCanvas(created.id);
      await load();
    } catch (e) {
      toasts.error('Could not create canvas', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  // -- display helpers -------------------------------------------------------

  const FORMAT_LABEL: Record<CanvasFormat, string> = {
    mermaid: 'mermaid',
    d2: 'd2',
    excalidraw: 'board',
  };

  function formatOf(ref: CanvasSceneSummary): CanvasFormat {
    return ref.format ?? 'mermaid';
  }

  function label(s: CanvasSceneSummary): string {
    return s.section ? `${s.section.replace(/\//g, ' / ')} · ${s.title}` : s.title;
  }

  function onRefKeydown(e: KeyboardEvent, ref: CanvasSceneSummary): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      void togglePreview(ref);
    }
  }
</script>

{#if !session}
  <EmptyState
    icon="shapes"
    title="No session selected"
    body="Open or focus an agent session to see its referenced canvases."
  />
{:else}
  <div class="canvas-panel">
    <div class="cp-list">
      {#if loading && refs.length === 0}
        <p class="empty-line dim">Loading canvases…</p>
      {:else if loadError}
        <div class="load-error" role="alert">
          <div class="error-head"><Icon name="warning" size={13} /> Couldn't load this session's canvases</div>
          <div class="error-detail">{loadError}</div>
          <button class="btn small" onclick={() => void load()}>
            <Icon name="refresh" size={12} /> Retry
          </button>
        </div>
      {:else if refs.length === 0}
        <EmptyState
          icon="shapes"
          title="No canvases referenced"
          body="Attach one below, or ask the agent to draw a diagram."
        />
      {:else}
        <ul class="refs">
          {#each refs as ref (ref.id)}
            <li class="ref-row">
              <!-- Mouse: the whole row toggles. Keyboard/AT: the title block is the
                   toggle (it can't wrap the row's own action buttons). -->
              <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
              <div class="ref-main" onclick={() => togglePreview(ref)}>
                <span class="ref-chevron">
                  {#if formatOf(ref) !== 'excalidraw'}
                    <Icon name={expanded[ref.id] ? 'chevronDown' : 'chevronRight'} size={11} />
                  {/if}
                </span>
                <div
                  class="ref-body"
                  role="button"
                  tabindex="0"
                  aria-expanded={!!expanded[ref.id]}
                  onkeydown={(e) => onRefKeydown(e, ref)}
                >
                  <div class="ref-title">{label(ref)}</div>
                  <div class="ref-meta">
                    <span class="chip fmt-{formatOf(ref)}">{FORMAT_LABEL[formatOf(ref)]}</span>
                    <span class="ref-time mono" title={new Date(ref.updated_at).toLocaleString()}>{rel(ref.updated_at)}</span>
                  </div>
                </div>
                <div class="ref-actions" onclick={(e) => e.stopPropagation()} role="presentation">
                  <button class="icon-btn" title="Open in Canvas" aria-label="Open in Canvas" onclick={() => openInCanvas(ref.id)}>
                    <Icon name="shapes" size={13} />
                  </button>
                  <button
                    class="icon-btn"
                    title="Detach from this session"
                    aria-label="Detach from this session"
                    onclick={() => detach(ref.id)}
                  >
                    <Icon name="x" size={13} />
                  </button>
                </div>
              </div>
              {#if expanded[ref.id]}
                <div class="ref-preview">
                  {#if formatOf(ref) === 'excalidraw'}
                    <div class="board-card">
                      <Icon name="shapes" size={18} />
                      <span>Excalidraw board — open in Canvas to view/edit</span>
                    </div>
                  {:else if previews[ref.id]?.loading}
                    <p class="empty-line dim">Rendering…</p>
                  {:else if previews[ref.id]?.error}
                    <p class="empty-line dim">{previews[ref.id]?.error}</p>
                  {:else if previews[ref.id]?.svg}
                    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                    <div class="svg-wrap">{@html previews[ref.id]?.svg}</div>
                  {:else}
                    <p class="empty-line dim">Empty diagram.</p>
                  {/if}
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="cp-footer">
      {#if attachOpen}
        <div class="attach-picker">
          <input
            class="attach-search"
            placeholder="Search scenes to attach…"
            aria-label="Search scenes to attach"
            bind:value={attachQuery}
            spellcheck="false"
          />
          {#if attachLoading}
            <p class="empty-line dim">Loading scenes…</p>
          {:else if attachError}
            <p class="empty-line attach-error" role="alert">
              Couldn't load scenes.
              <button class="btn ghost small" onclick={() => void loadAllScenes()}>Retry</button>
            </p>
          {:else if attachCandidates.length === 0}
            <p class="empty-line dim">
              {attachQuery.trim()
                ? `No scenes match "${attachQuery.trim()}".`
                : 'No other scenes in this workspace. Use New scene to create one.'}
            </p>
          {:else}
            <ul class="candidates">
              {#each attachCandidates as c (c.id)}
                <li>
                  <button class="candidate-btn" onclick={() => attachExisting(c.id)}>{label(c)}</button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}
      <div class="cp-footer-actions">
        <button class="footer-btn" class:on={attachOpen} onclick={toggleAttachPicker}>
          <Icon name="plug" size={12} /> Attach scene…
        </button>
        <button class="footer-btn" disabled={creating} onclick={createNewScene}>
          <Icon name="plus" size={12} /> New scene
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .canvas-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .cp-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 10px;
  }
  .empty-line {
    font-size: 11.5px;
    line-height: 1.4;
    margin: 2px 0;
  }

  .refs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ref-row {
    border-radius: var(--radius-s);
  }
  .ref-main {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 4px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .ref-main:hover {
    background: var(--surface-2);
  }
  .ref-body:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: var(--radius-s);
  }
  .ref-chevron {
    flex-shrink: 0;
    width: 12px;
    display: inline-flex;
    color: var(--text-dim);
  }
  .ref-body {
    min-width: 0;
    flex: 1;
  }
  .ref-title {
    font-size: 12.5px;
    color: var(--text);
    word-break: break-word;
  }
  .ref-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
  }
  .chip {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .chip.fmt-d2 {
    color: var(--accent-text);
  }
  .ref-time {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ref-actions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .ref-preview {
    margin: 2px 0 6px 18px;
    padding: 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    max-height: 260px;
    overflow: auto;
  }
  .svg-wrap :global(svg) {
    max-width: 100%;
    height: auto;
  }
  .board-card {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px;
    font-size: 11.5px;
    color: var(--text-dim);
  }

  .load-error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    font-size: 12px;
  }
  .error-head {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
  }
  .error-head :global(svg) {
    color: var(--danger);
  }
  .error-detail {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    word-break: break-word;
  }
  .attach-error {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
  }

  .cp-footer {
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    padding: 8px 10px;
  }
  .cp-footer-actions {
    display: flex;
    gap: 6px;
  }
  .footer-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
  }
  .footer-btn:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--accent);
  }
  .footer-btn.on {
    color: var(--accent-text);
    border-color: var(--accent);
  }
  .footer-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .attach-picker {
    margin-bottom: 8px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .attach-search {
    width: 100%;
    height: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 11px;
    padding: 0 7px;
    outline: none;
    box-sizing: border-box;
  }
  .attach-search:focus {
    border-color: var(--accent);
  }
  .candidates {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    max-height: 160px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .candidate-btn {
    width: 100%;
    text-align: start;
    padding: 4px 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: 11.5px;
    cursor: pointer;
  }
  .candidate-btn:hover {
    background: var(--surface);
  }
</style>
