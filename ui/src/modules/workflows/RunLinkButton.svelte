<script lang="ts">
  // "Copy link" in the run header: the run's deep link
  // (`#/workflows/<workflow id>/runs/<run id>`) on the Otto this page is
  // served from — paste it into a thread or a note to come back to this run.
  import Icon from '../../lib/components/Icon.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { runDeepLink } from './runRoute';

  let { workflowId, runId }: { workflowId: string; runId: string } = $props();
</script>

<button
  class="icon-btn"
  data-testid="run-copy-link"
  aria-label="Copy link to this run"
  title="Copy link to this run"
  onclick={() => void copyWithToast(runDeepLink(window.location, workflowId, runId), 'Run link')}
>
  <Icon name="link" size={13} />
</button>
