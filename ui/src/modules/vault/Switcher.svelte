<script lang="ts">
  // Quick switcher (⌘O inside the vault): server-side fuzzy over
  // title/aliases/path; Enter opens, Shift+Enter creates a note by that name.
  import type { VaultSwitchHit } from '../../lib/api/types';
  import { vault } from './vault.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  let query = $state('');
  let hits = $state<VaultSwitchHit[]>([]);
  let sel = $state(0);
  let input = $state<HTMLInputElement | undefined>();
  let seq = 0;
  let loading = $state(false);
  let lookupError = $state('');
  let resolvedQuery = $state<string | null>(null);
  const canCreate = $derived(!loading && !lookupError && resolvedQuery === query && hits.length === 0 && !!query.trim());

  $effect(() => {
    if (vault.switcherOpen) {
      query = '';
      hits = [];
      sel = 0;
      void refresh('');
      requestAnimationFrame(() => input?.focus());
    }
  });

  async function refresh(q: string): Promise<void> {
    const my = ++seq;
    const id = vault.current?.id, workspace = vault.wsId, generation = vault.lookupGeneration;
    const current = () => my === seq && query === q && vault.switcherOpen && vault.current?.id === id && vault.wsId === workspace && vault.lookupGeneration === generation;
    loading = true; lookupError = ''; resolvedQuery = null; hits = []; sel = 0;
    try {
      const got = await vault.switcherQuery(q);
      if (current()) { hits = got; sel = 0; resolvedQuery = q; }
    } catch (e) {
      if (current()) lookupError = e instanceof Error ? e.message : String(e);
    } finally {
      if (current()) loading = false;
    }
  }

  function close(): void {
    vault.switcherOpen = false;
  }

  function pick(h: VaultSwitchHit | undefined): void {
    if (!h) return;
    close();
    void vault.open(h.path);
  }

  function createFromQuery(explicit = false): void {
    const name = query.trim();
    if (!name) return;
    if (!explicit && (loading || lookupError || resolvedQuery !== query || hits.length > 0)) return;
    close();
    void vault.createNote(name.endsWith('.md') ? name : `${name}.md`, `# ${name}\n\n`);
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      sel = Math.max(0, Math.min(sel + 1, hits.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      sel = Math.max(sel - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (e.shiftKey) createFromQuery(true);
      else if (loading || lookupError || resolvedQuery !== query) return;
      else if (hits.length === 0) createFromQuery();
      else pick(hits[sel]);
    }
  }
</script>

{#if vault.switcherOpen}
  <Modal title="Quick switcher" width={620} onclose={close}>
    <div class="vs-body">
      <input
        bind:this={input}
        bind:value={query}
        class="vs-input"
        placeholder="Open note… (Shift+Enter creates)"
        aria-label="Open note"
        role="combobox"
        aria-autocomplete="list"
        aria-controls="vs-list"
        aria-expanded={hits.length > 0 || canCreate}
        aria-activedescendant={hits.length > 0 ? `vs-opt-${sel}` : canCreate ? 'vs-opt-create' : undefined}
        oninput={() => void refresh(query)}
        onkeydown={onKey}
      />
      {#if loading}
        <div role="status">Searching notes…</div>
      {:else if lookupError}
        <div role="alert">Couldn’t search notes. {lookupError}</div>
        <button class="btn small" onclick={() => void refresh(query)}>Retry</button>
      {/if}
      <!-- Combobox + listbox: focus stays in the input, ↑/↓ move the active
           option (aria-activedescendant); options are clickable too. -->
      <div class="hits" id="vs-list" role="listbox" aria-label="Matching notes">
        {#each hits.slice(0, 30) as h, i (h.path + (h.alias ?? ''))}
          <button class="hit" class:sel={i === sel} id="vs-opt-{i}" role="option" aria-selected={i === sel} tabindex="-1" onclick={() => pick(h)}>
            <span class="t">{h.alias ?? h.title}</span>
            {#if h.alias}<span class="via">→ {h.title}</span>{/if}
            <span class="p" title={h.path}>{h.path}</span>
          </button>
        {/each}
        {#if canCreate}
          <button class="hit create-btn sel" id="vs-opt-create" role="option" aria-selected="true" tabindex="-1" onclick={() => createFromQuery()}>
            <span class="t">Create “{query.trim()}”</span>
            <span class="p">New note · Enter</span>
          </button>
        {/if}
      </div>
    </div>
  </Modal>
{/if}

<style>
  .vs-body {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .vs-input {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-l);
    padding: 8px 10px;
    outline: none;
  }
  .vs-input:focus {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .hits {
    padding: 6px 0 0;
  }
  .hit {
    display: flex;
    gap: 8px;
    align-items: baseline;
    width: 100%;
    text-align: start;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    padding: 6px 10px;
    cursor: pointer;
    color: var(--text);
  }
  .hit.sel {
    background: var(--accent-soft);
  }
  .hit:hover:not(.sel) {
    background: var(--hover);
  }
  .t {
    font-size: var(--fs-m);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .via {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .p {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 45%;
  }
  .create-btn .t {
    color: var(--accent-text);
  }
</style>
