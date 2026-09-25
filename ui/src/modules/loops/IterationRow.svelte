<script lang="ts">
  import { loops } from '../../lib/stores/loops.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { GoalLoopIteration, LoopAgentState } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';

  let {
    iter,
    loopId,
    loopStatus,
    executorCount,
    open = false,
    onopensession,
  }: {
    iter: GoalLoopIteration;
    loopId: string;
    loopStatus: string;
    executorCount: number;
    open?: boolean;
    onopensession: (sessionId: string) => void;
  } = $props();

  // Initial expand state from the `open` prop (newest iteration starts open);
  // toggled independently thereafter.
  // svelte-ignore state_referenced_locally
  let expanded = $state(open);

  function dotClass(status: string): string {
    switch (status) {
      case 'running':
        return 'dot working';
      case 'done':
        return 'dot ok';
      case 'waiting':
        return 'dot warn';
      case 'error':
        return 'dot bad';
      default:
        return 'dot';
    }
  }

  async function retry(agentIndex: number): Promise<void> {
    try {
      await loops.retryExecutor(loopId, iter.idx, agentIndex);
    } catch (e) {
      toasts.error('Retry failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Retry is only valid while the loop is blocked (no live controller); a
  // running loop's controller owns the executor and a second run would race it.
  function canRetry(a: LoopAgentState): boolean {
    return loopStatus === 'blocked' && (a.status === 'waiting' || a.status === 'error');
  }
</script>

<div class="iter">
  <button class="iter-head" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
    <span class="chev"><Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} /></span>
    <span class="idx">Iteration {iter.idx}</span>
    <StatusBadge status={runStatus(iter.status)} variant="text" />
    {#if iter.evaluation}
      <span class="prog">{iter.evaluation.progress_pct}% · {iter.evaluation.verdict}</span>
    {/if}
  </button>

  {#if expanded}
    <div class="body">
      {#if iter.plan}
        <section>
          <h4>Plan</h4>
          <pre class="text">{iter.plan}</pre>
        </section>
      {/if}

      <section>
        <h4>Agents and roles</h4>
        {#each iter.agents as a, i (i)}
          <div class="agent">
            <span class={dotClass(a.status)}></span>
            <span class="aname">{a.name}</span>
            <span class="anote">{a.provider}</span>
            <span class="anote">{a.note || a.output_summary || a.status}</span>
            <span class="spacer"></span>
            {#if a.session_id}
              <button class="btn ghost small" onclick={() => onopensession(a.session_id ?? '')}>Open</button>
            {/if}
            {#if i < executorCount && canRetry(a)}
              <button class="btn small" onclick={() => retry(i)}>Retry</button>
            {/if}
          </div>
        {/each}
      </section>

      {#if iter.evaluation}
        <section>
          <h4>Evaluation</h4>
          <div class="crits">
            {#each iter.evaluation.criteria as c (c.id)}
              <span class={c.met ? 'chip met' : 'chip unmet'} title={c.evidence}>
                {c.met ? '✓' : '○'} {c.id}
              </span>
            {/each}
          </div>
          {#if iter.evaluation.feedback}
            <p class="fb"><strong>Feedback:</strong> {iter.evaluation.feedback}</p>
          {/if}
        </section>
      {/if}

      {#if iter.context_out}
        <section>
          <h4>Context carried forward</h4>
          <pre class="text dim">{iter.context_out}</pre>
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
  .iter {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    margin-bottom: 8px;
    background: var(--surface);
  }
  .iter-head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text);
    font-size: 12.5px;
  }
  .chev {
    display: inline-flex;
    color: var(--text-dim);
  }
  .idx {
    font-weight: 600;
  }
  .prog {
    margin-inline-start: auto;
    color: var(--status-working);
    font-size: 11.5px;
  }
  .body {
    padding: 4px 12px 12px;
  }
  h4 {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    margin: 12px 0 6px;
  }
  .text {
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 12px;
    background: var(--bg);
    border-radius: var(--radius-s);
    padding: 8px;
    margin: 0;
    font-family: inherit;
  }
  .text.dim {
    color: var(--text-dim);
  }
  .agent {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
    font-size: 12.5px;
  }
  .aname {
    font-weight: 600;
  }
  .anote {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .spacer {
    flex: 1;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--status-idle);
    flex: none;
  }
  .dot.working {
    background: var(--status-working);
  }
  .dot.ok {
    background: var(--status-working);
  }
  .dot.warn {
    background: var(--status-warn);
  }
  .dot.bad {
    background: var(--status-exited);
  }
  .crits {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
  }
  .chip.met {
    background: var(--success-soft);
    color: var(--success);
  }
  .chip.unmet {
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .fb {
    font-size: 12px;
    margin: 8px 0 0;
  }
</style>
