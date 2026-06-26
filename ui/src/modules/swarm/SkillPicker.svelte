<script lang="ts">
  // A reusable multi-select for library skill names — used for team skills
  // (swarm config) and project skills. Loads the library on mount; renders the
  // current selection as removable chips with an autocompleting add input.
  import { swarm } from '../../lib/stores/swarm.svelte';

  interface Props {
    /** Currently-selected skill names. */
    selected: string[];
    /** Called with the next selection whenever it changes. */
    onchange: (next: string[]) => void;
    label?: string;
  }
  let { selected, onchange, label }: Props = $props();

  let draft = $state('');

  $effect(() => {
    void swarm.loadLibrarySkills();
  });

  // Suggestions = library skills not already selected, matching the draft.
  const suggestions = $derived(
    swarm.librarySkills
      .filter((s) => !selected.includes(s.name))
      .filter((s) => !draft.trim() || s.name.toLowerCase().includes(draft.trim().toLowerCase())),
  );

  function add(name: string) {
    const n = name.trim();
    if (!n || selected.includes(n)) return;
    onchange([...selected, n]);
    draft = '';
  }

  function remove(name: string) {
    onchange(selected.filter((s) => s !== name));
  }

  const listId = `skill-list-${Math.random().toString(36).slice(2, 8)}`;
</script>

<div class="picker">
  {#if label}<span class="lbl">{label}</span>{/if}
  <div class="add">
    <input
      class="input grow"
      placeholder="Add a skill…"
      bind:value={draft}
      list={listId}
      onkeydown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          add(draft);
        }
      }}
    />
    <datalist id={listId}>
      {#each suggestions as s (s.name)}
        <option value={s.name}>{s.description}</option>
      {/each}
    </datalist>
    <button class="btn small" onclick={() => add(draft)} disabled={!draft.trim()}>Add</button>
  </div>

  {#if selected.length}
    <div class="chips">
      {#each selected as s (s)}
        <span class="chip">
          {s}
          <button class="x" onclick={() => remove(s)} aria-label="Remove {s}">×</button>
        </span>
      {/each}
    </div>
  {:else}
    <p class="dim none">No skills selected.</p>
  {/if}
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lbl {
    font-size: 12px;
    color: var(--text-dim);
  }
  .add {
    display: flex;
    gap: 6px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 2px 8px;
    font-size: 11.5px;
  }
  .x {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    font-size: 13px;
    line-height: 1;
  }
  .x:hover {
    color: var(--status-exited);
  }
  .none {
    font-size: 11.5px;
    margin: 0;
  }
</style>
