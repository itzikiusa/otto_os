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
    const got = await vault.switcherQuery(q);
    if (my === seq) {
      hits = got;
      sel = 0;
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

  function createFromQuery(): void {
    const name = query.trim();
    if (!name) return;
    close();
    void vault.createNote(name.endsWith('.md') ? name : `${name}.md`, `# ${name}\n\n`);
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      sel = Math.min(sel + 1, hits.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      sel = Math.max(sel - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (e.shiftKey || hits.length === 0) createFromQuery();
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
        oninput={() => void refresh(query)}
        onkeydown={onKey}
      />
      <div class="hits">
        {#each hits.slice(0, 30) as h, i (h.path + (h.alias ?? ''))}
          <button class="hit" class:sel={i === sel} onclick={() => pick(h)}>
            <span class="t">{h.alias ?? h.title}</span>
            {#if h.alias}<span class="via">→ {h.title}</span>{/if}
            <span class="p">{h.path}</span>
          </button>
        {/each}
        {#if hits.length === 0 && query.trim()}
          <div class="create">↵ Create “{query.trim()}”</div>
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
    border-color: var(--accent);
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
    border-radius: 7px;
    padding: 7px 10px;
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
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .via {
    font-size: 11px;
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
  .create {
    padding: 10px 12px;
    font-size: 12.5px;
    color: var(--text-dim);
  }
</style>
