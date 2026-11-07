<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import type { ComposerContext } from './composerContext';

  let { context, onremove, onfocus }: {
    context: ComposerContext[];
    onremove: (id: string) => void;
    onfocus?: () => void;
  } = $props();
</script>

{#if context.length}
  <ul class="context-chips" aria-label="Message context" data-testid="runtime-context-chips">
    {#each context as item (item.id)}
      <li class="context-chip" title={item.text}>
        <Icon name={item.kind === 'diff' ? 'file' : 'comment'} size={12} />
        <span class="ellipsis">{item.label}</span>
        <button type="button" class="icon-btn" aria-label="Remove {item.label}" title="Remove context"
          onclick={() => { onremove(item.id); onfocus?.(); }}><Icon name="x" size={12} /></button>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .context-chips { display: flex; flex-wrap: wrap; gap: 4px; list-style: none; padding: 4px 0; margin: 0; max-block-size: 120px; overflow-y: auto; }
  .context-chip { display: flex; align-items: center; gap: 6px; max-inline-size: 100%; padding-inline-start: 8px; border: 1px solid var(--border); border-radius: var(--radius-m); font-size: var(--fs-s); color: var(--text-dim); }
  .context-chip .ellipsis { min-inline-size: 0; }
</style>
