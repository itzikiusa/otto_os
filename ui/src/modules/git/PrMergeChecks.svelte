<script lang="ts">
  // The forge's merge rules and open tasks (`PrDetail.merge_checks`,
  // `task_count`) for the PR's details card: pass/fail icon plus the forge's
  // detail text. Called "Merge rules", never "checks" — that word is the CI
  // checks', which the PR's one status line carries.
  import type { PrDetail } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { mergeCheckSummary, openTasksLine } from './pr-merge-checks';

  interface Props {
    pr: Pick<PrDetail, 'merge_checks' | 'task_count'>;
  }
  let { pr }: Props = $props();
  const checks = $derived(pr.merge_checks ?? []);
  const summary = $derived(mergeCheckSummary(checks));
  const tasks = $derived(openTasksLine(pr.task_count));
</script>

{#if summary || tasks}
  <section aria-label="Merge rules">
    <h3 class="h">{summary ? 'Merge rules' : 'Tasks'} {#if summary}<span class="dim">{summary}</span>{/if}</h3>
    {#each checks as c (c.key)}
      <div class="mc-row">
        <span class="mc-ic" class:ok={c.passed} class:bad={!c.passed} aria-hidden="true"><Icon name={c.passed ? 'check' : 'x'} size={12} /></span>
        <span class="mc-text">
          <span>{c.label}<span class="sr-only">{c.passed ? ' — met' : ' — not met'}</span></span>
          {#if c.detail}<span class="mc-detail dim">{c.detail}</span>{/if}
        </span>
      </div>
    {/each}
    {#if tasks}<p class="mc-tasks" data-testid="pr-open-tasks">{tasks}</p>{/if}
  </section>
{/if}

<style>
  .h {
    margin: 0 0 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .h .dim { text-transform: none; letter-spacing: 0; font-weight: 400; }
  .mc-row { display: flex; align-items: flex-start; gap: 8px; padding-block: 4px; min-width: 0; }
  .mc-ic { display: inline-flex; flex: none; padding-block-start: 2px; }
  .mc-text { display: flex; flex-direction: column; min-width: 0; overflow-wrap: anywhere; }
  .mc-detail { font-size: var(--fs-xs); }
  .mc-tasks { margin: 4px 0 0; color: var(--warning); font-size: var(--fs-s); }
  .ok { color: var(--success); }
  .bad { color: var(--danger); }
</style>
