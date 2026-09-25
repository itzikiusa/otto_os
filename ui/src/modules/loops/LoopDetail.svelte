<script lang="ts">
  import { loops } from '../../lib/stores/loops.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import SessionView from '../agents/SessionView.svelte';
  import IterationRow from './IterationRow.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { humanVerification } from './verification';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { loopStatus } from './loopStatus';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { formatSeconds } from '../../lib/metric-format';
  import type { GoalLoop } from '../../lib/api/types';

  let { id, onback }: { id: string; onback: () => void } = $props();

  let answers = $state<Record<string, string>>({});
  let evidence = $state<Record<string, string>>({});
  /** Lifecycle call in flight (Pause / Resume / Stop) — disables the header verbs. */
  let acting = $state(false);

  let openSessionId = $state<string | null>(null);

  // Load + poll the open detail; stop polling when this view goes away.
  $effect(() => {
    void loops.loadDetail(id);
    loops.startPoll(id);
    return () => loops.stopPoll();
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

  const PHASES = ['planning', 'executing', 'evaluating', 'digesting'] as const;
  const PHASE_LABEL: Record<(typeof PHASES)[number], string> = {
    planning: 'Plan',
    executing: 'Execute',
    evaluating: 'Evaluate',
    digesting: 'Digest',
  };
  function phaseIndex(loop: GoalLoop): number {
    const p = loop.phase === 'waiting' ? 'executing' : loop.phase;
    return PHASES.indexOf(p as (typeof PHASES)[number]);
  }

  function elapsedSecs(l: GoalLoop): number {
    let s = l.elapsed_secs;
    if (l.status === 'running' && l.run_started_at) {
      s += Math.max(0, (Date.now() - new Date(l.run_started_at).getTime()) / 1000);
    }
    return s;
  }

  function errText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  /** Run a lifecycle/ledger call; a failure toasts with a human title. */
  async function act(fn: () => Promise<void>, failTitle: string): Promise<void> {
    acting = true;
    try {
      await fn();
    } catch (e) {
      toasts.error(failTitle, errText(e));
    } finally {
      acting = false;
    }
  }
  // Irreversible: confirm first, and only leave the page once the delete landed
  // (a failed delete keeps the user on the loop with the error toast).
  async function del(): Promise<void> {
    const name = loop?.name ?? 'this goal loop';
    if (!(await confirmer.ask(`Delete “${name}” and its iteration history? Retained work on disk is kept.`, { title: 'Delete goal loop' }))) return;
    try {
      await loops.remove(id);
    } catch (e) {
      toasts.error('Couldn’t delete the goal loop', errText(e));
      return;
    }
    toasts.success('Goal loop deleted', name);
    onback();
  }
  // Stop is terminal (a stopped loop can't be resumed), unlike Pause.
  async function stop(): Promise<void> {
    if (!(await confirmer.ask('Stop this goal loop? A stopped loop can’t be resumed — use Pause to continue later.', { title: 'Stop goal loop', confirmLabel: 'Stop loop' }))) return;
    await act(() => loops.stop(id), 'Couldn’t stop the goal loop');
  }
  /** ⋯ next to Resume: the destructive verbs stay one step away from the primary. */
  function moreMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Stop loop…', icon: 'square', danger: true, action: () => void stop() },
      { label: 'Delete loop…', icon: 'trash', danger: true, action: () => void del() },
    ]);
  }
  const unanswered = $derived(loop?.ledger?.questions.some((q) => !q.answer) ?? false);
  const openQuestions = $derived((loop?.ledger?.questions ?? []).filter((q) => !q.answer));
  const answeredQuestions = $derived((loop?.ledger?.questions ?? []).filter((q) => !!q.answer));
  const canVerify = $derived(!!loop && ['paused', 'blocked', 'exhausted'].includes(loop.status));
</script>

