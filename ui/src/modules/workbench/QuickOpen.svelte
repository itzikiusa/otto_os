<script lang="ts">
  // ⌘P quick open: fuzzy (subsequence) filter over the workbench's files.
  import Modal from '../../lib/components/Modal.svelte';
  import { workbench, sortDocs } from './workbench.svelte';
  import type { WorkbenchDoc } from '../../lib/api/types';

  interface Props {
    onclose: () => void;
    onopen: (id: string) => void;
  }
  let { onclose, onopen }: Props = $props();

  let query = $state('');
  let index = $state(0);
  let input: HTMLInputElement | undefined = $state();

  /** Subsequence score: lower is better; -1 = no match. Contiguous runs and
   *  matches at the start of the name rank first. */
  function score(name: string, q: string): number {
    if (!q) return 0;
    const n = name.toLowerCase();
    const direct = n.indexOf(q);
    if (direct >= 0) return direct;
    let pos = -1;
    let gaps = 0;
    for (const ch of q) {
      const next = n.indexOf(ch, pos + 1);
      if (next < 0) return -1;
      if (pos >= 0) gaps += next - pos - 1;
      pos = next;
    }
    return 100 + gaps;
  }

  const results = $derived.by((): WorkbenchDoc[] => {
    const q = query.trim().toLowerCase();
    const all = sortDocs(workbench.docs);
    if (!q) return all.slice(0, 50);
    return all
      .map((d) => ({ d, s: score(d.name, q) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => a.s - b.s)
      .slice(0, 50)
      .map((x) => x.d);
  });

  $effect(() => {
    void query;
    index = 0;
  });

  $effect(() => {
    input?.focus();
  });

  // Keep the active option visible as the arrows move it through a long list.
  $effect(() => {
    document.getElementById(`wb-qo-${index}`)?.scrollIntoView({ block: 'nearest' });
  });

  function pick(d: WorkbenchDoc | undefined): void {
    if (!d) return;
    onopen(d.id);
    onclose();
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      index = Math.min(index + 1, results.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      index = Math.max(index - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      pick(results[index]);
    }
  }
</script>

<Modal title="Open file" {onclose}>
  <!-- Combobox + listbox, as the ⌘K palette: focus stays in the input and
       aria-activedescendant names the arrowed option, so a screen reader
       hears the selection move. Options are kept out of the tab order. -->
  <input dir="ltr"
    bind:this={input}
    class="wb-qo-input"
    type="search"
    placeholder="e.g. deploy.sh"
    aria-label="File name"
    role="combobox"
    aria-expanded="true"
    aria-autocomplete="list"
    aria-controls="wb-qo-list"
    aria-activedescendant={results.length > 0 ? `wb-qo-${index}` : undefined}
    bind:value={query}
    onkeydown={onKey}
    data-testid="wb-quickopen-input"
  />
  <div class="wb-qo-list" id="wb-qo-list" role="listbox" aria-label="Files">
    {#each results as d, i (d.id)}
      <button
        id="wb-qo-{i}"
        tabindex="-1"
        role="option"
        aria-selected={i === index}
        class="wb-qo-row"
        class:sel={i === index}
        onmousedown={(e) => e.preventDefault()}
        onclick={() => pick(d)}
        onmouseenter={() => (index = i)}
      >
        <span class="wb-qo-name">{d.name}</span>
        <span class="wb-qo-lang">{d.language === 'auto' ? '' : d.language}</span>
      </button>
    {:else}
      <div class="wb-qo-empty">No files match.</div>
    {/each}
  </div>
</Modal>

<style>
  .wb-qo-input {
    width: 100%;
    box-sizing: border-box;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-m);
  }
  .wb-qo-list {
    margin: 8px 0 0;
    padding: 0;
    max-height: min(50vh, 360px);
    overflow-y: auto;
  }
  .wb-qo-row {
    display: flex;
    width: 100%;
    gap: 8px;
    align-items: center;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .wb-qo-row.sel {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .wb-qo-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wb-qo-lang {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .wb-qo-empty {
    padding: 12px 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
