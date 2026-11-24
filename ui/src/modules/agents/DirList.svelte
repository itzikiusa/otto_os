<script lang="ts">
  // The "additional directories" editor (meta.extra_dirs → `--add-dir`): the
  // chosen folders, each removable, and a field to add one (Enter or Add).
  // Shared by New session (Options) and the pane's ⋯ › Additional directories.
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    dirs: string[];
    draft: string;
    inputId: string;
    /** Wraps the input (a PathField); the bare input when omitted. */
    field?: Snippet<[Snippet]>;
    /** A Browse… button before Add. */
    onbrowse?: () => void;
  }
  let { dirs = $bindable(), draft = $bindable(), inputId, field, onbrowse }: Props = $props();

  function add(): void {
    const path = draft.trim();
    if (path !== '' && !dirs.includes(path)) dirs = [...dirs, path];
    draft = '';
  }
</script>

{#if dirs.length > 0}
  <ul class="dir-list">
    {#each dirs as dir (dir)}
      <li class="dir-row">
        <span class="dir-path mono" dir="ltr" title={dir}>{dir}</span>
        <button type="button" class="dir-remove" title="Remove directory" aria-label="Remove {dir}" onclick={() => (dirs = dirs.filter((d) => d !== dir))}><Icon name="x" size={12} /></button>
      </li>
    {/each}
  </ul>
{/if}
{#snippet input()}
  <input dir="ltr" id={inputId} aria-label="Add a folder" class="input mono" bind:value={draft} spellcheck="false" placeholder="/absolute/path/to/repo" onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); add(); } }} />
{/snippet}
<div class="dir-add">
  {#if field}{@render field(input)}{:else}{@render input()}{/if}
  {#if onbrowse}<button type="button" class="btn" onclick={onbrowse}>Browse…</button>{/if}
  <button type="button" class="btn" disabled={draft.trim() === ''} onclick={add}>Add</button>
</div>

<style>
  .dir-list {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dir-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .dir-path {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .dir-remove {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 2px 4px;
    border-radius: var(--radius-s);
    line-height: 1;
  }
  .dir-remove:hover {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .dir-add {
    display: flex;
    gap: 6px;
  }
  .dir-add :global(.input) {
    flex: 1;
    min-width: 0;
  }
</style>
