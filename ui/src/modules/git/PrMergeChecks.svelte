<script lang="ts">
  // The forge's merge checks and open tasks (`PrDetail.merge_checks`,
  // `task_count`). Full: the Summary side column's list, pass/fail icon plus
  // the forge's detail text. Compact: one "2 of 3 checks passed · 1 task open"
  // line for the PR header on phones, where the side column sits far below.
  import type { PrDetail } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { mergeCheckSummary, openTasksLine } from './pr-merge-checks';

  interface Props {
    pr: Pick<PrDetail, 'merge_checks' | 'task_count'>;
    compact?: boolean;
  }
  let { pr, compact = false }: Props = $props();
  const checks = $derived(pr.merge_checks ?? []);
  const summary = $derived(mergeCheckSummary(checks));
  const tasks = $derived(openTasksLine(pr.task_count));
  const failing = $derived(checks.some((c) => !c.passed));
</script>

{#if compact}
  {#if summary || tasks}
    <div class="mc-line" class:bad={failing} data-testid="pr-merge-checks-line">
      <Icon name={failing ? 'warning' : 'check'} size={12} />
      <span>{[summary, tasks].filter(Boolean).join(' · ')}</span>
    </div>
  {/if}
{:else if summary || tasks}
  <section aria-label="Merge checks">
    <h3 class="h">Merge checks {#if summary}<span class="dim">{summary}</span>{/if}</h3>
    {#each checks as c (c.key)}
      <div class="mc-row">
        <span class="mc-ic" class:ok={c.passed} class:bad={!c.passed} aria-hidden="true"><Icon name={c.passed ? 'check' : 'x'} size={12} /></span>
        <span class="mc-text">
          <span>{c.label}<span class="sr-only">{c.passed ? ' — passed' : ' — not passed'}</span></span>
          {#if c.detail}<span class="mc-detail dim">{c.detail}</span>{/if}
        </span>
      </div>
    {/each}
    {#if tasks}<p class="mc-tasks">{tasks}</p>{/if}
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
  .mc-line {
    display: none;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--success);
  }
  .mc-line.bad { color: var(--danger); }
  @media (max-width: 1024px) {
    .mc-line { display: flex; }
  }
</style>
