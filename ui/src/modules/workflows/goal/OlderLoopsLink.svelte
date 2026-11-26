<script lang="ts">
  // The Runs tab's foot (goal.md §5.2): "Older goal loops (n)", only when the
  // workspace has loops from before goals were folded into Workflows.
  import { api } from '../../../lib/api/client';
  import type { GoalLoop } from '../../../lib/api/types';
  import { router } from '../../../lib/router.svelte';

  let { workspaceId }: { workspaceId: string } = $props();
  let count = $state(0);

  $effect(() => {
    const w = workspaceId;
    count = 0;
    api.get<GoalLoop[]>(`/workspaces/${encodeURIComponent(w)}/goal-loops`).then((rows) => { if (w === workspaceId) count = rows.length; }, () => {});
  });
</script>

{#if count > 0}
  <button type="button" class="older" data-testid="wf-older-loops" onclick={() => router.go('loops')}>Older goal loops ({count})</button>
{/if}

<style>
  .older {
    align-self: center;
    padding: var(--sp-2) var(--sp-4);
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .older:hover {
    text-decoration: underline;
  }
</style>
