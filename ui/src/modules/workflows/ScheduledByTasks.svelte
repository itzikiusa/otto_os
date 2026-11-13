<script lang="ts">
  // "Also started by scheduled task X (Every day)": the other way a workflow is
  // scheduled (a Scheduled task of kind `workflow`), shown under the workflow's
  // own triggers so "why did it run at 9?" has an answer here. A secondary
  // hint: nothing while loading or when there are none, and a failed load is
  // LoadState's one compact line with Retry, never a toast.
  import { scheduledTasksApi } from '../../lib/api/scheduledTasks';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { ScheduledTask } from '../../lib/api/types';
  import { canSeeScheduledBy, scheduledByTasks } from './scheduledBy';

  let { workflowId }: { workflowId: string } = $props();

  const allowed = $derived(canSeeScheduledBy((f, c) => auth.can(f, c)));
  let tasks = $state<ScheduledTask[]>([]);
  let failed = $state<string | null>(null);
  let seq = 0;

  async function load(wsId: string): Promise<void> {
    const mine = ++seq;
    failed = null;
    try {
      const list = await scheduledTasksApi.list(wsId);
      if (mine === seq) tasks = list;
    } catch (e) {
      if (mine === seq) { tasks = []; failed = loadErrorText(e); }
    }
  }

  $effect(() => {
    const wsId = ws.currentId;
    void workflowId; // refetch when the inspector switches workflow
    if (!allowed || !wsId) { tasks = []; failed = null; return; }
    void load(wsId);
  });

  const rows = $derived.by(() => scheduledByTasks(tasks, workflowId));
</script>

{#if allowed}
  <LoadState
    what="the scheduled tasks that start this workflow"
    variant="compact"
    error={failed}
    empty={rows.length === 0}
    onretry={() => ws.currentId && void load(ws.currentId)}
  >
    <ul class="sb" data-testid="scheduled-by">
      {#each rows as r (r.id)}
        <li class="sb-line">
          Also started by scheduled task
          <button type="button" class="sb-link" onclick={() => router.go(r.route)}><strong>{r.name}</strong></button>
          <span class="sb-muted">({r.cadence})</span>
        </li>
      {/each}
    </ul>
  </LoadState>
{/if}

<style>
  .sb {
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .sb-line {
    margin: 0;
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .sb-muted { color: var(--text-dim); }
  .sb-link {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .sb-link:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: var(--radius-s); }
</style>