<div class="detail-page">
<PageHeader title={loop?.name ?? 'Goal loop'}>
  {#snippet leading()}
    <button class="icon-btn" title="Back to Goal Loops" aria-label="Back to Goal Loops" onclick={onback}>
      <Icon name="chevronLeft" size={15} />
    </button>
  {/snippet}
  {#snippet badge()}
    {#if loop}<StatusBadge status={loopStatus(loop.status)} />{/if}
  {/snippet}
  {#snippet actions()}
    {#if loop}
      {#if loop.status === 'running'}
        <button class="btn small" data-icon="square" disabled={acting} onclick={() => act(() => loops.pause(id), 'Couldn’t pause the goal loop')}>Pause</button>
        <button class="btn small danger" data-overflow="-1" data-icon="x" disabled={acting} onclick={stop}>Stop…</button>
      {:else if loop.status === 'paused' || loop.status === 'blocked' || loop.status === 'exhausted'}
        <button class="icon-btn" data-overflow="-1" data-icon="more" data-label="More actions" onclick={moreMenu}
          aria-label="More actions" title="More actions" aria-haspopup="menu">
          <Icon name="more" size={14} />
        </button>
        <button class="btn small primary" disabled={unanswered || acting} title={unanswered ? 'Answer the open decision below first' : 'Continue iterating toward the goal'}
          onclick={() => act(() => loops.resume(id), 'Couldn’t resume the goal loop')}><Icon name="play" size={12} /> Resume</button>
      {:else}
        <button class="icon-btn" data-overflow="-2" data-icon="trash" data-label="Delete loop" onclick={del}
          aria-label="Delete goal loop" title="Delete goal loop"><Icon name="trash" size={14} /></button>
      {/if}
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
    error={!loop && !loops.loadingDetail ? 'The daemon didn’t return it — it may have been deleted, or the daemon is unreachable.' : null}
    empty={!loop}
    onretry={() => void loops.loadDetail(id)}
  >
  {#if loop}
    <section class="overview card-box" aria-label="Progress">
      <div class="progress-row">
        <div class="bar" role="progressbar" aria-label="Progress toward the goal" aria-valuemin={0} aria-valuemax={100} aria-valuenow={loop.progress_pct}>
          <span class="bar-fill" class:done={loop.status === 'succeeded'} style:width={`${loop.progress_pct}%`}></span>
        </div>
        <span class="pct">{loop.progress_pct}%</span>
      </div>

      <ol class="stepper" aria-label="Iteration phases">
        {#each PHASES as p, i (p)}
          {@const cur = loop.status === 'running' ? phaseIndex(loop) : -1}
          <li class="step" class:active={cur === i} class:past={cur > i} aria-current={cur === i ? 'step' : undefined}>
            <span class="step-dot">{#if cur > i}<Icon name="check" size={10} />{/if}</span>{PHASE_LABEL[p]}
          </li>
        {/each}
        {#if loop.status === 'running' && loop.phase === 'waiting'}<li class="step waiting">Waiting on an agent</li>{/if}
      </ol>

      <dl class="stats">
        <div><dt>Iteration</dt><dd>{loop.current_iteration} <span class="of">/ {loop.limits.max_iterations}</span></dd></div>
        <div><dt>Time used</dt><dd>{formatSeconds(Math.round(elapsedSecs(loop)))} <span class="of">/ {formatSeconds(loop.limits.max_runtime_secs)}</span></dd></div>
        <div><dt>Mode</dt><dd>{loop.config.mode === 'research' ? 'Research' : 'Build'}</dd></div>
        {#if loop.branch}<div class="wide"><dt>Branch</dt><dd class="mono" title={loop.branch}>{loop.branch}</dd></div>{/if}
      </dl>

      {#if loop.summary}<p class="summary">{loop.summary}</p>{/if}
      {#if loop.error}
        <p class="errline" role="alert"><Icon name="warning" size={13} /> <span>{loop.error}</span></p>
      {/if}
      {#if loop.ledger?.next_action}<p class="summary"><strong>Next:</strong> {loop.ledger.next_action}</p>{/if}
      {#if loop.worktree_path}
        <p class="retained">Retained work <code title={loop.worktree_path}>{loop.worktree_path}</code></p>
      {/if}
    </section>

    {#each openQuestions as q (q.id)}
      <section class="decision" aria-label="Decision needed">
        <h3 class="section-title"><Icon name="warning" size={12} /> Decision needed</h3>
        <p class="q">{q.question}</p>
        <textarea class="input answer" rows="2" aria-label="Answer question" placeholder="Your answer — the agents read it on Resume" bind:value={answers[q.id]}></textarea>
        <div class="row-end">
          <button class="btn small" disabled={!answers[q.id]?.trim() || loop.status !== 'blocked' || acting}
            title={loop.status !== 'blocked' ? 'Answers are recorded while the loop is blocked' : undefined}
            onclick={() => act(() => loops.answerQuestion(id, q.id, answers[q.id]), 'Couldn’t record the answer')}>Record answer</button>
        </div>
      </section>
    {/each}

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
          {@const met = latestMet[c.id]}
          <li class="crit">
            <span class="crit-mark" class:met={met === true} title={met === true ? 'Met at the last evaluation' : met === false ? 'Not met at the last evaluation' : 'Not evaluated yet'}>
              <Icon name={met === true ? 'check' : 'dot'} size={12} />
            </span>
            <div class="crit-body">
              <span class="crit-text">{c.text}</span>
              <span class="crit-how">Verified by {c.verify_kind === 'human' ? 'a person' : c.verify_kind === 'command' ? 'a shell command' : 'an agent'} — {c.verify}</span>
              {#if c.verify_kind === 'human'}
                {@const approval = humanVerification(c, loop.ledger)}
                {#if approval}<p class="verified"><Icon name="userCheck" size={12} /> Verified by {approval.verified_by}: {approval.evidence}</p>
                {:else}
                  <textarea class="input answer" rows="2" aria-label={`Evidence for ${c.id}`} placeholder="What did you verify?" bind:value={evidence[c.id]}></textarea>
                  <div class="row-end">
                    <button class="btn small" disabled={!evidence[c.id]?.trim() || !canVerify || acting}
                      title={canVerify ? undefined : 'Record verification while the loop is paused, blocked or exhausted'}
                      onclick={() => act(() => loops.verifyCriterion(id, c.id, evidence[c.id]), 'Couldn’t record the verification')}>Record verification</button>
                  </div>
                {/if}
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    </section>

    {#if answeredQuestions.length}
      <section class="goal">
        <h3 class="section-title">Decisions</h3>
        {#each answeredQuestions as q (q.id)}
          <p class="q-done"><span class="dim">{q.question}</span><br />{q.answer} <span class="dim">— {q.answered_by}</span></p>
        {/each}
      </section>
    {/if}

    {#if openSessionId}
      <section class="sess">
        <SessionView
          sessionId={openSessionId}
          focused={true}
          showClose={true}
          onfocus={() => {}}
          onclosepane={() => (openSessionId = null)}
        />
      </section>
    {/if}

    <section class="timeline">
      <h3 class="section-title">Iterations</h3>
      {#if iterations.length === 0}
        <p class="dim empty-line">{loop.status === 'running' ? 'The first iteration is being planned…' : 'No iterations yet.'}</p>
      {:else}
        {#each iterations as it, i (it.id)}
          <IterationRow
            iter={it}
            loopId={id}
            loopStatus={loop.status}
            executorCount={loop.config.executors.length}
            criteria={critText}
            open={i === 0}
            onopensession={(sid) => (openSessionId = sid)}
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
    border-radius: 3px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar-fill {
    display: block;
    height: 100%;
    background: var(--status-working);
  }
  .bar-fill.done {
    background: var(--success);
  }
  .pct {
    font-size: var(--fs-s);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    min-width: 4ch;
    text-align: end;
  }
  .stepper {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    list-style: none;
    margin: 12px 0 0;
    padding: 0;
  }
  .step {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .step.active {
    color: var(--text);
    font-weight: 600;
  }
  .step.waiting {
    color: var(--warning);
  }
  .step-dot {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1.5px solid var(--border-strong);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
  }
  .step.past .step-dot {
    border-color: var(--status-working);
    color: var(--status-working);
  }
  .step.active .step-dot {
    background: var(--status-working);
    border-color: var(--status-working);
  }
  @media (prefers-reduced-motion: no-preference) {
    .step.active .step-dot { animation: pulse 1.6s ease-in-out infinite; }
  }
  @keyframes pulse { 50% { opacity: 0.45; } }
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
  .decision {
    border: 1px solid color-mix(in srgb, var(--warning) 40%, transparent);
    background: var(--warning-soft);
    border-radius: var(--radius-m);
    padding: 12px 16px;
  }
  .decision .section-title {
    margin-top: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--warning);
  }
  .q {
    font-size: var(--fs-m);
    margin: 0 0 8px;
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
  .sess {
    height: 360px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
</style>
