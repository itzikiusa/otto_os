<script lang="ts">
  // The Goal form's one mount point (goal.md §3.3), mounted once on the
  // Workflows page. It loads the form on first open, registers ⌘K "New
  // goal…", and opens the form filled from a staged draft (Insights, the
  // Automate guide, an older loop's Continue). Nothing starts until Start goal.
  import { onMount } from 'svelte';
  import { api } from '../../../lib/api/client';
  import type { GoalLoopDetail, Repo } from '../../../lib/api/types';
  import { toastError } from '../../../lib/toastError';
  import { registry } from '../../../lib/commands.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { takeStagedGoalDraft } from './goalDraft';
  import { fromLoop, fromStaged } from './goalForm';
  import { goalLauncher } from './goalLauncher.svelte';

  const GoalForm = lazyComponent(() => import('./GoalForm.svelte'));

  let { workspaceId }: { workspaceId: string } = $props();

  onMount(() => {
    const draft = takeStagedGoalDraft();
    if (draft?.loopId) void continueLoop(draft.loopId);
    // A blank draft (the Automate guide's Goal row) opens an empty form.
    else if (draft) goalLauncher.open(draft.prompt ? fromStaged(draft) : undefined);
    return () => goalLauncher.close();
  });

  /** An older loop's "Continue as a goal": the form filled from the loop. */
  async function continueLoop(id: string): Promise<void> {
    try {
      const [detail, repos] = await Promise.all([
        api.get<GoalLoopDetail>(`/goal-loops/${encodeURIComponent(id)}?summary=true`),
        api.get<Repo[]>(`/workspaces/${encodeURIComponent(workspaceId)}/repos`).catch(() => [] as Repo[]),
      ]);
      goalLauncher.open(fromLoop(detail.loop, repos));
    } catch (e) {
      toastError('Couldn’t open the loop to continue', e);
    }
  }

  $effect(() =>
    registry.register('goal', [
      { id: 'workflows.goal', title: 'New goal…', group: 'Workflows', keywords: 'goal loop objective criteria iterate until done budget', run: () => goalLauncher.open() },
    ]),
  );

  $effect(() => {
    if (goalLauncher.isOpen && GoalForm.error) {
      toasts.error('Couldn’t open the goal form', GoalForm.error);
      goalLauncher.close();
      GoalForm.retry();
    }
  });
</script>

{#if goalLauncher.isOpen && GoalForm.component}
  {#key goalLauncher.generation}
    <GoalForm.component {workspaceId} initial={goalLauncher.fields} onclose={() => goalLauncher.close()} />
  {/key}
{/if}
