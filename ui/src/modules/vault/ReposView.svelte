<script lang="ts">
  // Repos tab — indexed code repositories with status/counts, an "Index a repo"
  // form, and a per-repo "View graph" jump (scopes the Graph tab to that repo).
  import { onMount } from 'svelte';
  import { vault } from './vault.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { CodeRepo } from '../../lib/api/types';

  let root = $state('');
  let name = $state('');

  onMount(() => {
    void vault.loadRepos();
  });

  async function doIndex() {
    const r = await vault.indexRepo(root, name);
    if (r) {
      root = '';
      name = '';
    }
  }

  function statusClass(s: string): string {
    if (s === 'ready') return 'ok';
    if (s === 'indexing') return 'busy';
    if (s === 'error') return 'err';
    return 'idle';
  }

  function fmtDate(s: string | null): string {
    if (!s) return '—';
    const d = new Date(s);
    return Number.isNaN(d.getTime()) ? s : d.toLocaleString();
  }

  function reindex(r: CodeRepo) {
    void vault.indexRepo(r.root, r.name);
  }
</script>

<div class="repos">
  <section class="index-card">
    <h2><Icon name="plus" size={15} /> Index a repository</h2>
    <p class="hint">Point Otto at a local repo to build its code graph (files, symbols, calls, http/db edges).</p>
    <div class="index-form">
      <input
        class="path"
        type="text"
        placeholder="/absolute/path/to/repo"
        bind:value={root}
        onkeydown={(e) => e.key === 'Enter' && root.trim() && doIndex()}
      />
      <input class="name" type="text" placeholder="Name (optional)" bind:value={name} />
      <button class="index-btn" disabled={vault.indexing || !root.trim()} onclick={doIndex}>
        {#if vault.indexing}Indexing…{:else}<Icon name="zap" size={13} /> Index{/if}
      </button>
    </div>
    {#if vault.lastIndex}
      <div class="last-index">
        Indexed: <b>{vault.lastIndex.files}</b> files · <b>{vault.lastIndex.symbols}</b> symbols ·
        <b>{vault.lastIndex.edges}</b> edges · <b>{vault.lastIndex.chunks}</b> chunks
      </div>
    {/if}
  </section>

  <section class="list">
    <div class="list-head">
      <h2>Indexed repositories <span class="muted">({vault.repos.length})</span></h2>
      <button class="refresh" onclick={() => vault.loadRepos()} title="Refresh">
        <Icon name="refresh" size={13} />
      </button>
    </div>

    {#if vault.reposLoading && !vault.repos.length}
      <p class="empty">Loading…</p>
    {:else if !vault.repos.length}
      <p class="empty">No repositories indexed yet. Use the form above to index one.</p>
    {:else}
      <div class="cards">
        {#each vault.repos as r (r.id)}
          <article class="repo-card">
            <header>
              <Icon name="box" size={15} />
              <span class="repo-name" title={r.root}>{r.name}</span>
              <span class="status {statusClass(r.status)}">{r.status}</span>
            </header>
            <div class="repo-root" title={r.root}>{r.root}</div>
            {#if r.message}
              <div class="repo-msg {statusClass(r.status)}">{r.message}</div>
            {/if}
            <div class="counts">
              <span><b>{r.files}</b> files</span>
              <span><b>{r.symbols}</b> symbols</span>
              <span><b>{r.edges}</b> edges</span>
              <span><b>{r.chunks}</b> chunks</span>
            </div>
            <div class="meta">
              {#if r.head}<span class="head" title="HEAD"><Icon name="commit" size={11} /> {r.head.slice(0, 8)}</span>{/if}
              <span class="when">indexed {fmtDate(r.indexed_at)}</span>
            </div>
            <footer>
              <button class="card-btn primary" onclick={() => vault.openRepoGraph(r.id)}>
                <Icon name="branch" size={13} /> View graph
              </button>
              <button class="card-btn" disabled={vault.indexing} onclick={() => reindex(r)} title="Re-index this repo">
                <Icon name="refresh" size={13} /> Re-index
              </button>
            </footer>
          </article>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .repos {
    height: 100%;
    overflow-y: auto;
    padding: 18px 22px 28px;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 15px;
    margin: 0 0 4px;
  }
  .hint { font-size: 12px; color: var(--text-dim); margin: 0 0 10px; }
  .index-card {
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 14px 16px;
    background: var(--surface);
    margin-bottom: 22px;
    max-width: 820px;
  }
  .index-form { display: flex; gap: 8px; flex-wrap: wrap; }
  .index-form input {
    font-size: 13px;
    padding: 7px 9px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
  }
  .index-form .path { flex: 2; min-width: 220px; }
  .index-form .name { flex: 1; min-width: 130px; }
  .index-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 13px;
    font-weight: 600;
    padding: 7px 14px;
    border-radius: 7px;
    border: 1px solid #7ee787;
    background: #7ee787;
    color: #0b0b0b;
    cursor: pointer;
  }
  .index-btn:disabled { opacity: 0.45; cursor: default; }
  .last-index {
    margin-top: 10px;
    font-size: 12px;
    color: var(--text-dim);
    padding: 7px 10px;
    border-radius: 7px;
    background: color-mix(in srgb, #7ee787 12%, transparent);
    border: 1px solid color-mix(in srgb, #7ee787 35%, transparent);
  }

  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }
  .refresh {
    display: inline-flex;
    padding: 5px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .refresh:hover { color: var(--text); }
  .empty { font-size: 13px; color: var(--text-dim); }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
  }
  .repo-card {
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 12px 14px;
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .repo-card header {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .repo-name {
    flex: 1;
    font-weight: 600;
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 2px 7px;
    border-radius: 999px;
  }
  .status.ok { background: color-mix(in srgb, #7ee787 22%, transparent); color: #4cae5a; }
  .status.busy { background: color-mix(in srgb, #ffd43b 22%, transparent); color: #c79400; }
  .status.err { background: color-mix(in srgb, #ff6b6b 22%, transparent); color: #ff6b6b; }
  .status.idle { background: var(--surface-2); color: var(--text-dim); }
  .repo-root {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono, monospace);
  }
  .repo-msg { font-size: 11px; }
  .repo-msg.err { color: #ff6b6b; }
  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .counts b { color: var(--text); }
  .meta {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .head { display: inline-flex; align-items: center; gap: 3px; font-family: var(--font-mono, monospace); }
  footer { display: flex; gap: 7px; margin-top: 2px; }
  .card-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    padding: 5px 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .card-btn:hover:not(:disabled) { color: var(--text); }
  .card-btn:disabled { opacity: 0.45; cursor: default; }
  .card-btn.primary {
    background: #7ee787;
    color: #0b0b0b;
    border-color: #7ee787;
    font-weight: 600;
  }
  .muted { opacity: 0.55; font-weight: 400; }
</style>
