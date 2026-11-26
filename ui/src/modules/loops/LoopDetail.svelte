<script lang="ts">
  // One older goal loop, read-only (goal.md §5.2): its progress, criteria and
  // evidence per iteration, the graph of a graph loop, the kept work and its
  // diff. The only actions: Continue as a goal (an unfinished loop) and
  // Delete history. Goal Loops are now goals in Workflows.
  import { announceShown } from '../../lib/announce.svelte';
  import { scrollBehavior } from '../../lib/motion';
  import { loops } from '../../lib/stores/loops.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import IterationRow from './IterationRow.svelte';
  import GoalGraphView from './GoalGraphView.svelte';
  import IterationDiagram from './IterationDiagram.svelte';
  import LoopChanges from './LoopChanges.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { PHASE_LABEL, progressFill, criterionMark } from './loopPresentation';
  import LoadState from '../../lib/components/LoadState.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { formatSeconds } from '../../lib/metric-format';
  import type { AcceptanceCriterion, GoalLoop, GoalLoopLedger } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import { canContinue, loopStatusOf, pausedByRestart } from './loopStatus';
  import { tick } from 'svelte';

  // The embedded session (terminal + xterm) loads on a click on an agent.
  const SessionViewLazy = lazyComponent(() => import('../agents/SessionView.svelte'));

  let { id, onback }: { id: string; onback: () => void } = $props();

  let openSessionId = $state<string | null>(null);
  let sessEl = $state<HTMLElement | null>(null);
  /** True only while focus is inside the embedded session — it must not take keys otherwise. */
  let sessFocused = $state(false);
  /** Phone has no room for an embedded transcript; open the session in Agents. */
  async function openSession(sid: string): Promise<void> {
    if (viewport.isPhone) {
      ws.navigateToSession(sid);
      return;
    }
    sessFocused = false;
    openSessionId = sid;
    await tick();
    sessEl?.scrollIntoView({ block: 'nearest', behavior: scrollBehavior() });
  }
  function sessionBlur(e: FocusEvent): void {
    const next = e.relatedTarget;
    if (!(next instanceof Node) || !sessEl?.contains(next)) sessFocused = false;
  }

  $effect(() => {
    void loops.loadDetail(id);
  });

  const detail = $derived(loops.detail && loops.detail.loop.id === id ? loops.detail : null);
  const loop = $derived<GoalLoop | null>(detail?.loop ?? null);
  // Newest iteration first.
  const iterations = $derived([...(detail?.iterations ?? [])].reverse());
  /** Criterion id → its text, so evaluation chips can say what they judged. */
  const critText = $derived(
    Object.fromEntries((loop?.definition.acceptance_criteria ?? []).map((c) => [c.id, c.text])) as Record<string, string>,
  );
  /** Latest verdict per criterion (from the newest evaluated iteration). */
  const latestMet = $derived.by(() => {
    const ev = iterations.find((it) => it.evaluation)?.evaluation;
    return Object.fromEntries((ev?.criteria ?? []).map((c) => [c.id, c.met])) as Record<string, boolean>;
  });
  const questions = $derived(loop?.ledger?.questions ?? []);
  const cap = $derived(!loop ? null : loop.iterations_started >= loop.limits.max_iterations ? 'iterations' : loop.elapsed_secs >= loop.limits.max_runtime_secs ? 'runtime' : null);
  const continuable = $derived(!!loop && canContinue(loop));

  /** A person's recorded verification, only while it matches the criterion as written. */
  function humanVerification(c: AcceptanceCriterion, ledger?: GoalLoopLedger) {
    return ledger?.verifications.find((v) => {
      if (v.criterion_id !== c.id || !v.evidence.trim()) return false;
      try {
        const prior = JSON.parse(v.criterion_revision) as AcceptanceCriterion;
        return prior.text === c.text && prior.verify === c.verify && prior.verify_kind === c.verify_kind && (prior.verify_cmd ?? null) === (c.verify_cmd ?? null);
      } catch {
        return false;
      }
    });
  }

  /** Continue as a goal: the Workflows page opens the Goal form filled from
   *  this loop (workflows/goal/goalDraft.ts takes the staged id). */
  function continueAsGoal(): void {
    try {
      sessionStorage.setItem('otto:automation-draft', JSON.stringify({ target: 'goal', loop_id: id }));
    } catch {
      /* private storage: the page still opens */
    }
    router.go('workflows');
  }

  // Irreversible: confirm first, and only leave the page once the delete landed.
  async function del(): Promise<void> {
    const loopId = id;
    const name = loop?.name ?? 'this goal loop';
    if (!(await confirmer.ask(`Delete the history of “${name}”? Its kept work on disk stays.`, { title: 'Delete history', confirmLabel: 'Delete', danger: true }))) return;
    try {
      await loops.remove(loopId);
    } catch (e) {
      toastError('Couldn’t delete the history', e);
      return;
    }
    toasts.success('History deleted', name);
    if (id === loopId) onback();
  }
  function more(): MenuItem[] {
    return loop ? [{ label: 'Delete history…', icon: 'trash', danger: true, action: () => void del() }] : [];
  }
