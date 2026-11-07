<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import type { ComposerContext } from './composerContext';

  let { context, onremove, onfocus }: {
    context: ComposerContext[];
    onremove: (id: string) => void;
    onfocus?: () => void;
  } = $props();
  let selectedId = $state<string | null>(null);
  const selected = $derived(context.find((item) => item.id === selectedId));
</script>

{#if context.length}
  <ul class="context-chips" aria-label="Message context" data-testid="runtime-context-chips">
    {#each context as item (item.id)}
      <li class="context-chip" title={item.text}>
        <button type="button" class="btn ghost context-open" aria-label="View context {item.label}" title="View context"
          onclick={() => (selectedId = item.id)}>
          <Icon name={item.kind === 'diff' ? 'file' : 'comment'} size={12} />
          <span class="ellipsis">{item.label}</span>
        </button>
        <button type="button" class="icon-btn context-remove" aria-label="Remove {item.label}" title="Remove context"
          onclick={() => { onremove(item.id); onfocus?.(); }}><Icon name="x" size={12} /></button>
      </li>
    {/each}
  </ul>
{/if}

{#if selected}
  <Modal title="Message context" width={640} onclose={() => (selectedId = null)}>
    <p class="context-label" dir="auto">{selected.label}</p>
    <pre class="context-preview" dir="auto">{selected.text}</pre>
  </Modal>
{/if}

<style>
  .context-chips { display: flex; flex-wrap: wrap; gap: 4px; list-style: none; padding: 4px 0; margin: 0; max-block-size: 120px; overflow-y: auto; }
  .context-chip { display: flex; align-items: center; gap: 2px; max-inline-size: 100%; border: 1px solid var(--border); border-radius: var(--radius-m); font-size: var(--fs-s); color: var(--text-dim); }
  .context-chip .ellipsis { min-inline-size: 0; }
  .context-open { min-inline-size: 0; max-inline-size: 100%; }
  .context-label { margin-block: 0 12px; font-size: var(--fs-s); color: var(--text-dim); overflow-wrap: anywhere; }
  .context-preview { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-family: var(--font-mono); font-size: var(--fs-s); line-height: 1.6; }
  @media (pointer: coarse) { .context-open, .context-remove { min-block-size: 44px; } }
</style>
