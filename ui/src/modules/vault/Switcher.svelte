<script lang="ts">
  // Quick switcher (⌘O inside the vault): server-side fuzzy over
  // title/aliases/path; Enter opens, Shift+Enter creates a note by that name.
  import type { VaultSwitchHit } from '../../lib/api/types';
  import { vault } from './vault.svelte';

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
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="overlay" onclick={close}>
    <div class="panel" role="dialog" tabindex="-1" aria-label="Quick switcher" onclick={(e) => e.stopPropagation()}>
      <input
        bind:this={input}
        bind:value={query}
        placeholder="Open note… (Shift+Enter creates)"
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
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 90;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
  }
  .panel {
    width: min(620px, 92vw);
    max-height: 60vh;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: 0 18px 50px rgba(0, 0, 0, 0.45);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  input {
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border);
    color: var(--text);
    font-size: 14px;
    padding: 12px 14px;
    outline: none;
  }
  .hits {
    overflow-y: auto;
    min-height: 0;
    padding: 6px;
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
  .hit.sel,
  .hit:hover {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
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
    font-size: 10.5px;
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