</script>

<div class="detail-page">
<PageHeader class="loop-header" title={loop?.name ?? 'Goal loop'} subtitle={loop ? (loop.config.graph ? 'Older engineering graph' : 'Older goal loop') : undefined} {more}>
  {#snippet leading()}
    <button class="icon-btn" title="Back to older goal loops" aria-label="Back to older goal loops" onclick={onback}>
      <Icon name="chevronLeft" size={14} />
    </button>
  {/snippet}
  {#snippet badge()}
    {#if loop && !viewport.isPhone}<StatusBadge status={loopStatusOf(loop)} />{/if}
  {/snippet}
  {#snippet actions()}
    {#if continuable}
      <button class="btn small primary" data-testid="loop-continue" title="Go on in this loop’s kept work, as a goal in Workflows" onclick={continueAsGoal}>
        <Icon name="play" size={12} /> {viewport.isPhone ? 'Continue' : 'Continue as a goal'}
      </button>
    {/if}
  {/snippet}
</PageHeader>
<PageBody width="readable">
<div class="detail">
  <!-- The store keeps the prior detail on a failed load, so "no loop and not
       loading" means the first load failed — say so, with Retry. -->
  <LoadState
    what="this goal loop"
    variant="page"
    loading={loops.loadingDetail}
    error={loops.detailError}
    empty={!loop}
    onretry={() => void loops.loadDetail(id)}
  >
  {#if loop}
    <section class="overview card-box" aria-label="Progress">
      <div class="progress-row">
        <div class="bar" role="progressbar" aria-label="Progress toward the goal" aria-valuemin={0} aria-valuemax={100} aria-valuenow={loop.progress_pct}>
          <span class="bar-fill {progressFill(loop.status)}" style:width={`${loop.progress_pct}%`}></span>
        </div>
        <span class="pct">{loop.progress_pct}%</span>
      </div>

      {#if !loop.config.graph}<IterationDiagram {loop} />
      {:else if loop.phase !== 'done'}<p class="summary">Current phase: {loop.phase === 'waiting' ? 'Waiting on an agent' : PHASE_LABEL[loop.phase]}</p>{/if}

      <dl class="stats">
        <div><dt>Iteration</dt><dd>{loop.current_iteration} <span class="of" class:cap-hit={cap === 'iterations'}>/ {loop.limits.max_iterations}</span>{#if cap === 'iterations'} <span class="cap-hit">cap reached</span>{/if}</dd></div>
        <div><dt>Time used</dt><dd>{formatSeconds(Math.round(loop.elapsed_secs))} <span class="of" class:cap-hit={cap === 'runtime'}>/ {formatSeconds(loop.limits.max_runtime_secs)}</span>{#if cap === 'runtime'} <span class="cap-hit">cap reached</span>{/if}</dd></div>
        <div><dt>Work type</dt><dd>{loop.config.mode === 'research' ? 'Research' : 'Build'}</dd></div>
        {#if loop.branch}<div class="wide"><dt>Branch</dt><dd class="mono" title={loop.branch}>{loop.branch}</dd></div>{/if}
      </dl>
      {#if pausedByRestart(loop)}<p class="summary">Otto restarted while this loop ran; its work is kept. Continue it as a goal to go on.</p>{/if}

      {#if loop.summary}<p class="summary">{loop.summary}</p>{/if}
      {#if loop.error && !pausedByRestart(loop)}<p class="errline" use:announceShown={{ text: loop.error, assertive: true }}><Icon name="warning" size={13} /> <span>{loop.error}</span></p>{/if}
      {#if loop.ledger?.next_action}<p class="summary"><strong>Next:</strong> {loop.ledger.next_action}</p>{/if}
      {#if loop.worktree_path}
        <p class="retained">Retained work <code title={loop.worktree_path}>{loop.worktree_path}</code></p>
      {/if}
    </section>

    {#if loop.worktree_path}<LoopChanges loopId={id} />{/if}

    {#if loop.config.graph}
      <div class="card-box">
        <GoalGraphView graph={loop.config.graph} states={loop.ledger?.graph_nodes ?? []} criteria={loop.definition.acceptance_criteria} loopStatus={loop.status} />
      </div>
    {/if}


    {#if loop.ledger?.review_summary}
      <details class="review">
        <summary>
          <Icon name={loop.ledger.review_passed ? 'check' : 'warning'} size={12} />
          Completion review · {loop.ledger.review_passed ? 'passed' : 'needs attention'}
        </summary>
        <pre>{loop.ledger.review_summary}</pre>
      </details>
    {/if}

    <section class="goal">
      <h3 class="section-title">Goal</h3>
      <p class="goal-sum">{loop.definition.summary || loop.definition.title}</p>
      <ul class="crit-list" aria-label="Acceptance criteria">
        {#each loop.definition.acceptance_criteria as c (c.id)}
          {@const mark = criterionMark(latestMet[c.id])}
          <li class="crit">
            <span class="crit-mark {mark.kind}" title={mark.label}>
              <Icon name={mark.icon} size={12} />
            </span>
            <div class="crit-body">
              <span class="crit-state {mark.kind}">{mark.label}</span>
              <span class="crit-text" id={`crit-text-${c.id}`}>{c.text}</span>
              <span class="crit-how">Verified by {c.verify_kind === 'human' ? 'a person' : c.verify_kind === 'command' ? 'a shell command' : 'an agent'} — {c.verify}</span>
              {#if c.verify_kind === 'human'}
                {@const approval = humanVerification(c, loop.ledger)}
                {#if approval}<p class="verified"><Icon name="userCheck" size={12} /> Verified by {approval.verified_by}: {approval.evidence}</p>{:else}<p class="dim">No person recorded this.</p>{/if}
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    </section>

    {#if questions.length}
      <section class="goal">
        <h3 class="section-title">Decisions</h3>
        {#each questions as q (q.id)}
          <p class="q-done"><span class="dim">{q.question}</span><br />{#if q.answer}{q.answer} <span class="dim">— {q.answered_by}</span>{:else}<span class="dim">Not answered</span>{/if}</p>
        {/each}
      </section>
    {/if}

    {#if openSessionId && !viewport.isPhone}
      <section class="sess" bind:this={sessEl} aria-label="Agent session" onfocusin={() => (sessFocused = true)} onfocusout={sessionBlur}>
        <div class="sess-bar">
          <button type="button" class="btn small" onclick={() => ws.navigateToSession(openSessionId ?? '')}>Open in Agents</button>
        </div>
        <div class="sess-pane">
          <LazyMount
            lazy={SessionViewLazy}
            what="the agent session"
            variant="panel"
            props={{
              sessionId: openSessionId,
              focused: sessFocused,
              showClose: true,
              onfocus: () => (sessFocused = true),
              onclosepane: () => { openSessionId = null; sessFocused = false; },
            }}
          />
        </div>
      </section>
    {/if}

    <section class="timeline">
      <h3 class="section-title">Iterations</h3>
      {#if iterations.length === 0}
        <p class="dim empty-line">{loop.status === 'running' && loop.phase === 'waiting' ? 'Waiting on an agent.' : loop.status === 'running' ? 'The first iteration is being planned…' : 'No iterations yet.'}</p>
      {:else}
        {#each iterations as it, i (it.id)}
          <IterationRow
            iter={it}
            loopId={id}
            criteria={critText}
            open={i === 0}
            onopensession={(sid) => void openSession(sid)}
          />
        {/each}
      {/if}
    </section>
  {/if}
  </LoadState>
</div>
</PageBody>
</div>

<style>
  /* Keep Resume and the overflow menu reachable beside a long goal name. */
  @media (max-width: 640px) {
    .detail-page :global(.loop-header .ph-title-block) { min-width: 0; max-width: 45%; }
  }
  .cap-hit { color: var(--warning); }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--fs-s); margin: 8px 0 0; }
  .detail-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .detail {
    display: flex;
    flex-direction: column;
    gap: 16px;
    flex: 1;
    min-height: 0;
  }
  .card-box {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 14px 16px;
  }
  .progress-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .bar {
    flex: 1;
    height: 6px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar-fill {
    display: block;
    height: 100%;
    background: var(--text-dim);
  }
  .bar-fill.running { background: var(--info); }
  .bar-fill.warn { background: var(--warning); }
  .bar-fill.bad { background: var(--danger); }
  .bar-fill.ok { background: var(--success); }
  .bar-fill.dim { background: var(--text-dim); }
  .pct {
    font-size: var(--fs-s);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    min-width: 4ch;
    text-align: end;
  }
  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 28px;
    margin: 14px 0 0;
  }
  .stats > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .stats > div.wide {
    flex: 1 1 200px;
  }
  .stats dt {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .stats dd {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .stats dd.mono {
    font-family: var(--font-mono);
    font-size: var(--fs-m);
    font-weight: 500;
    direction: ltr;
    text-align: start;
  }
  .of {
    color: var(--text-dim);
    font-weight: 400;
    font-size: var(--fs-m);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .dim {
    color: var(--text-dim);
  }
  .summary {
    font-size: var(--fs-m);
    margin: 12px 0 0;
  }
  .errline {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: var(--fs-m);
    color: var(--text);
    background: var(--danger-soft);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    margin: 12px 0 0;
  }
  .errline :global(svg) {
    color: var(--danger);
    flex-shrink: 0;
    margin-block-start: 2px;
  }
  .retained {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin: 10px 0 0;
    min-width: 0;
  }
  .retained code {
    font-family: var(--font-mono);
    color: var(--text);
    direction: ltr;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .q {
    font-size: var(--fs-m);
    margin: 0 0 8px;
  }
  .resume-hint {
    margin: -4px 0 8px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .q-done {
    font-size: var(--fs-m);
    margin: 0 0 8px;
  }
  .answer {
    width: 100%;
    box-sizing: border-box;
  }
  .row-end {
    display: flex;
    justify-content: flex-end;
    margin-top: 6px;
  }
  .review {
    font-size: var(--fs-m);
  }
  .review summary {
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .goal .section-title,
  .timeline .section-title {
    margin-top: 0;
  }
  .goal-sum {
    white-space: pre-line;
    font-size: var(--fs-m);
    margin: 0 0 10px;
  }
  .crit-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .crit {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }
  .crit-mark {
    width: 18px;
    height: 18px;
    flex: none;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
    background: var(--surface-2);
  }
  .crit-mark.met {
    color: var(--success);
    background: var(--success-soft);
  }
  .crit-mark.unmet {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .crit-state {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .crit-state.met { color: var(--success); }
  .crit-state.unmet { color: var(--danger); }
  .crit-state.unchecked { color: var(--text-dim); }
  .crit-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .crit-text {
    font-size: var(--fs-m);
  }
  .crit-how {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .verified {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--success);
    margin: 4px 0 0;
  }
  .empty-line {
    font-size: var(--fs-m);
    margin: 0;
  }
  .detail-page :global(.page-body-inner) {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }
  .sess {
    flex: 1;
    min-height: 320px;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .sess-bar {
    display: flex;
    justify-content: flex-end;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex: none;
  }
  .sess-pane {
    flex: 1;
    min-height: 0;
  }
  .sess-pane :global(.pane) {
    height: 100%;
    border: none;
    border-radius: 0;
  }
</style>
