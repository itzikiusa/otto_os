<script module lang="ts">
  import type { AttentionRef, Criterion, WorkflowRun, RunState } from '../../../lib/api/types';

  /** Which card the run shows (ux.md §4.3): exactly one or none. */
  export type CardKind = 'error' | 'approval' | 'question' | 'limit' | 'verify' | 'hold' | 'delivered' | 'goal' | null;

  /** The oldest person-wait of the run (the overview's, else a step's own). */
  export function oldestWait(run: WorkflowRun): AttentionRef | null {
    const a = run.overview?.attention?.[0];
    if (a) return a;
    for (const n of run.nodes) if (n.status === 'running' && n.attention) return { ...n.attention, node_id: n.node_id, name: n.node_id };
    return null;
  }

  export function cardKind(run: WorkflowRun, runState: RunState): CardKind {
    if (runState === 'failed') return 'error';
    if (runState === 'blocked' || runState === 'exhausted') return 'hold';
    if (runState === 'needs_you') {
      const kind = oldestWait(run)?.kind;
      if (kind === 'question') return 'question';
      if (kind === 'usage_limit') return 'limit';
      if (kind === 'verify') return 'verify';
      return 'approval';
    }
    if (runState === 'succeeded' && run.overview?.deliverables?.length) return 'delivered';
    if (run.overview?.goal) return 'goal';
    return null;
  }
</script>

<script lang="ts">
  // The run's one runState card between the facts line and the steps (ux.md
  // §4.3): the failing step's OWN error, the approval / question / usage
  // limit / verify card with its buttons (so the header has no primary for
  // the same action), the hold card, what the run delivered, or a goal run's
  // criteria. Engine approval gates keep RunSync's preview-then-approve flow.
  import ApprovalActions from '../../../lib/components/ApprovalActions.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { elapsed } from '../../../lib/statusWords';
  import type { RunSync } from '../runSync.svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { extendProblem, extendProposal } from '../goal/goalExtend';
  import { answerRequest, resumeRun, steerRun, verifyCriterion } from './runActions';
  import { isHttpUrl } from '../../../lib/browserDeepLink';
  import { clockTime, deliverablesInOrder, deliverableWord, errorParts } from './runWords';
  import GoalCard from './GoalCard.svelte';

  interface Props {
    run: WorkflowRun;
    runState: RunState;
    sync: RunSync;
    nodeName: (id: string) => string;
    /** Open a step in the pane, on a tab. */
    onopenstep: (nodeId: string, tab?: string) => void;
    criteria?: readonly Criterion[];
  }
  let { run, runState, sync, nodeName, onopenstep, criteria = [] }: Props = $props();

  const kind = $derived(cardKind(run, runState));
  const wait = $derived(oldestWait(run));
  const waitName = $derived(wait ? (wait.name && wait.name !== wait.node_id ? wait.name : nodeName(wait.node_id)) : '');
  let busy = $state<'approve' | 'deny' | null>(null);

  const failure = $derived.by(() => {
    const fs = run.overview?.failed_step;
    const node = run.nodes.find((n) => n.node_id === fs?.node_id) ?? run.nodes.find((n) => n.status === 'error');
    const error = fs?.error ?? node?.error ?? run.error ?? 'The run failed.';
    const child = node?.children?.find((c) => c.status === 'error');
    const ended = node?.started_at && node.duration_ms != null ? new Date(Date.parse(node.started_at) + node.duration_ms).toISOString() : run.finished_at;
    const meta = [node ? nodeName(node.node_id) : fs?.name, child?.label, child ? null : node?.thread?.provider, ended ? `failed at ${clockTime(ended)}` : null];
    return { ...errorParts(error), meta: meta.filter(Boolean).join(' · ') };
  });

  async function decide(approve: boolean, note?: string | null): Promise<void> {
    if (!wait?.thread_id || !wait.request_id) {
      // An engine approval gate (human_approval / a publish gate).
      await sync.approveRun(approve, note);
      return;
    }
    busy = approve ? 'approve' : 'deny';
    await answerRequest(wait, approve, note);
    busy = null;
  }

  const where = $derived.by(() => {
    const s = run.overview?.subject;
    if (s?.pr) return `${s.pr.repo} · PR #${s.pr.number}`;
    if (s?.ticket) return s.ticket.key;
    return s?.repo ?? null;
  });

  // Hold: Extend and resume… (caps +50 %, never at or below what is used).
  let extending = $state(false);
  let iterations = $state(0);
  let minutes = $state(0);
  const extendBlocked = $derived(extendProblem(run.overview?.goal, iterations, minutes));
  function openExtend(): void {
    ({ iterations, minutes } = extendProposal(run.overview?.goal));
    extending = true;
  }
  async function extend(): Promise<void> {
    if (!extendBlocked && (await resumeRun(run.id, { iterations, active_s: minutes * 60 }))) extending = false;
  }

  // Verify: record what you checked; "Not met…" asks for a note.
  let evidence = $state('');
  async function record(met: boolean): Promise<void> {
    if (!wait?.criterion_id) return;
    const note = evidence.trim() || (met ? '' : ((await confirmer.promptText('The next iteration reads your note as the reason.', { title: 'Not met', confirmLabel: 'Record as not met', placeholder: 'What is still wrong' })) ?? '').trim());
    if (!note) return;
    if (await verifyCriterion(run.id, wait.node_id, wait.criterion_id, met, note)) evidence = '';
  }
