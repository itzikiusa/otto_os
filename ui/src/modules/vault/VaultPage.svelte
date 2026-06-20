<script lang="ts">
  // Obsidian-like memory vault: index + hybrid search + note reader + backlinks
  // + a dependency-free SVG knowledge graph. Scoped to the current workspace.
  import { vault } from './vault.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { renderMarkdown } from '../../lib/md';
  import type { Memory } from '../../lib/api/types';
  import { copyAsJson } from '../../lib/components/exporters';

  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    // Reload when the workspace changes.
    if (ws.currentId) void vault.load();
  });

  function onSearchInput() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void vault.search(), 250);
  }

  function pick(m: Memory) {
    void vault.select(m);
  }

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
</script>

<div class="vault">
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

    <ul class="vault-list">
      {#each vault.visible as m (m.id)}
        <li>
          <button
            class="vault-item"
            class:active={vault.selected?.id === m.id}
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
          {#each m.tags as t (t)}<span class="tag">#{t}</span>{/each}
        </div>
        <div class="prov">
          source: {m.source_kind}{#if m.source_ref}
            · {m.source_ref}{/if} · confidence {m.confidence.toFixed(2)}
        </div>
      </header>

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
        <button class="danger" onclick={() => void vault.forget(m)}>Forget</button>
      </footer>
    {:else}
      <div class="placeholder">Select a memory to read it.</div>
    {/if}
  </main>
</div>

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
    border-right: 1px solid var(--border, #2a2a2a);
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
    text-align: left;
    padding: 6px 8px;
    border: none;
    background: none;
  }
  .vault-item.active {
    background: var(--surface-2, #1e2330);
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
</style>
