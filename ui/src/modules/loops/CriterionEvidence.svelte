<script lang="ts">
  // One iteration's per-criterion verdicts with the evidence behind each:
  // the evaluator's citation, a person's record, or a verify command's exit
  // status plus the tail of what it printed (kept on the iteration record).
  import type { EvalCriterion } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { splitEvidence } from './loopSafety';

  let { criteria, names = {} }: {
    criteria: EvalCriterion[];
    /** Criterion id → text, so each row names what it judged. */
    names?: Record<string, string>;
  } = $props();
</script>

<ul class="ev-list" aria-label="Criteria and evidence">
  {#each criteria as c (c.id)}
    {@const ev = splitEvidence(c.evidence ?? '')}
    <li class="ev">
      <span class="crit-chip" class:met={c.met}>
        <Icon name={c.met ? 'check' : 'dot'} size={12} />
        <span class="crit-label" title={names[c.id] || c.id}>{names[c.id] || c.id}</span>
        <span class="sr-only">{c.met ? 'met' : 'not met'}</span>
      </span>
      {#if ev.headline}<span class="ev-text" dir="auto">{ev.headline}</span>{/if}
      {#if ev.output}
        <details class="ev-out">
          <summary>Command output</summary>
          <pre dir="ltr">{ev.output}</pre>
        </details>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .ev-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ev {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    min-width: 0;
  }
  .crit-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    max-width: 100%;
    min-width: 0;
  }
  .crit-chip.met {
    background: var(--success-soft);
    color: var(--success);
  }
  .crit-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 40ch;
  }
  .ev-text {
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
    padding-inline-start: 8px;
  }
  .ev-out {
    align-self: stretch;
    padding-inline-start: 8px;
    font-size: var(--fs-s);
  }
  .ev-out summary {
    cursor: pointer;
    color: var(--text-dim);
  }
  .ev-out pre {
    margin: 4px 0 0;
    padding: 8px;
    max-block-size: 240px;
    overflow-y: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    background: var(--bg);
    border-radius: var(--radius-s);
    text-align: start;
  }
</style>
