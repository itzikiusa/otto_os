<script lang="ts">
  // The composer's attach button and its hidden file chooser (images, and
  // PDFs where the agent takes them). The files go through the same intake
  // as paste (runtimeComposer.ts appendDraftAttachments).
  import Icon from '../../../lib/components/Icon.svelte';

  interface Props {
    canFiles: boolean;
    testId: string;
    disabled: boolean;
    onfiles: (files: FileList) => void;
  }
  let { canFiles, testId, disabled, onfiles }: Props = $props();
  let fileEl = $state<HTMLInputElement | null>(null);
</script>

<button
  type="button"
  class="icon-btn"
  aria-label={canFiles ? 'Attach an image or PDF' : 'Attach an image'}
  title={canFiles ? 'Attach an image or PDF (or paste one)' : 'Attach an image (or paste one)'}
  data-testid={testId}
  {disabled}
  onclick={() => fileEl?.click()}
>
  <Icon name="image" size={14} />
</button>
<input
  bind:this={fileEl}
  type="file"
  accept={canFiles ? 'image/*,application/pdf' : 'image/*'}
  multiple
  hidden
  onchange={(e) => {
    if (e.currentTarget.files) onfiles(e.currentTarget.files);
    e.currentTarget.value = '';
  }}
/>
