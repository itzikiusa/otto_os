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

  async function load(): Promise<void> {
    const sid = sessionId;
    if (!sid) {
      refs = [];
      return;
    }
    loading = true;
    loadError = null;
    try {
      refs = await api.get<CanvasSceneSummary[]>(`/sessions/${sid}/canvas-refs`);
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
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
  let creating = $state(false);

  async function toggleAttachPicker(): Promise<void> {
    attachOpen = !attachOpen;
    if (attachOpen && allScenes.length === 0) {
      attachLoading = true;
      try {
        allScenes = await api.get<CanvasSceneSummary[]>('/canvas/scenes');
      } catch {
        allScenes = [];
      } finally {
        attachLoading = false;
      }
    }
  }

  const attachCandidates = $derived(
    allScenes
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

  /** Relative time: "now", "3m", "2h", else a short date. */
  function relTime(iso: string): string {
    try {
      const then = new Date(iso).getTime();
      const secs = Math.max(0, Math.round((Date.now() - then) / 1000));
      if (secs < 10) return 'now';
      if (secs < 60) return `${secs}s`;
      const mins = Math.floor(secs / 60);
      if (mins < 60) return `${mins}m`;
      const hrs = Math.floor(mins / 60);
      if (hrs < 24) return `${hrs}h`;
      return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    } catch {
      return '';
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
        <p class="empty-line dim">Loading…</p>
      {:else if loadError}
        <p class="empty-line dim">Could not load canvas references: {loadError}</p>
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
              <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
              <div class="ref-main" onclick={() => togglePreview(ref)}>
                <span class="ref-chevron">
                  {#if formatOf(ref) !== 'excalidraw'}
                    <Icon name={expanded[ref.id] ? 'chevronDown' : 'chevronRight'} size={11} />
                  {/if}
                </span>
                <div class="ref-body">
                  <div class="ref-title">{label(ref)}</div>
                  <div class="ref-meta">
                    <span class="chip fmt-{formatOf(ref)}">{FORMAT_LABEL[formatOf(ref)]}</span>
                    <span class="ref-time mono">{relTime(ref.updated_at)}</span>
                  </div>
                </div>
                <div class="ref-actions" onclick={(e) => e.stopPropagation()} role="presentation">
                  <button class="icon-btn" title="Open in Canvas" onclick={() => openInCanvas(ref.id)}>
                    <Icon name="shapes" size={13} />
                  </button>
                  <button class="icon-btn" title="Detach from this session" onclick={() => detach(ref.id)}>
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
            bind:value={attachQuery}
            spellcheck="false"
          />
          {#if attachLoading}
            <p class="empty-line dim">Loading…</p>
          {:else if attachCandidates.length === 0}
            <p class="empty-line dim">No matching scenes.</p>
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
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .chip.fmt-d2 {
    color: var(--accent);
  }
  .ref-time {
    font-size: 9.5px;
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
    color: var(--accent);
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
