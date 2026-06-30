<script lang="ts">
  // Knowledge tab — the original Obsidian-like memory browser: index + hybrid
  // search + note reader + backlinks, with lifecycle governance (state chips,
  // forget-with-undo, merge, split, provenance diff, governed import) and the
  // embedder config panel. Vault v2 adds "why selected" chips on each search
  // hit (vector / keyword / symbol / … reasons) and a quick Add-doc form.
  import { vault, OLLAMA_EMBED_MODELS } from './vault.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { renderMarkdown } from '../../lib/md';
  import type { ContextReason, Memory, MemoryHit, VaultDocReq } from '../../lib/api/types';
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

  // Add-doc inline form.
  let showAddDoc = $state(false);
  let docTitle = $state('');
  let docBody = $state('');
  let docRepoId = $state('');
  let docBusy = $state(false);

  // Embedder settings panel.
  let showEmbedder = $state(false);
  let embProvider = $state<'local' | 'ollama' | 'openai' | 'voyage'>('local');
  let embKey = $state('');
  let embOllamaModel = $state(OLLAMA_EMBED_MODELS[0].model);
  let embCustomModel = $state('');
  let embOllamaUrl = $state('');

  // Keep the provider selector in sync with the loaded status. A legacy `stub`
  // status maps onto the "local" option (both are the keyless local path).
  $effect(() => {
    const p = vault.embedder?.provider;
    if (p === 'local' || p === 'ollama' || p === 'openai' || p === 'voyage') embProvider = p;
    else if (p === 'stub') embProvider = 'local';
  });

  async function applyEmbedder(): Promise<void> {
    // Resolve the model: a known list entry, or a custom Ollama model name.
    const custom = embOllamaModel === '__custom__';
    const model = custom ? embCustomModel.trim() : embOllamaModel;
    const known = OLLAMA_EMBED_MODELS.find((x) => x.model === model);
    if (embProvider === 'ollama' && !model) return; // need a model name
    await vault.setEmbedder(embProvider, {
      apiKey: embKey,
      ollamaModel: model,
      ollamaDim: known?.dim, // unknown → backend default; stored dim auto-corrects
      ollamaUrl: embOllamaUrl,
    });
    embKey = '';
    // Switching the embedder leaves old vectors under the prior model — re-embed.
    if (embProvider !== 'local') await vault.reindex();
  }

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

  async function openAddDoc() {
    showAddDoc = true;
    if (!vault.repos.length) void vault.loadRepos();
  }

  async function submitDoc() {
    if (!docTitle.trim() || !docBody.trim()) return;
    docBusy = true;
    try {
      const req: VaultDocReq = { title: docTitle.trim(), body: docBody.trim() };
      if (docRepoId) req.repo_id = docRepoId;
      const created = await vault.addDoc(req);
      if (created) {
        showAddDoc = false;
        docTitle = '';
        docBody = '';
        docRepoId = '';
        void vault.select(created);
      }
    } finally {
      docBusy = false;
    }
  }

  // Merge sources from the store (resolved Memory objects).
  const mergeSources = $derived.by(() => {
    return vault.items.filter((m) => vault.mergeIds.includes(m.id));
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

  // "Why selected" chips: colour each reason by its signal kind.
  function reasonColor(kind: string): string {
    switch (kind) {
      case 'vector':
        return '#6ea8fe';
      case 'keyword':
        return '#7ee787';
      case 'symbol':
        return '#da77f2';
      case 'graph':
        return '#63e6be';
      case 'recent':
        return '#adb5bd';
      case 'test':
        return '#ffa94d';
      case 'doc':
        return '#ff9ff3';
      case 'scope':
        return '#4dd2e6';
      default:
        return '#74c0fc';
    }
  }

  /** The chip's secondary text: prefer a human detail, else the score. */
  function reasonValue(r: ContextReason): string {
    const d = r.detail?.trim();
    return d ? d : r.score.toFixed(2);
  }

  /** The hit for a memory id (present only while a search is active). */
  function hitFor(id: string): MemoryHit | undefined {
    return vault.query.trim() ? vault.hitsById.get(id) : undefined;
  }

  const STATE_FILTER_OPTS: Array<{ value: MemoryState | ''; label: string }> = [
    { value: '', label: 'all' },
    { value: 'suggested', label: 'suggested' },
    { value: 'accepted', label: 'accepted' },
    { value: 'stale', label: 'stale' },
    { value: 'contradicted', label: 'contradicted' },
  ];
</script>

<div class="vault" class:has-selection={!!vault.selected}>
  <aside class="vault-side">
    <div class="vault-search">
      <input
        type="text"
        placeholder="Search memory…"
        bind:value={vault.query}
        oninput={onSearchInput}
      />
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
        <button class="import-btn" onclick={openAddDoc} title="Add a knowledge doc">
          + Doc
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

    <!-- Add-doc inline form -->
    {#if showAddDoc}
      <div class="adddoc-form">
        <input type="text" placeholder="Doc title…" bind:value={docTitle} />
        <textarea placeholder="Markdown body…" rows="4" bind:value={docBody}></textarea>
        {#if vault.repos.length}
          <select bind:value={docRepoId} title="Link this doc to a repo (optional)">
            <option value="">No repo</option>
            {#each vault.repos as r (r.id)}
              <option value={r.id}>{r.name}</option>
            {/each}
          </select>
        {/if}
        <div class="adddoc-actions">
          <button
            class="adddoc-save"
            disabled={docBusy || !docTitle.trim() || !docBody.trim()}
            onclick={submitDoc}
          >
            {docBusy ? 'Saving…' : 'Save doc'}
          </button>
          <button onclick={() => (showAddDoc = false)}>Cancel</button>
        </div>
      </div>
    {/if}

    <!-- Embedder status + settings (semantic search backend) -->
    {#if vault.embedder}
      <div class="embedder-bar" data-testid="vault-embedder">
        <button
          class="embedder-summary"
          onclick={() => (showEmbedder = !showEmbedder)}
          title="Configure the semantic-search embedder"
        >
          <span class="embedder-dot" class:active={vault.embedder.active}></span>
          <span class="embedder-label">
            Embedder: {vault.embedder.model ?? vault.embedder.provider}
            {#if vault.embedder.dim}<span class="dim">· {vault.embedder.dim}d</span>{/if}
          </span>
          <span class="embedder-caret">{showEmbedder ? '▾' : '▸'}</span>
        </button>
        {#if showEmbedder}
          <div class="embedder-form">
            <label class="embedder-row">
              <span>Provider</span>
              <select bind:value={embProvider} data-testid="embedder-provider">
                <option value="local">Local — code-aware (no key, no install)</option>
                <option value="ollama">Ollama — local neural (needs Ollama running)</option>
                <option value="openai">OpenAI (API key)</option>
                <option value="voyage">Voyage (API key)</option>
              </select>
            </label>
            <p class="embedder-note">
              {#if embProvider === 'local'}
                The default. Deterministic, offline, no setup. Good baseline; upgrade to a neural
                provider for stronger semantic recall.
              {:else if embProvider === 'ollama'}
                Real <b>local</b> neural embeddings via a localhost Ollama server (no API key). Install
                it from the <b>Backends</b> tab, or run <code>ollama pull nomic-embed-text</code>.
              {:else}
                Neural embeddings via {embProvider === 'openai' ? 'OpenAI' : 'Voyage'} (cloud API key).
                {#if embProvider === 'voyage'}Anthropic recommends Voyage — Claude has no embeddings API.{/if}
              {/if}
            </p>
            {#if embProvider === 'openai' || embProvider === 'voyage'}
              <label class="embedder-row">
                <span>API key</span>
                <input
                  type="password"
                  placeholder={vault.embedder.key_present ? '•••• (stored)' : 'paste key…'}
                  bind:value={embKey}
                  data-testid="embedder-key"
                />
              </label>
            {/if}
            {#if embProvider === 'ollama'}
              <label class="embedder-row">
                <span>Model</span>
                <select bind:value={embOllamaModel} data-testid="embedder-ollama-model">
                  {#each OLLAMA_EMBED_MODELS as m (m.model)}
                    <option value={m.model}>{m.model} · {m.dim}d — {m.note}</option>
                  {/each}
                  <option value="__custom__">Custom model…</option>
                </select>
              </label>
              {#if embOllamaModel === '__custom__'}
                <label class="embedder-row">
                  <span>Custom</span>
                  <input
                    type="text"
                    placeholder="exact ollama model name (e.g. nomic-embed-text-v2-moe)"
                    bind:value={embCustomModel}
                    data-testid="embedder-custom-model"
                  />
                </label>
              {/if}
              <label class="embedder-row">
                <span>URL</span>
                <input type="text" placeholder="http://127.0.0.1:11434" bind:value={embOllamaUrl} />
              </label>
              <p class="embedder-note">
                Run <code>ollama pull {embOllamaModel === '__custom__' ? embCustomModel || '<model>' : embOllamaModel}</code>
                first. Any Ollama embedding model works — its dimension is detected automatically.
                Applying re-embeds existing memories under the new model (may take a while).
              </p>
            {/if}
            <div class="embedder-actions">
              <button
                class="embedder-apply"
                disabled={vault.embedderBusy}
                onclick={applyEmbedder}
                data-testid="embedder-apply"
              >
                {vault.embedderBusy ? 'Saving…' : 'Apply'}
              </button>
              <button
                class="embedder-reindex"
                disabled={vault.embedderBusy || !vault.embedder.active}
                onclick={() => vault.reindex()}
                title="Re-embed existing memories under the active model"
              >
                Reindex
              </button>
            </div>
          </div>
        {/if}
      </div>
    {/if}

    <ul class="vault-list">
      {#each vault.visible as m (m.id)}
        {@const hit = hitFor(m.id)}
        <li>
          <button
            class="vault-item"
            class:active={vault.selected?.id === m.id}
            class:merge-selected={vault.mergeIds.includes(m.id)}
            onclick={() => pick(m)}
          >
            <div class="item-row">
              <span class="kind" style:background={nodeColor(m.kind)}>{m.kind}</span>
              <span class="title">{m.title}</span>
              {#if m.visibility === 'private'}<span class="lock" title="private">🔒</span>{/if}
            </div>
            {#if hit}
              <div class="reasons">
                {#if hit.reasons && hit.reasons.length}
                  {#each hit.reasons as r, i (i)}
                    <span
                      class="reason"
                      style:--c={reasonColor(r.kind)}
                      title={`${r.kind}: ${r.detail || 'matched'} (score ${r.score.toFixed(2)})`}
                    >
                      <b>{r.kind}</b> {reasonValue(r)}
                    </span>
                  {/each}
                {:else}
                  {#each hit.why as w, i (i)}
                    <span class="reason" style:--c="#74c0fc" title={w}>{w}</span>
                  {/each}
                  <span class="reason score-only" style:--c="#7ee787">score {hit.score.toFixed(2)}</span>
                {/if}
              </div>
            {/if}
          </button>
        </li>
      {:else}
        <li class="empty">No memories yet — run an analysis or ingest a story.</li>
      {/each}
    </ul>
  </aside>

  <main class="vault-main">
    <!-- Phone-only: return to the index/list. -->
    <button class="mobile-back" onclick={() => (vault.selected = null)}>
      ‹ Index
    </button>
    {#if vault.selected}
      {@const m = vault.selected}
      <header class="note-head">
        <h1>{m.title}</h1>
        <div class="badges">
          <span class="badge kind-badge" style:--c={nodeColor(m.kind)}>{m.kind}</span>
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
        <!-- "Why selected" for the open note (when arrived via search) -->
        {#if hitFor(m.id)}
          {@const hit = hitFor(m.id)}
          {#if hit && hit.reasons && hit.reasons.length}
            <div class="reasons reasons-head">
              <span class="why-label">why selected:</span>
              {#each hit.reasons as r (r.kind + r.detail)}
                <span
                  class="reason"
                  style:--c={reasonColor(r.kind)}
                  title={`${r.kind}: ${r.detail || 'matched'} (score ${r.score.toFixed(2)})`}
                >
                  <b>{r.kind}</b> {reasonValue(r)}
                </span>
              {/each}
            </div>
          {/if}
        {/if}
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
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
  }
  .vault-search input::placeholder {
    color: var(--text-dim);
  }
  .embedder-row input,
  .embedder-row select {
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 5px 7px;
  }
  .vault-chips {
    display: flex;
    gap: 4px;
    padding: 0 8px 8px;
    flex-wrap: wrap;
  }
  .vault-chips button {
    font-size: 11px;
    padding: 3px 8px;
    border-radius: 6px;
    opacity: 0.7;
  }
  .vault-chips button.active {
    opacity: 1;
    font-weight: 600;
    color: #0b0b0b;
    background: #7ee787;
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
  .adddoc-form {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 8px;
    border-bottom: 1px solid var(--border, #2a2a2a);
  }
  .adddoc-form input,
  .adddoc-form textarea,
  .adddoc-form select {
    width: 100%;
    font-size: 12px;
    padding: 5px 7px;
    border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: var(--surface, transparent);
    color: var(--text, #ddd);
    resize: vertical;
  }
  .adddoc-actions {
    display: flex;
    gap: 6px;
  }
  .adddoc-actions button {
    font-size: 11px;
    padding: 4px 10px;
    border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: transparent;
    color: var(--text-dim, #aaa);
    cursor: pointer;
  }
  .adddoc-save {
    background: #7ee787 !important;
    color: #0b0b0b !important;
    border-color: #7ee787 !important;
  }
  .adddoc-actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .embedder-bar {
    padding: 4px 8px 6px;
    border-bottom: 1px solid var(--border, #2a2a2a);
    font-size: 11px;
  }
  .embedder-summary {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--text-dim, #aaa);
    cursor: pointer;
    padding: 2px 0;
    font-size: 11px;
  }
  .embedder-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim, #888);
    flex: none;
  }
  .embedder-dot.active {
    background: #7ee787;
  }
  .embedder-label {
    flex: 1;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .embedder-label .dim {
    opacity: 0.6;
  }
  .embedder-form {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 6px 0 2px;
  }
  .embedder-row {
    display: flex;
    align-items: center;
    gap: 6px;
    justify-content: space-between;
  }
  .embedder-row span {
    color: var(--text-dim, #aaa);
  }
  .embedder-row select,
  .embedder-row input {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    padding: 3px 5px;
    border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: var(--bg-elev, transparent);
    color: var(--text, #ddd);
  }
  .embedder-actions {
    display: flex;
    gap: 6px;
  }
  .embedder-actions button {
    font-size: 11px;
    padding: 3px 9px;
    border-radius: 5px;
    border: 1px solid var(--border, #444);
    background: transparent;
    color: var(--text-dim, #aaa);
    cursor: pointer;
  }
  .embedder-apply {
    background: var(--accent, #4c6ef5) !important;
    color: #fff !important;
    border-color: var(--accent, #4c6ef5) !important;
  }
  .embedder-actions button:disabled {
    opacity: 0.5;
    cursor: default;
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
    flex-direction: column;
    gap: 4px;
    width: 100%;
    text-align: start;
    padding: 6px 8px;
    border: none;
    background: none;
  }
  .item-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
  }
  .vault-item.active {
    background: var(--surface-2, #1e2330);
    box-shadow: inset 2px 0 0 #7ee787;
  }
  .vault-item.merge-selected {
    outline: 2px solid #7ee787;
    outline-offset: -2px;
  }
  .vault-item .kind {
    font-size: 9px;
    padding: 1px 5px;
    border-radius: 4px;
    color: #000;
    flex: none;
  }
  .vault-item .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .reasons {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding-inline-start: 2px;
  }
  .reason {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 9.5px;
    line-height: 1.4;
    padding: 1px 6px;
    border-radius: 999px;
    /* Calm, themed chip: a faint kind-tinted fill/border (works on any bg) with
       neutral, high-contrast text. The kind word (b) carries the only emphasis. */
    border: 1px solid color-mix(in srgb, var(--c) 35%, var(--border));
    background: color-mix(in srgb, var(--c) 12%, transparent);
    color: var(--text-dim);
    white-space: nowrap;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .reason b {
    font-weight: 700;
    color: var(--text);
  }
  .reasons-head {
    margin-top: 8px;
    align-items: center;
  }
  .reasons-head .reason {
    font-size: 11px;
    padding: 2px 8px;
  }
  .why-label {
    font-size: 11px;
    opacity: 0.6;
    margin-inline-end: 2px;
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
    padding: 2px 7px;
    border-radius: 5px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  /* Kind badge keeps a faint taxonomy tint on its border; text stays readable. */
  .badge.kind-badge {
    color: var(--text);
    border-color: color-mix(in srgb, var(--c) 35%, var(--border));
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
    color: var(--status-warn);
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
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .copy-btn {
    font-size: 11.5px;
    padding: 2px 10px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: transparent;
    cursor: pointer;
    color: var(--text);
  }
  .copy-btn:hover {
    background: var(--surface-2);
  }
  .danger {
    font-size: 11.5px;
    padding: 2px 10px;
    border-radius: 4px;
    border: 1px solid color-mix(in srgb, #f87171 40%, var(--border));
    background: transparent;
    cursor: pointer;
    color: #f87171;
  }
  .danger:hover {
    background: color-mix(in srgb, #f87171 14%, transparent);
  }
  .undo-btn {
    font-size: 12px;
    padding: 3px 10px;
    border-radius: 5px;
    border: 1px solid var(--status-warn);
    color: var(--status-warn);
    background: transparent;
    cursor: pointer;
  }
  .undo-btn:hover {
    background: var(--status-warn-soft);
  }

  /* The phone "‹ Index" back button is hidden on desktop/tablet. */
  .mobile-back {
    display: none;
  }

  @media (max-width: 640px) {
    .vault {
      grid-template-columns: 1fr;
      grid-template-rows: 1fr;
    }
    .vault-side {
      grid-row: 1;
      grid-column: 1;
      width: 100%;
      border-inline-end: none;
      min-height: 0;
      overflow: hidden;
    }
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

    .vault-search input {
      padding: 10px 12px;
      font-size: 16px; /* ≥16px stops iOS Safari from zooming on focus */
    }
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
    .placeholder {
      text-align: center;
    }
  }
</style>
