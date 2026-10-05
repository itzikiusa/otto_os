<script lang="ts">
  // The Automate chooser outside empty states (S20-18): a header action that
  // collapses into the page's ⋯ first (lowest `data-overflow`) and opens the
  // guide expanded in a sheet.
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';
  import AutomateGuide, { type AutomateModule } from './AutomateGuide.svelte';

  let { current }: { current: AutomateModule } = $props();
  let open = $state(false);
</script>

<button
  type="button"
  class="icon-btn"
  data-overflow="-20"
  data-icon="info"
  data-label="Which automation should I use?"
  aria-label="Which automation should I use?"
  title="Which automation should I use?"
  data-testid="automate-guide-open"
  onclick={() => (open = true)}
>
  <Icon name="info" size={14} />
</button>

{#if open}
  <Modal title="Which automation should I use?" width={560} onclose={() => (open = false)}>
    <AutomateGuide {current} open onnavigate={() => (open = false)} />
  </Modal>
{/if}