</script>

{#if kind}
  <section class="scard {kind}" data-testid="wf-run-card" data-kind={kind} aria-label="Run status">
    {#if kind === 'error'}
      <h3 class="title danger"><Icon name="x" size={13} /><span dir="auto">{failure.title}</span></h3>
      {#if failure.body}<p class="body" dir="auto">{failure.body}</p>{/if}
      {#if failure.meta}<p class="meta">{failure.meta}</p>{/if}
    {:else if kind === 'approval'}
      <h3 class="title"><Icon name="shield" size={13} /><span dir="auto">{waitName || nodeName(run.approval_node_id ?? '')} asks to go ahead</span></h3>
      {#if where}<p class="meta">Where: <strong>{where}</strong> · Who sees it: everyone on it</p>{/if}
      {#if wait?.text}<p class="body" dir="auto">{wait.text}</p>{/if}
      {#if sync.approvalNeedsPreview}
        {#if sync.approvalPreviewLoading || sync.approvalPreviewError}
          <LoadState what="the publication preview" variant="compact" loading={sync.approvalPreviewLoading} error={sync.approvalPreviewError} empty onretry={() => sync.retryApprovalPreview()} />
        {:else if sync.approvalPreview}
          <p class="body" dir="auto"><strong>{sync.approvalPreview.request.reviewed_content.title}</strong> — publish via {sync.approvalPreview.account_label}.</p>
        {/if}
      {/if}
      {#if wait}<button type="button" class="link" onclick={() => onopenstep(wait.node_id, 'preview')}>Show all</button>{/if}
      <div class="actions">
        <ApprovalActions
          busy={busy ?? (sync.approving ? sync.approvingKind : null)}
          disabled={!wait?.request_id && sync.approvalBlocked}
          disabledReason="Load and review the publication preview before approving."
          denyTarget="the run at {waitName || 'this step'}"
          denyHint="Denying stops the run; nothing is posted."
          onapprove={() => decide(true)}
          ondeny={(reason) => decide(false, reason)}
        />
      </div>
    {:else if kind === 'question' && wait}
      <h3 class="title"><Icon name="comment" size={13} /><span dir="auto">{waitName} asks:</span></h3>
      <p class="body" dir="auto">{wait.text.split('\n')[0]}</p>
      <div class="actions"><button type="button" class="btn primary" onclick={() => onopenstep(wait.node_id, 'conversation')}>Answer in the conversation</button></div>
    {:else if kind === 'limit' && wait}
      <h3 class="title"><Icon name="clock" size={13} /><span dir="auto">{wait.text || `The usage limit stopped ${waitName}.`}</span></h3>
      {#if wait.resets_at}<p class="body">Otto continues it at {clockTime(wait.resets_at)}, when the limit resets.</p>{/if}
    {:else if kind === 'verify' && wait}
      <h3 class="title"><Icon name="userCheck" size={13} /><span dir="auto">Check that {wait.text} — record what you verified</span></h3>
      <textarea class="input" rows="3" dir="auto" aria-label="What you verified" placeholder="What you checked and what you saw" bind:value={evidence}></textarea>
      <div class="actions">
        <button type="button" class="btn" data-testid="wf-verify-not-met" onclick={() => record(false)}>Not met…</button>
        <button type="button" class="btn primary" data-testid="wf-verify-record" disabled={!evidence.trim()} onclick={() => record(true)}>Record</button>
      </div>
    {:else if kind === 'hold'}
      <h3 class="title"><Icon name="pause" size={13} /><span dir="auto">{run.overview?.hold?.reason ?? run.hold?.reason ?? 'The run is held.'}</span></h3>
      <div class="actions">
        {#if runState === 'exhausted'}
          <button type="button" class="btn primary" data-testid="wf-hold-extend" onclick={openExtend}>Extend and resume…</button>
        {:else}
          <button type="button" class="link" data-testid="wf-hold-note" onclick={() => steerRun(run.id)}>Add a note first</button>
          <button type="button" class="btn primary" data-testid="wf-hold-resume" onclick={() => resumeRun(run.id)}>Resume</button>
        {/if}
      </div>
    {:else if kind === 'delivered'}
      <h3 class="title">Delivered</h3>
      <ul class="deliv">
        {#each deliverablesInOrder(run.overview?.deliverables ?? []) as d, i (i)}
          {@const w = deliverableWord(d)}
          <li class={w.tone}>
            <Icon name={d.state === 'done' ? 'check' : d.state === 'failed' ? 'x' : 'clock'} size={13} />
            <span class="dl" dir="auto">{d.label}</span>
            {#if d.url && isHttpUrl(d.url)}<a href={d.url} target="_blank" rel="noreferrer noopener">{w.word}</a>{:else}<span class="dw">{w.word}</span>{/if}
          </li>
        {/each}
      </ul>
    {:else if kind === 'goal' && run.overview?.goal}
      <GoalCard goal={run.overview.goal} {run} {criteria} />
    {/if}
  </section>
{/if}

{#if extending}
  <Modal title="Extend and resume" onclose={() => (extending = false)} bottom>
    <label class="field">Iterations<input class="input" type="number" min={(run.overview?.goal?.iteration ?? 0) + 1} max="50" bind:value={iterations} /></label>
    <label class="field">Active time (minutes)<input class="input" type="number" min="15" max="480" bind:value={minutes} /></label>
    <p class="meta">New totals for the loop that ran out{run.overview?.goal ? `: it used ${run.overview.goal.iteration} iterations and ${elapsed(run.overview.goal.active_ms)}` : ''}.</p>
    {#if extendBlocked}<p class="meta warn" use:announceShown={{ text: extendBlocked, assertive: true }}>{extendBlocked}</p>{/if}
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => (extending = false)}>Cancel</button>
      <button type="button" class="btn primary" disabled={!!extendBlocked} onclick={extend}>Extend and resume</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .scard {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding: var(--sp-5) var(--sp-6);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  .scard.error {
    border-color: var(--danger);
    background: var(--danger-soft);
  }
  .scard.approval,
  .scard.question,
  .scard.limit,
  .scard.verify,
  .scard.hold {
    border-color: var(--warning);
    background: var(--warning-soft);
  }
  .title {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .title :global(svg) {
    flex: none;
    margin-block-start: var(--sp-1);
  }
  .title.danger {
    color: var(--danger);
  }
  .body {
    margin: 0;
    font-size: var(--fs-m);
    overflow-wrap: anywhere;
  }
  .meta {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-4);
  }
  .link {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .deliv {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--fs-m);
  }
  .deliv li {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding-block: var(--sp-3);
    border-block-start: 1px solid var(--border);
  }
  .deliv li:first-child {
    border-block-start: 0;
  }
  .deliv li.success :global(svg) {
    color: var(--success);
  }
  .deliv li.danger {
    color: var(--danger);
  }
  .deliv li.warning :global(svg) {
    color: var(--warning);
  }
  .dl {
    flex: 1;
    min-inline-size: 0;
  }
  .dw,
  .deliv a {
    flex: none;
    font-size: var(--fs-s);
  }
  .dw {
    color: var(--text-dim);
  }
  .deliv a {
    color: var(--accent-text);
  }
  .meta.warn {
    color: var(--warning);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin-block-end: var(--sp-5);
    font-size: var(--fs-m);
  }
  /* Phone: the card's buttons are full-width halves, 44 px. */
  @media (max-width: 640px) {
    .actions > :global(*),
    .actions :global(.btn) {
      flex: 1;
      min-block-size: 44px;
    }
  }
</style>
