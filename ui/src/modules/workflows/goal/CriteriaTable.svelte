<script lang="ts">
  // A goal's criteria (goal.md §3.5): the word (Met · Not met · Verify), the
  // numeric ratio + meter + round trend + blocker in words when recorded, the
  // method chip (Command · Agent · You · Rule), the evidence's first line, and
  // an expander with the rest of the evidence, inspected values, and a
  // command's output tail. Keyed by `(step_id, id)` so two check steps with
  // identical criterion ids never share state.
  import Badge from '../../../lib/components/Badge.svelte';
  import {
    METHOD_WORD,
    STATE_WORD,
    formatRatio,
    observedPairs,
    progressPct,
    recentTrend,
    type CriterionRow,
  } from './goalCheck';

  let { rows }: { rows: readonly CriterionRow[] } = $props();
  const MET_ATTR = { met: 'true', not_met: 'false', verify: 'verify', pending: undefined } as const;
  const multiStep = $derived(new Set(rows.map((r) => r.step_id).filter(Boolean)).size > 1);
</script>

<ul class="crit">
  {#each rows as r (r.key)}
    {@const ratio = formatRatio(r.progress)}
    {@const pct = progressPct(r.progress, r.state, r.expr)}
    {@const trend = recentTrend(r.history)}
    {@const blocker = r.state !== 'met' ? (r.progress?.label ?? '') : ''}
    {@const pairs = observedPairs(r.progress)}
    {@const ariaMax = r.progress?.target ?? 100}
    {@const ariaNow = Math.max(0, Math.min(ariaMax, r.progress?.value ?? 0))}
    <li
      data-testid="wf-goal-criterion"
      data-step-id={r.step_id || undefined}
      data-criterion-id={r.id}
      data-met={MET_ATTR[r.state]}
      data-method={r.method}
    >
      <div class="line">
        <span class="w {r.state}">{STATE_WORD[r.state]}</span>
        <span class="text" dir="auto">
          {#if multiStep && r.step_name}<span class="step-label">{r.step_name}</span>{/if}
          {r.text}
        </span>
        {#if ratio}<span class="ratio" data-testid="wf-criterion-ratio">{ratio}</span>{/if}
        <Badge variant="outline" label={METHOD_WORD[r.method] ?? r.method} />
      </div>
      {#if pct !== null || trend}
        <div class="meter-row" data-testid="wf-criterion-meter">
          {#if pct !== null}
            <div
              class="track"
              role="progressbar"
              aria-valuenow={ariaNow}
              aria-valuemin={0}
              aria-valuemax={ariaMax}
              aria-valuetext={ratio || undefined}
              aria-label={`${r.step_name ? `${r.step_name}: ` : ''}${r.text} (${ratio})`}
            >
              <div class="fill {r.state}" style:inline-size="{pct}%"></div>
            </div>
          {/if}
          {#if trend}<span class="trend" data-testid="wf-criterion-trend" dir="ltr">{trend}</span>{/if}
        </div>
      {/if}
      {#if blocker}
        <p class="blocker" data-testid="wf-criterion-blocker" dir="auto">{blocker}</p>
      {/if}
      {#if r.detail || r.output || pairs.length}
        <details class="ev">
          <summary dir="auto">{r.headline || 'Evidence'}{r.by && !['command', 'rule', 'agent'].includes(r.by) ? ` — ${r.by}` : ''}</summary>
          {#if r.detail}<p class="detail" dir="auto">{r.detail}</p>{/if}
          {#if pairs.length}
            <dl class="obs" data-testid="wf-criterion-observed">
              {#each pairs as p (p.key)}
                <div class="obs-item"><dt dir="ltr">{p.key}</dt><dd dir="auto">{p.value}</dd></div>
              {/each}
            </dl>
          {/if}
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
    flex-wrap: wrap;
    gap: var(--sp-3) var(--sp-4);
  }
  .text {
    flex: 1;
    min-inline-size: 0;
    overflow-wrap: anywhere;
  }
  .step-label {
    margin-inline-end: var(--sp-2);
    font-weight: 600;
    color: var(--text);
  }
  .ratio {
    flex: none;
    font-size: var(--fs-s);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
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
  .meter-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-3);
  }
  .track {
    flex: 1;
    min-inline-size: 96px;
    max-inline-size: 260px;
    block-size: 6px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    overflow: hidden;
  }
  .fill {
    block-size: 100%;
    border-radius: var(--radius-s);
    background: var(--accent);
  }
  .fill.met {
    background: var(--success);
  }
  .trend {
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }
  .blocker {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--warning);
    overflow-wrap: anywhere;
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
  .obs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2) var(--sp-4);
    margin: var(--sp-2) 0 0;
    font-size: var(--fs-xs);
  }
  .obs-item {
    display: inline-flex;
    gap: var(--sp-1);
    min-inline-size: 0;
  }
  .obs-item dt {
    font-family: var(--font-mono);
    color: var(--text-dim);
  }
  .obs-item dt::after {
    content: '=';
    margin-inline-start: var(--sp-1);
  }
  .obs-item dd {
    margin: 0;
    font-family: var(--font-mono);
    color: var(--text);
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
  /* Indent the meter, blocker, and evidence under the text, past the word. */
  .meter-row,
  .blocker,
  .head,
  .ev {
    padding-inline-start: calc(5.5em + var(--sp-4));
  }
  @media (max-width: 640px) {
    .meter-row,
    .blocker,
    .head,
    .ev {
      padding-inline-start: 0;
    }
    .track {
      max-inline-size: none;
    }
  }
</style>
