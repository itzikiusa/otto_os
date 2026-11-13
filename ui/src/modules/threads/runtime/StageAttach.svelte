<script lang="ts">
  // The new-thread stage's attachments: `button` picks images / PDFs,
  // `list` previews them (they go with a runtime thread's first message).
  import Icon from '../../../lib/components/Icon.svelte';
  import RuntimeComposerImages from './RuntimeComposerImages.svelte';
  import type { StageAttachments } from './stageAttachments.svelte';

  let { part, files, disabled = false }: { part: 'button' | 'list'; files: StageAttachments; disabled?: boolean } = $props();
  let input = $state<HTMLInputElement | null>(null);
</script>

{#if part === 'list'}
  {#if files.list.length}<RuntimeComposerImages images={files.list} onremove={(i) => files.remove(files.list[i])} />{/if}
{:else if files.can.images}
  <button
    type="button"
    class="icon-btn"
    aria-label={files.can.files ? 'Attach an image or PDF' : 'Attach an image'}
    title={files.can.files ? 'Attach an image or PDF (or paste one)' : 'Attach an image (or paste one)'}
    data-testid="new-thread-attach-image"
    {disabled}
    onclick={() => input?.click()}
  >
    <Icon name="image" size={16} />
  </button>
  <input
    bind:this={input}
    type="file"
    accept={files.can.files ? 'image/*,application/pdf' : 'image/*'}
    multiple
    hidden
    onchange={(e) => {
      void files.add([...(e.currentTarget.files ?? [])]);
      e.currentTarget.value = '';
    }}
  />
{/if}
