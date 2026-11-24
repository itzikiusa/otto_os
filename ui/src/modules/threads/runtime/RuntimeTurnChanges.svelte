<script lang="ts">
  // Under a finished turn that changed files: "3 changed files +71 −2 ·
  // Open diff", opening to the file list with each file's +/− (turnChanges.ts).
  // Open diff opens the thread's Changes surface on that file (the first,
  // from the header). A file without diff text shows its name only. Open
  // by default up to five files, folded past that.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { changeTotals, type FileDelta } from './turnChanges';

  let { files, onopendiff }: { files: FileDelta[]; onopendiff?: (path: string) => void } = $props();
  // A few files show at once; more than five start folded.
  let open = $state(untrack(() => files.length <= 5));
  const total = $derived(changeTotals(files));
  const name = $derived(`${files.length} changed ${files.length === 1 ? 'file' : 'files'}`);
</script>

<section class="tc" aria-label={name} data-testid="runtime-turn-changes">
  <div class="tc-head">
    <button type="button" class="tc-toggle" aria-expanded={open} onclick={() => (open = !open)}>
      <Icon name="chevronRight" size={12} />
      <span class="tc-name">{name}</span>
      {#if total.added !== null}<span class="tc-add">+{total.added}</span><span class="tc-del">−{total.removed}</span>{/if}
    </button>
    {#if onopendiff}
      <button type="button" class="btn small ghost tc-open" data-testid="runtime-turn-changes-open" onclick={() => onopendiff(files[0].path)}>Open diff</button>
    {/if}
  </div>
  {#if open}
    <ul class="tc-files">
      {#each files as f (f.path)}
        <li>
          {#snippet row()}
            <span class="ellipsis mono" dir="ltr">{f.path}</span>
            {#if f.added !== null}<span class="tc-n"><span class="tc-add">+{f.added}</span> <span class="tc-del">−{f.removed}</span></span>{/if}
          {/snippet}
          {#if onopendiff}
            <button type="button" class="tc-file" title="Open {f.path} in Changes" onclick={() => onopendiff(f.path)}>{@render row()}</button>
          {:else}
            <span class="tc-file">{@render row()}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .tc {
    margin-block: 0 var(--sp-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .tc-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding-block: var(--sp-2);
    padding-inline: var(--sp-3) var(--sp-2);
  }
  .tc-toggle {
    display: inline-flex;
    flex: 1;
    align-items: center;
    gap: var(--sp-2);
    min-inline-size: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .tc-toggle[aria-expanded='true'] > :global(svg) {
    transform: rotate(90deg);
  }
  .tc-name {
    font-weight: 600;
  }
  .tc-add {
    color: var(--success);
  }
  .tc-del {
    color: var(--danger);
  }
  .tc-files {
    margin: 0;
    padding-block: 0 var(--sp-2);
    padding-inline: var(--sp-3);
    list-style: none;
    border-block-start: 1px solid var(--border);
  }
  .tc-files li {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-block-size: 28px;
  }
  .tc-n {
    flex-shrink: 0;
    margin-inline-start: auto;
  }
  .tc-file {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 1;
    min-inline-size: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    text-align: start;
  }
  button.tc-file {
    cursor: pointer;
  }
  button.tc-file:hover {
    color: var(--text);
  }
  .tc-toggle:focus-visible,
  button.tc-file:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  @media (pointer: coarse) {
    .tc-toggle,
    .tc-open,
    button.tc-file {
      min-block-size: calc(var(--hit-min) + 8px);
    }
  }
</style>
