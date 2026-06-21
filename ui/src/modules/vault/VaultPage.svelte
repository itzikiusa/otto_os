<script lang="ts">
  // Obsidian-like memory vault: index + hybrid search + note reader + backlinks
  // + a dependency-free SVG knowledge graph. Scoped to the current workspace.
  // Includes lifecycle governance: state chips/filter, forget-with-undo, merge,
  // split, provenance diff, and governed import of AGENTS.md/CLAUDE.md/.cursorrules.
  import { vault } from './vault.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { renderMarkdown } from '../../lib/md';
  import type { Memory } from '../../lib/api/types';
  import { copyAsJson } from '../../lib/components/exporters';
  import DiffView from '../../lib/components/DiffView.svelte';
  import MemoryStateBadge from './MemoryStateBadge.svelte';
  import ImportGovDialog from './ImportGovDialog.svelte';
  import MergeDialog from './MergeDialog.svelte';
  import SplitDialog from './SplitDialog.svelte';
  import type { MemoryState } from './vault.svelte';

  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  // Provenance diff: show the body of the selected memory against its
  // superseded-by sibling (if present and loaded).
  let supersededMemory = $state<Memory | null>(null);
  let showProvDiff = $state(false);

  // Dialog visibility flags.
  let showImport = $state(false);
  let showMerge = $state(false);
  let showSplit = $state(false);

  $effect(() => {
    if (ws.currentId) void vault.load();
  });

  // Load the superseded-by memory whenever the selected note changes.
  $effect(() => {
    const m = vault.selected;
    supersededMemory = null;
    showProvDiff = false;
    if (m?.superseded_by && ws.currentId) {
      void loadSuperseded(m.superseded_by, ws.currentId);
    }
  });

  async function loadSuperseded(id: string, wsId: string) {
    try {
      const { api } = await import('../../lib/api/client');
      supersededMemory = await api.get<Memory>(`/workspaces/${wsId}/memories/${id}`);
    } catch {
      // superseded memory may be inaccessible — ignore
    }
  }

  function onSearchInput() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void vault.search(), 250);
  }

  function pick(m: Memory) {
    // In merge mode, toggle selection; otherwise open the note.
    if (vault.mergeMode) {
      vault.toggleMergeSelect(m);
    } else {
      void vault.select(m);
    }
  }

  // Merge sources from the store (resolved Memory objects).
  const mergeSources = $derived.by(() => {
    return vault.items.filter((m) => vault.mergeIds.includes(m.id));
  });

  // --- graph layout (circular, no external lib) ---
  type Pt = { x: number; y: number };
  const W = 720;
  const H = 520;
  const positions = $derived.by(() => {
    const map = new Map<string, Pt>();
    const nodes = vault.graph?.nodes ?? [];
    const n = Math.max(nodes.length, 1);
    const cx = W / 2;
    const cy = H / 2;
    const r = Math.min(W, H) / 2 - 60;
    nodes.forEach((node, i) => {
      const a = (2 * Math.PI * i) / n - Math.PI / 2;
      map.set(node.id, { x: cx + r * Math.cos(a), y: cy + r * Math.sin(a) });
    });
    return map;
  });

  /** Rough token estimate: ~4 chars per token (tiktoken p50 approximation). */
  function estimateTokens(m: Memory): number {
    return Math.ceil((m.title.length + m.body.length) / 4);
  }

  function nodeColor(kind: string): string {
    switch (kind) {
      case 'entity':
        return '#6ea8fe';
      case 'decision':
        return '#63e6be';
      case 'constraint':
      case 'requirement':
        return '#ffa94d';
      case 'qa':
        return '#da77f2';
      case 'chunk':
        return '#adb5bd';
      default:
        return '#74c0fc';
    }
  }

  const STATE_FILTER_OPTS: Array<{ value: MemoryState | ''; label: string }> = [
    { value: '', label: 'all' },
    { value: 'suggested', label: 'suggested' },
    { value: 'accepted', label: 'accepted' },
    { value: 'stale', label: 'stale' },
    { value: 'contradicted', label: 'contradicted' },
  ];
