<!-- The reset dry-run's refusal warning (api-contracts-5-02): shown above the
     preview while the real reset would be refused (live consumers…). -->
<script module lang="ts">
  import { resetWarning } from './resetWarning';
  export { resetWarning };
</script>

<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import type { DryRunResp } from './types';

  let { preview, group }: { preview: DryRunResp | null; group: string | null } = $props();
  const text = $derived(resetWarning(preview, group));
</script>

{#if text}
  <p class="reset-warning" role="status">
    <Icon name="warning" size={12} />
    <span>{text}</span>
  </p>
{/if}

<style>
  .reset-warning {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin-block: 0 8px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--fs-s);
  }
</style>
