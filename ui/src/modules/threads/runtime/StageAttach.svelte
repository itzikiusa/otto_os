<script lang="ts">
  // The new-thread stage's attachments: `picker` is the hidden image / PDF
  // input the composer's one attach menu opens (`open()`), `list` previews
  // them (they go with a runtime thread's first message).
  import RuntimeComposerImages from './RuntimeComposerImages.svelte';
  import type { StageAttachments } from './stageAttachments.svelte';

  let { part, files }: { part: 'picker' | 'list'; files: StageAttachments } = $props();
  let input = $state<HTMLInputElement | null>(null);

  /** Open the system file chooser (images, and PDFs where the agent takes them). */
  export function open(): void {
    input?.click();
  }
</script>

{#if part === 'list'}
  {#if files.list.length}<RuntimeComposerImages images={files.list} onremove={(i) => files.remove(files.list[i])} />{/if}
{:else if files.can.images}
  <input
    bind:this={input}
    type="file"
    accept={files.can.files ? 'image/*,application/pdf' : 'image/*'}
    multiple
    hidden
    data-testid="new-thread-attach-input"
    onchange={(e) => {
      void files.add([...(e.currentTarget.files ?? [])]);
      e.currentTarget.value = '';
    }}
  />
{/if}
