<script lang="ts">
  // Commit search above the graph. ⌘/Ctrl+F focuses the box; typing runs
  // `log --all --grep=<q>` (or `--author=<q>`) on the daemon after a 250 ms
  // debounce, and clicking a hit asks the graph to select that commit through
  // `gitBridge` (no prop path from here into GraphView).
  //
  // The search is SERVER-backed rather than a filter of the rows the graph
  // happens to have loaded: the daemon is local, `--fixed-strings` matching is
  // cheap, and it finds commits outside the loaded page — "search all history"
  // with no second mode to toggle.
  import type { CommitInfo } from '../../lib/api/types';
  import { api, isAbortError } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { gitBridge } from './gitBridge.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
  }
  let { repoId }: Props = $props();

  let q = $state('');
  /** Search the AUTHOR instead of the subject. */
  let byAuthor = $state(false);
  let results = $state<CommitInfo[]>([]);
  let searching = $state(false);
  /** The query `results` belong to ('' = no active search). */
  let ran = $state('');
  let listOpen = $state(false);
  let inputEl: HTMLInputElement | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let inflight: AbortController | null = null;

  // Re-running a repo switch must not leave another repo's hits on screen.
  $effect(() => {
    void repoId;
    return () => clear();
  });

  function onWindowKeydown(e: KeyboardEvent): void {
    // Only while this bar is mounted — i.e. the graph is the visible tab — so
    // ⌘F keeps its browser meaning everywhere else.
    if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      inputEl?.focus();
      inputEl?.select();
    }
  }

  function clear(): void {
    if (timer) clearTimeout(timer);
    timer = null;
    inflight?.abort();
    inflight = null;
    q = '';
    ran = '';
    results = [];
    searching = false;
    listOpen = false;
  }

  function schedule(): void {
    if (timer) clearTimeout(timer);
    const term = q.trim();
    if (!term) {
      inflight?.abort();
      inflight = null;
      results = [];
      ran = '';
      searching = false;
      listOpen = false;
      return;
    }
    searching = true;
    listOpen = true;
    timer = setTimeout(() => void run(term), 250);
  }

  async function run(term: string): Promise<void> {
    inflight?.abort();
    const ac = new AbortController();
    inflight = ac;
    const field = byAuthor ? 'author' : 'grep';
    try {
      const rows = await api.get<CommitInfo[]>(
        `/repos/${repoId}/log?all=true&limit=200&${field}=${encodeURIComponent(term)}`,
        ac.signal,
      );
      if (ac !== inflight) return; // a newer query already won
      results = rows;
      ran = term;
    } catch (e) {
      if (isAbortError(e)) return;
      results = [];
      ran = term;
      toasts.error('Search failed', e instanceof Error ? e.message : String(e));
    } finally {
      if (ac === inflight) {
        searching = false;
        inflight = null;
      }
    }
  }

  function toggleAuthor(): void {
    byAuthor = !byAuthor;
    if (q.trim()) schedule();
  }

  function pick(c: CommitInfo): void {
    listOpen = false;
    gitBridge.focusCommit(repoId, c.sha);
  }

  function onInputKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (listOpen && results.length > 0) listOpen = false;
      else clear();
    } else if (e.key === 'Enter' && results.length > 0) {
      pick(results[0]);
    }
  }

  function short(date: string): string {
    const d = new Date(date);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div class="gsb">
  <div class="gsb-row">
    <div class="gsb-box">
      <Icon name="search" size={12} />
      <input
        class="gsb-input"
        bind:this={inputEl}
        bind:value={q}
        oninput={schedule}
        onkeydown={onInputKeydown}
        onfocus={() => (listOpen = results.length > 0)}
        placeholder="Search commits — ⌘F"
        aria-label="Search commits"
        spellcheck="false"
      />
      <button
        class="gsb-chip"
        class:on={byAuthor}
        onclick={toggleAuthor}
        title="Match the author instead of the subject"
      >
        Author
      </button>
      {#if q}
        <button class="gsb-x" onclick={clear} aria-label="Clear search">
          <Icon name="x" size={11} />
        </button>
      {/if}
    </div>

    {#if ran}
      <div class="gsb-banner">
        {#if searching}
          Searching…
        {:else}
          {results.length} match{results.length === 1 ? '' : 'es'} for "{ran}"
        {/if}
        <button class="gsb-link" onclick={clear}>Clear</button>
      </div>
    {/if}
  </div>

  {#if listOpen && results.length > 0}
    <!-- Anchored to the input (not free coordinates) and height-capped, so a
         200-row result set stays scrollable inside the viewport. -->
    <ul class="gsb-results">
      {#each results as c (c.sha)}
        <li>
          <button class="gsb-result" onclick={() => pick(c)}>
            <span class="mono gsb-sha">{c.short_sha}</span>
            <span class="gsb-subject" title={c.subject}>{c.subject}</span>
            <span class="gsb-author">{c.author}</span>
            <span class="gsb-date">{short(c.date)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .gsb {
    position: relative;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .gsb-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .gsb-box {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 180px;
    max-width: 460px;
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .gsb-box:focus-within {
    border-color: var(--accent);
  }
  .gsb-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    padding: 2px 0;
    outline: none;
  }
  .gsb-chip {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 999px;
    cursor: pointer;
  }
  .gsb-chip.on {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .gsb-x {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    display: inline-flex;
    padding: 1px;
  }
  .gsb-banner {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .gsb-link {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 11.5px;
    cursor: pointer;
    padding: 0;
  }
  .gsb-results {
    position: absolute;
    z-index: 30;
    top: calc(100% - 4px);
    left: 10px;
    right: 10px;
    max-width: 640px;
    margin: 0;
    padding: 4px;
    list-style: none;
    max-height: 50vh;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    box-shadow: var(--shadow);
  }
  .gsb-result {
    display: grid;
    grid-template-columns: 72px 1fr auto auto;
    gap: 8px;
    align-items: center;
    width: 100%;
    padding: 4px 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: 12px;
    text-align: start;
    cursor: pointer;
  }
  .gsb-result:hover {
    background: var(--surface-2);
  }
  .gsb-sha {
    color: var(--text-dim);
    font-size: 11px;
  }
  .gsb-subject {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .gsb-author,
  .gsb-date {
    color: var(--text-dim);
    font-size: 11px;
    white-space: nowrap;
  }
  @media (max-width: 720px) {
    .gsb-result {
      grid-template-columns: 60px 1fr;
    }
    .gsb-author,
    .gsb-date {
      display: none;
    }
  }
</style>
