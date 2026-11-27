<script lang="ts">
  // A goal's criteria (goal.md §3.5): the word (Met · Not met · Verify), the
  // method chip (Command · Agent · You · Rule), the evidence's first line, and
  // an expander with the rest of the evidence and a command's output tail.
  import Badge from '../../../lib/components/Badge.svelte';
  import { METHOD_WORD, STATE_WORD, type CriterionRow } from './goalCheck';

  let { rows }: { rows: readonly CriterionRow[] } = $props();
  const MET_ATTR = { met: 'true', not_met: 'false', verify: 'verify', pending: undefined } as const;
</script>

<ul class="crit">
  <!-- Two check steps of one run may each have a criterion with the same id: key by place too. -->
  {#each rows as r, i (`${i}:${r.id}`)}
    <li data-testid="wf-goal-criterion" data-criterion-id={r.id} data-met={MET_ATTR[r.state]} data-method={r.method}>
      <div class="line">
        <span class="w {r.state}">{STATE_WORD[r.state]}</span>
        <span class="text" dir="auto">{r.text}</span>
        <Badge variant="outline" label={METHOD_WORD[r.method] ?? r.method} />
      </div>
      {#if r.detail || r.output}
        <details class="ev">
          <summary dir="auto">{r.headline || 'Evidence'}{r.by && !['command', 'rule', 'agent'].includes(r.by) ? ` — ${r.by}` : ''}</summary>
          {#if r.detail}<p class="detail" dir="auto">{r.detail}</p>{/if}
          {#if r.output}<pre class="out" dir="ltr">{r.output}</pre>{/if}
        </details>
      {:else if r.headline}
        <p class="head" dir="auto">{r.headline}{r.by && !['command', 'rule', 'agent'].includes(r.by) ? ` — ${r.by}` : ''}</p>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .crit {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .crit li {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding-block: var(--sp-3);
    border-block-start: 1px solid var(--border);
    font-size: var(--fs-m);
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: var(--sp-4);
  }
  .text {
    flex: 1;
    min-inline-size: 0;
    overflow-wrap: anywhere;
  }
  .w {
    flex: none;
    min-inline-size: 5.5em;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
  }
  .w.met {
    color: var(--success);
  }
  .w.verify {
    color: var(--warning);
  }
  .head,
  .ev summary {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .ev summary {
    cursor: pointer;
  }
  .detail {
    margin: var(--sp-2) 0 0;
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .out {
    max-block-size: 240px;
    margin: var(--sp-2) 0 0;
    padding: var(--sp-3);
    overflow: auto;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    white-space: pre-wrap;
  }
  /* Indent the evidence under the text, past the word. */
  .head,
  .ev {
    padding-inline-start: calc(5.5em + var(--sp-4));
  }
  @media (max-width: 640px) {
    .head,
    .ev {
      padding-inline-start: 0;
    }
  }
</style>
