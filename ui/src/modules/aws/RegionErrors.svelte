<script lang="ts">
  // Inline note for an "All enabled regions" list: the regions that failed
  // (denied, opt-in disabled, throttled…) while the rest still rendered.
  import Icon from '../../lib/components/Icon.svelte';
  import type { AwsRegionError } from '../../lib/api/types';

  interface Props {
    errors: AwsRegionError[];
  }
  let { errors }: Props = $props();
  let open = $state(false);
</script>

{#if errors.length}
  <div class="rerr" role="status">
    <Icon name="warning" size={13} />
    <span>{errors.length} region{errors.length === 1 ? '' : 's'} could not be listed — the rows below come from the others.</span>
    <button class="link" onclick={() => (open = !open)} aria-expanded={open}>{open ? 'Hide' : 'Details'}</button>
    {#if open}
      <ul>
        {#each errors as e (e.region)}
          <li><span class="mono">{e.region}</span> — {e.message}</li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .rerr {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 8px 12px;
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .rerr :global(svg) {
    color: var(--warning);
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent-text);
    cursor: pointer;
    font: inherit;
  }
  ul {
    flex-basis: 100%;
    margin: 4px 0 0;
    padding-inline-start: 18px;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
