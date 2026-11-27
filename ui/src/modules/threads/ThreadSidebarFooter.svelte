<script lang="ts">
  // The Threads list pane's footer, shown only when it has something to say
  // (decision 13; an element-less footer is hidden, app.css): the usage meter
  // while there are numbers for the focused thread's agent, or "Back" while a
  // stage view (Pull requests) is open. Usage, Settings, Pull requests and
  // Refresh live in the pane's ⋯.
  import Icon from '../../lib/components/Icon.svelte';
  import { UsageMeterView } from './threadLazy.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
</script>

{#if threadsChrome.stageView}
  <button type="button" class="btn small ghost" data-testid="threads-stage-back" onclick={() => (threadsChrome.stageView = null)}>
    <Icon name="chevronLeft" size={12} /> Back
  </button>
{:else if UsageMeterView.component}
  <UsageMeterView.component quiet />
{/if}
