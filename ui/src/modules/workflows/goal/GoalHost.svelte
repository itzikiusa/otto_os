<script lang="ts">
  // The Goal form's one mount point (goal.md §3.3): mounted once on the
  // Workflows page (and on the older-loops page for Continue as a goal). It
  // loads the form on first open, registers ⌘K "New goal…", and opens the form
  // filled when Insights staged a goal draft. Nothing starts until Start goal.
  import { onMount } from 'svelte';
  import { registry } from '../../../lib/commands.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { takeStagedGoalDraft } from './goalDraft';
  import { fromStaged } from './goalForm';
  import { goalLauncher } from './goalLauncher.svelte';

  const GoalForm = lazyComponent(() => import('./GoalForm.svelte'));

  let { workspaceId, command = true }: { workspaceId: string; command?: boolean } = $props();

  onMount(() => {
    const draft = takeStagedGoalDraft();
    if (draft) goalLauncher.open(fromStaged(draft));
    return () => goalLauncher.close();
  });

  $effect(() => {
    if (!command) return;
    return registry.register('goal', [
      { id: 'workflows.goal', title: 'New goal…', group: 'Workflows', keywords: 'goal loop objective criteria iterate until done budget', run: () => goalLauncher.open() },
    ]);
  });

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
