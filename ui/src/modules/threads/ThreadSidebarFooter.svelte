<script lang="ts">
  // The Threads list pane's footer: the usage meter (or its Usage button until
  // the meter's chunk is in); "Back" while a stage view (Pull requests) is
  // open. Settings, Pull requests and Refresh live in the pane's ⋯.
  import Icon from '../../lib/components/Icon.svelte';
  import { UsageMeterView, UsageView } from './threadLazy.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
</script>

{#if threadsChrome.stageView}
  <button type="button" class="btn small ghost" data-testid="threads-stage-back" onclick={() => (threadsChrome.stageView = null)}>
    <Icon name="chevronLeft" size={12} /> Back
  </button>
{:else if UsageMeterView.component}
  <UsageMeterView.component />
{:else}
  <button
    type="button"
    class="icon-btn"
    aria-label="Usage statistics"
    title="Usage statistics"
    data-testid="threads-open-usage"
    onpointerenter={() => UsageView.prefetch()}
    onclick={() => (threadsChrome.sheet = 'usage')}
  >
    <Icon name="chart" size={14} />
  </button>
{/if}
