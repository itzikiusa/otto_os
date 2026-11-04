<script lang="ts">
  // Settings and Usage as large sheets over Threads (T3's footer ⚙ / 📊):
  // Otto's own Settings page (in its sheet mode — its own section list, "/"
  // to search) and Usage page, each a lazy chunk, with Threads left in place.
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { SettingsView, UsageView } from './threadLazy.svelte';
  import { threadsChrome } from './threadsChrome.svelte';

  const close = (): void => {
    threadsChrome.sheet = null;
  };
</script>

{#if threadsChrome.sheet === 'settings'}
  <Modal title="Settings" width={1080} height={720} onclose={close}>
    <div class="ts-body" data-testid="threads-settings-sheet">
      <LazyMount lazy={SettingsView} what="settings" variant="panel" props={{ sheet: { onclose: close } }} />
    </div>
  </Modal>
{:else if threadsChrome.sheet === 'usage'}
  <Modal title="Usage" width={1080} height={720} onclose={close}>
    <div class="ts-body" data-testid="threads-usage-sheet">
      <LazyMount lazy={UsageView} what="usage" variant="panel" />
    </div>
  </Modal>
{/if}

<style>
  .ts-body {
    display: flex;
    flex-direction: column;
    block-size: min(640px, 72vh);
    min-height: 0;
    margin-block: -4px -16px;
    margin-inline: -16px;
  }
</style>
