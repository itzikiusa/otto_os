<script lang="ts">
  // Symbols tab — a fast symbol browser: search → /vault/symbols, with a repo
  // filter and a results table (name, kind, file:line, signature).
  import { onMount } from 'svelte';
  import { vault } from './vault.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { CodeSymbol } from '../../lib/api/types';

  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    if (!vault.repos.length) void vault.loadRepos();
    void vault.searchSymbols();
  });

  function onInput() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void vault.searchSymbols(), 220);
  }

  function onRepoChange() {
    void vault.searchSymbols();
  }

  function kindColor(kind: string): string {
    switch (kind) {
      case 'function':
      case 'method': return '#6ea8fe';
      case 'struct':
      case 'class':
      case 'type': return '#63e6be';
      case 'interface':
      case 'trait': return '#da77f2';
      case 'enum': return '#ffa94d';
      case 'const':
      case 'var': return '#adb5bd';
      default: return '#74c0fc';
    }
  }

  function openInGraph(s: CodeSymbol) {
    void vault.openRepoGraph(s.repo_id);
  }
</script>

<div class="symbols">
  <div class="sym-bar">
    <div class="sym-search">
      <Icon name="search" size={14} />
      <input
        type="text"
        placeholder="Search symbols (name, signature)…"
        bind:value={vault.symbolQuery}
        oninput={onInput}
      />
    </div>
    <select class="repo-filter" bind:value={vault.symbolRepoId} onchange={onRepoChange} aria-label="Filter by repository">
      <option value="">All repos</option>
      {#each vault.repos as r (r.id)}
        <option value={r.id}>{r.name}</option>
      {/each}
    </select>
    <span class="count">{vault.symbols.length} result{vault.symbols.length === 1 ? '' : 's'}</span>
  </div>

  <div class="sym-table-wrap">
    {#if vault.symbolsLoading && !vault.symbols.length}
      <p class="empty">Searching…</p>
    {:else if !vault.symbols.length}
      <p class="empty">No symbols. Index a repo in the Repos tab, then search here.</p>
    {:else}
      <table class="sym-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Kind</th>
            <th>Lang</th>
            <th>Location</th>
            <th>Signature</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each vault.symbols as s (s.id)}
            <tr>
              <td class="name">{s.name}</td>
              <td><span class="kind" style:--c={kindColor(s.kind)}>{s.kind}</span></td>
              <td class="lang">{s.lang}</td>
              <td class="loc" title={`${s.file}:${s.line}`}>{s.file}:{s.line}</td>
              <td class="sig" title={s.signature}>{s.signature}</td>
              <td class="actions">
                <button class="mini" onclick={() => openInGraph(s)} title="Open this repo's graph">
                  <Icon name="branch" size={12} />
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>

<style>
  .symbols {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .sym-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .sym-search {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 200px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    color: var(--text-dim);
  }
  .sym-search input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    outline: none;
  }
  .repo-filter {
    font-size: 12.5px;
    padding: 6px 9px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    max-width: 200px;
  }
  .count { font-size: 12px; color: var(--text-dim); }
  .sym-table-wrap {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  .empty { padding: 20px; font-size: 13px; color: var(--text-dim); }
  .sym-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  .sym-table thead th {
    position: sticky;
    top: 0;
    text-align: start;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    background: var(--bg-sidebar, var(--surface));
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    z-index: 1;
  }
  .sym-table tbody tr {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .sym-table tbody tr:hover { background: var(--surface-2); }
  .sym-table td { padding: 7px 12px; vertical-align: top; }
  .name { font-weight: 600; color: var(--text); font-family: var(--font-mono, monospace); }
  .kind {
    font-size: 10px;
    padding: 1px 7px;
    border-radius: 999px;
    /* Calm, themed chip — faint kind tint with readable, themed text. */
    border: 1px solid color-mix(in srgb, var(--c) 35%, var(--border));
    background: color-mix(in srgb, var(--c) 12%, transparent);
    color: var(--text);
    white-space: nowrap;
  }
  .lang { color: var(--text-dim); }
  .loc {
    font-family: var(--font-mono, monospace);
    font-size: 11.5px;
    color: var(--text-dim);
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sig {
    font-family: var(--font-mono, monospace);
    font-size: 11.5px;
    color: var(--text);
    max-width: 420px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions { width: 36px; }
  .mini {
    display: inline-flex;
    padding: 4px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .mini:hover { color: var(--text); }
</style>
