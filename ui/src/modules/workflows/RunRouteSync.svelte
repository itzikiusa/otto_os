<script lang="ts">
  // Follows a run deep link: `#/workflows/<workflow id>/runs/<run id>` (the
  // run header's "Copy link") opens that workflow and that run, once per
  // link. Opening another run afterwards drops the stale run part from the
  // URL, so a reload never jumps back to it. Renders nothing; mounted at the
  // page root so the phone's list ↔ editor switch never re-follows the link.
  import { untrack } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { parseRunRoute } from './runRoute';

  interface Props {
    /** The workflow and the run on screen. */
    currentWorkflowId: string | null;
    openRunId: string | null;
    onopen: (workflowId: string, runId: string) => void;
  }
  let { currentWorkflowId, openRunId, onopen }: Props = $props();

  let followed: string | null = null;
  let reached = false;
  let attempts = 0;
  $effect(() => {
    const target = parseRunRoute(router.parts);
    if (!target) {
      followed = null;
      return;
    }
    const key = `${target.workflowId}/${target.runId}`;
    if (followed === key) return;
    followed = key;
    reached = false;
    attempts = 1;
    untrack(() => onopen(target.workflowId, target.runId));
  });
  $effect(() => {
    const open = openRunId;
    const wf = currentWorkflowId;
    untrack(() => {
      const target = parseRunRoute(router.parts);
      if (!target) return;
      if (open === target.runId) reached = true;
      else if (reached) router.replace(`workflows/${target.workflowId}`);
      // A cold load's own workflow pick (the remembered / linked workflow,
      // opened once the list lands) can supersede the first open before the
      // run is shown — ask once more when that workflow is on screen.
      else if (wf === target.workflowId && attempts < 2) {
        attempts += 1;
        onopen(target.workflowId, target.runId);
      }
    });
  });
</script>