</script>

<div class="vault" class:has-selection={!!vault.selected || vault.mode === 'graph'}>
  <aside class="vault-side">
    <div class="vault-search">
      <input
        type="text"
        placeholder="Search memory…"
        bind:value={vault.query}
        oninput={onSearchInput}
      />
    </div>

    <div class="vault-toggle">
      <button class:active={vault.mode === 'list'} onclick={() => (vault.mode = 'list')}>
        Index
      </button>
      <button
        class:active={vault.mode === 'graph'}
        onclick={() => {
          vault.mode = 'graph';
          void vault.loadGraph();
        }}
      >
        Graph
      </button>
    </div>

    {#if vault.collections.length > 1}
      <div class="vault-chips">
        <button class:active={vault.collection === ''} onclick={() => (vault.collection = '')}>
          all
        </button>
        {#each vault.collections as c (c)}
          <button class:active={vault.collection === c} onclick={() => (vault.collection = c)}>
            {c}
          </button>
        {/each}
      </div>
    {/if}

    <!-- State filter chips -->
    <div class="vault-chips state-chips">
      {#each STATE_FILTER_OPTS as opt (opt.value)}
        <button
          class:active={vault.stateFilter === opt.value}
          onclick={() => (vault.stateFilter = opt.value)}
        >
          {opt.label}
        </button>
      {/each}
    </div>

    <!-- Merge mode toolbar -->
    <div class="merge-bar">
      {#if !vault.mergeMode}
        <button class="merge-enter" onclick={() => (vault.mergeMode = true)} title="Select memories to merge">
          Merge…
        </button>
        <button class="import-btn" onclick={() => (showImport = true)} title="Import AGENTS.md / CLAUDE.md / .cursorrules">
          Import…
        </button>
      {:else}
        <span class="merge-hint">{vault.mergeIds.length} selected</span>
        <button
          class="merge-go"
          disabled={vault.mergeIds.length < 2}
          onclick={() => (showMerge = true)}
        >
          Merge {vault.mergeIds.length}
        </button>
        <button onclick={() => { vault.mergeMode = false; vault.mergeIds = []; }}>
          Cancel
        </button>
      {/if}
    </div>

    <ul class="vault-list">
      {#each vault.visible as m (m.id)}
        <li>
          <button
            class="vault-item"
            class:active={vault.selected?.id === m.id}
            class:merge-selected={vault.mergeIds.includes(m.id)}
            onclick={() => pick(m)}
          >
            <span class="kind" style:background={nodeColor(m.kind)}>{m.kind}</span>
            <span class="title">{m.title}</span>
            {#if m.visibility === 'private'}<span class="lock" title="private">🔒</span>{/if}
          </button>
        </li>
      {:else}
        <li class="empty">No memories yet — run an analysis or ingest a story.</li>
      {/each}
    </ul>
  </aside>

  <main class="vault-main">
    <!-- Phone-only: return to the index/list (the two panes don't fit side by
         side on a phone, so the main pane covers the list when active). -->
    <button
      class="mobile-back"
      onclick={() => {
        if (vault.mode === 'graph') vault.mode = 'list';
        vault.selected = null;
      }}
    >
      ‹ Index
    </button>
    {#if vault.mode === 'graph'}
      <svg viewBox={`0 0 ${W} ${H}`} class="vault-graph" role="img" aria-label="Knowledge graph">
        {#each vault.graph?.edges ?? [] as e (e.src_id + e.dst_id + e.rel)}
          {@const a = positions.get(e.src_id)}
          {@const b = positions.get(e.dst_id)}
          {#if a && b}
            <line
              x1={a.x}
              y1={a.y}
              x2={b.x}
              y2={b.y}
              stroke="#888"
              stroke-opacity="0.4"
              stroke-dasharray={e.certainty === 'inferred' ? '4 3' : undefined}
            />
          {/if}
        {/each}
        {#each vault.graph?.nodes ?? [] as node (node.id)}
          {@const p = positions.get(node.id)}
          {#if p}
            <g class="g-node" transform={`translate(${p.x},${p.y})`}>
              <circle r="9" fill={nodeColor(node.kind)} />
              <text x="12" y="4" font-size="11" fill="currentColor">{node.label}</text>
            </g>
          {/if}
        {/each}
      </svg>
      <p class="hint">
        {vault.graph?.nodes.length ?? 0} nodes · {vault.graph?.edges.length ?? 0} links
      </p>
    {:else if vault.selected}
      {@const m = vault.selected}
      <header class="note-head">
        <h1>{m.title}</h1>
        <div class="badges">
          <span class="badge" style:background={nodeColor(m.kind)}>{m.kind}</span>
          <span class="badge">{m.collection}</span>
          <span class="badge">{m.visibility}</span>
          <!-- Governance state chip -->
          <MemoryStateBadge memory={m} />
          {#each m.tags as t (t)}<span class="tag">#{t}</span>{/each}
        </div>
        <div class="prov">
          source: {m.source_kind}{#if m.source_ref}
            · {m.source_ref}{/if} · confidence {m.confidence.toFixed(2)}
        </div>
        {#if m.superseded_by}
          <div class="prov warn">
            Superseded by: <code>{m.superseded_by}</code>
            {#if supersededMemory}
              <button class="inline-btn" onclick={() => (showProvDiff = !showProvDiff)}>
                {showProvDiff ? 'Hide diff' : 'Show diff'}
              </button>
            {/if}
          </div>
        {/if}
      </header>

      {#if showProvDiff && supersededMemory}
        <section class="prov-diff">
          <h3>Provenance diff (this → superseded)</h3>
          <DiffView before={m.body} after={supersededMemory.body} mode="word" contextLines={3} />
        </section>
      {/if}

      <article class="note-body">{@html renderMarkdown(m.body)}</article>

      {#if m.refs.length}
        <section class="refs">
          <h3>Provenance</h3>
          <ul>
            {#each m.refs as r (r.ref)}
              <li>
                {#if r.url}<a href={r.url} target="_blank" rel="noreferrer">{r.label ?? r.ref}</a>
                {:else}{r.label ?? r.ref}{/if}
                <span class="muted">({r.kind})</span>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if vault.backlinks.length}
        <section class="backlinks">
          <h3>Linked by ({vault.backlinks.length})</h3>
          <ul>
            {#each vault.backlinks as l (l.src_id + l.rel)}
              <li>{l.rel} ← {l.src_id}</li>
            {/each}
          </ul>
        </section>
      {/if}

      <footer class="note-actions">
        <span class="token-badge" title="Estimated token count (~4 chars/token)">
          ~{estimateTokens(m).toLocaleString()} tokens
        </span>
        <button class="copy-btn" onclick={() => void copyAsJson(m)} title="Copy memory as JSON">
          Copy JSON
        </button>
        <button onclick={() => (showSplit = true)} title="Split this memory into parts">
          Split…
        </button>
        <!-- Governance forget with undo affordance -->
        {#if vault._pendingUndo}
          <button class="undo-btn" onclick={() => void vault.undoForget()}>
            Undo forget
          </button>
        {:else}
          <button class="danger" onclick={() => void vault.softForget(m)}>Forget</button>
        {/if}
      </footer>
    {:else}
      <div class="placeholder">Select a memory to read it.</div>
    {/if}
  </main>
</div>

<!-- Dialogs -->
{#if showImport}
  <ImportGovDialog onclose={() => (showImport = false)} />
{/if}

{#if showMerge && mergeSources.length >= 2}
  <MergeDialog sources={mergeSources} onclose={() => (showMerge = false)} />
{/if}

{#if showSplit && vault.selected}
  <SplitDialog source={vault.selected} onclose={() => (showSplit = false)} />
{/if}

<style>
  .vault {
    display: grid;
    grid-template-columns: 300px 1fr;
    height: 100%;
    overflow: hidden;
  }
  .vault-side {
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--border, #2a2a2a);
    min-height: 0;
  }
  .vault-search {
    padding: 8px;
  }
  .vault-search input {
    width: 100%;
    padding: 6px 8px;
    border-radius: 6px;
  }
  .vault-toggle,
  .vault-chips {
    display: flex;
    gap: 4px;
    padding: 0 8px 8px;
    flex-wrap: wrap;
  }
  .vault-toggle button,
  .vault-chips button {
    font-size: 11px;
    padding: 3px 8px;
    border-radius: 6px;
    opacity: 0.7;
  }
  .vault-toggle button.active,
  .vault-chips button.active {
    opacity: 1;
    font-weight: 600;
  }
  .state-chips {
    border-top: 1px solid var(--border, #2a2a2a);
    padding-top: 6px;
  }
  .merge-bar {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 4px 8px 6px;
    border-bottom: 1px solid var(--border, #2a2a2a);
    flex-wrap: wrap;
  }
  .merge-bar button {
    font-size: 11px;
    padding: 3px 9px;
    border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: transparent;
    color: var(--text-dim, #aaa);
    cursor: pointer;
  }
  .merge-enter, .import-btn {
    opacity: 0.8;
  }
  .merge-go {
    background: var(--accent, #4c6ef5) !important;
    color: #fff !important;
    border-color: transparent !important;
  }
  .merge-go:disabled { opacity: 0.4 !important; cursor: default !important; }
  .merge-hint { font-size: 11px; opacity: 0.6; }
  .vault-list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    flex: 1;
  }
  .vault-item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    text-align: start;
    padding: 6px 8px;
    border: none;
    background: none;
  }
  .vault-item.active {
    background: var(--surface-2, #1e2330);
  }
  .vault-item.merge-selected {
    outline: 2px solid var(--accent, #4c6ef5);
    outline-offset: -2px;
  }
  .vault-item .kind {
    font-size: 9px;
    padding: 1px 5px;
    border-radius: 4px;
    color: #000;
  }
  .vault-item .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty,
  .placeholder {
    padding: 16px;
    opacity: 0.6;
    font-size: 13px;
  }
  .vault-main {
    overflow-y: auto;
    padding: 16px 24px;
    min-height: 0;
  }
  .note-head h1 {
    margin: 0 0 6px;
  }
  .badges {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
  }
  .badge {
    font-size: 10px;
    padding: 2px 6px;
    border-radius: 5px;
    background: var(--surface-2, #1e2330);
  }
  .tag {
    font-size: 11px;
    opacity: 0.7;
  }
  .prov {
    font-size: 11px;
    opacity: 0.6;
    margin-top: 6px;
  }
  .prov.warn {
    opacity: 0.85;
    color: #fab005;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .prov.warn code {
    font-size: 10px;
    opacity: 0.8;
    word-break: break-all;
  }
  .inline-btn {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid currentColor;
    background: transparent;
    cursor: pointer;
    color: inherit;
  }
  .prov-diff {
    margin: 12px 0;
  }
  .prov-diff h3 {
    font-size: 12px;
    opacity: 0.7;
    margin: 0 0 6px;
  }
  .note-body {
    margin-top: 14px;
    line-height: 1.55;
  }
  .refs,
  .backlinks {
    margin-top: 18px;
    font-size: 13px;
  }
  .muted {
    opacity: 0.5;
  }
  .vault-graph {
    width: 100%;
    height: calc(100% - 24px);
    color: var(--text, #ddd);
  }
  .hint {
    font-size: 11px;
    opacity: 0.6;
    text-align: center;
  }
  .note-actions {
    margin-top: 24px;
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .token-badge {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--surface-2, #1e2330);
    border: 1px solid var(--border, #333);
    color: var(--text-dim, #888);
  }
  .copy-btn {
    font-size: 11.5px;
    padding: 2px 10px;
    border-radius: 4px;
    border: 1px solid var(--border, #333);
    background: transparent;
    cursor: pointer;
    color: var(--text-dim, #aaa);
  }
  .copy-btn:hover {
    background: var(--surface-2, #1e2330);
  }
  .danger {
    color: #ff6b6b;
    font-size: 12px;
  }
  .undo-btn {
    font-size: 12px;
    padding: 3px 10px;
    border-radius: 5px;
    border: 1px solid #fab005;
    color: #fab005;
    background: transparent;
    cursor: pointer;
  }
  .undo-btn:hover {
    background: color-mix(in srgb, #fab005 15%, transparent);
  }

  /* The phone "‹ Index" back button is hidden on desktop/tablet (two panes
     show side by side); it only appears in the phone single-column layout. */
  .mobile-back {
    display: none;
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     The desktop layout is a fixed two-column grid (300px sidebar | reader). On a
     phone that 300px sidebar leaves the reader/graph a ~120px sliver where the
     note title wraps one word per line and the SVG graph spills off-screen. On a
     phone we make it a single full-width column that swaps between the INDEX
     (search + filters + note list) and the OPEN note / graph — controlled by the
     `.has-selection` class (set when a note is selected or the graph is shown).
     A "‹ Index" back button returns to the list. Each pane scrolls on its own. */
  @media (max-width: 640px) {
    .vault {
      grid-template-columns: 1fr;
      grid-template-rows: 1fr;
    }
    /* The sidebar (index) fills the page when nothing is open… */
    .vault-side {
      grid-row: 1;
      grid-column: 1;
      width: 100%;
      border-inline-end: none;
      min-height: 0;
      overflow: hidden;
    }
    /* …and the reader/graph occupies the SAME cell, stacked on top, only when a
       note is open or the graph is shown. */
    .vault-main {
      grid-row: 1;
      grid-column: 1;
      display: none;
      padding: 0 14px 20px;
      -webkit-overflow-scrolling: touch;
    }
    .vault.has-selection .vault-side {
      display: none;
    }
    .vault.has-selection .vault-main {
      display: block;
    }

    /* Bigger, legible search box + tap targets. */
    .vault-search input {
      padding: 10px 12px;
      font-size: 16px; /* ≥16px stops iOS Safari from zooming on focus */
    }
    .vault-toggle button,
    .vault-chips button {
      font-size: 13px;
      padding: 7px 12px;
      min-height: 36px;
    }
    .merge-bar button {
      font-size: 13px;
      padding: 7px 12px;
      min-height: 36px;
    }
    /* The note list: roomy rows with readable titles, its own scroll. */
    .vault-list {
      -webkit-overflow-scrolling: touch;
    }
    .vault-item {
      padding: 12px 10px;
      gap: 8px;
    }
    .vault-item .kind {
      font-size: 11px;
      padding: 2px 7px;
    }
    .vault-item .title {
      font-size: 15px;
    }

    /* Phone back bar to leave the reader/graph and return to the index. */
    .mobile-back {
      display: block;
      position: sticky;
      top: 0;
      z-index: 2;
      width: 100%;
      text-align: start;
      padding: 12px 4px;
      margin: 0 -14px 6px;
      padding-inline-start: 14px;
      border: none;
      border-bottom: 1px solid var(--border, #2a2a2a);
      background: var(--bg, #0d1117);
      color: var(--accent-fg, #74c0fc);
      font-size: 15px;
      font-weight: 600;
      cursor: pointer;
    }

    /* Reader: prevent the long words / code / refs from forcing the page wider
       than the viewport. */
    .note-head h1 {
      font-size: 22px;
      overflow-wrap: anywhere;
    }
    .note-body {
      overflow-wrap: anywhere;
      font-size: 15px;
    }
    .note-body :global(pre),
    .note-body :global(code) {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }
    .prov.warn code {
      overflow-wrap: anywhere;
    }
    .refs,
    .backlinks {
      overflow-wrap: anywhere;
    }
    .note-actions button,
    .note-actions .copy-btn {
      min-height: 36px;
    }

    /* Graph: cap the SVG to the viewport so it pans/fits instead of spilling. */
    .vault-graph {
      width: 100%;
      max-width: 100%;
      height: 70vh;
    }
    .placeholder {
      /* When nothing's selected on a phone, the index covers the page, so this
         placeholder is never seen — but keep it sane if it ever shows. */
      text-align: center;
    }
  }
</style>
