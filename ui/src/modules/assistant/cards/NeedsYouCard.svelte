<script lang="ts">
  // One card for every "needs you" item (patterns §5), in the thread and on
  // the Tasks tab:
  //  • approval — who asked, WHERE it goes, WHO sees it, WHY, and a preview of
  //    exactly WHAT is sent; the primary names the action, Deny asks for an
  //    optional reason, and "Always allow" is opt-in, scoped to this
  //    destination and never offered for purchases or prod.
  //  • question — the agent's question with its options (or a free answer).
  //  • limit    — "Claude limit reached until 14:00 — continue on Codex?"
  //  • takeover — you have control; Hand back resumes the agent.
  //  • memory   — a suggested memory to accept or reject.
  // A decided card stays in the thread with its outcome.
  import ActionCard from './ActionCard.svelte';
  import StatePill from './StatePill.svelte';
  import DenySheet from './DenySheet.svelte';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { limitNotice, providerLabel, providerName, TASK_KIND } from '../model';
  import { clock } from '../format';
  import type { AssistantApprovalCard, AssistantDecisionReq, AssistantNeedsYou, AssistantTask, AssistantTaskAction } from '../../../lib/api/types';

  interface Props {
    task: AssistantTask;
    /** Tighter card (disclosed preview) — the Tasks tab lists many. */
    compact?: boolean;
  }
  let { task, compact = false }: Props = $props();

  // The payload can be cleared once decided; keep the last one we saw so the
  // card still shows what was approved.
  let lastNeeds = $state<AssistantNeedsYou | null>(null);
  $effect(() => {
    if (task.needs_you) lastNeeds = task.needs_you;
  });
  const ny = $derived(task.needs_you ?? lastNeeds);
  const pending = $derived(task.state === 'needs_you');
  const kind = $derived(ny?.kind ?? (task.kind === 'memory_review' ? 'memory' : task.kind));
  const a = $derived<AssistantApprovalCard | null>(ny?.approval ?? null);

  const VERB: Record<AssistantApprovalCard['category'], string> = {
    send: 'Send',
    post: 'Post',
    publish: 'Publish',
    purchase: 'Buy',
    delete: 'Delete',
    submit: 'Submit',
    prod: 'Run on prod',
    other: 'Approve',
  };
  const verb = $derived(a ? VERB[a.category] ?? 'Approve' : 'Approve');
  // Never for purchases or prod, whatever the payload says.
  const canAlwaysAllow = $derived(!!a?.always_allow_allowed && a.category !== 'purchase' && a.category !== 'prod');

  const ICON: Record<string, IconName> = { approval: 'hand', question: 'comment', limit: 'warning', takeover: 'cursor', memory: 'book' };
  const TITLE: Record<string, string> = { approval: 'Approval needed', question: 'Otto asks', limit: 'Usage limit', takeover: 'You have control', memory: 'Suggested memory' };
  const headKind = $derived(pending ? (TITLE[kind] ?? 'Needs you') : (TASK_KIND[task.kind] ?? 'Request'));

  const decision = $derived(typeof task.result?.decision === 'string' ? (task.result.decision as string) : null);
  const outcome = $derived.by(() => {
    if (pending) return null;
    const by = typeof task.result?.decided_by === 'string' ? ` by ${task.result.decided_by}` : ' by you';
    // Outcome verbs match the buttons that decided them (Skip… / Reject / Keep).
    if (decision === 'denied' || task.state === 'cancelled')
      return { tone: 'neutral' as const, label: kind === 'question' ? `Skipped${by}` : kind === 'memory' ? `Rejected${by}` : `Denied${by}` };
    if (decision === 'approved' || task.state === 'done' || task.state === 'running')
      return { tone: 'ok' as const, label: kind === 'approval' ? `Approved${by}` : kind === 'memory' ? `Kept${by}` : 'Answered' };
    if (task.state === 'failed') return { tone: 'bad' as const, label: 'Failed' };
    return { tone: 'neutral' as const, label: 'Closed' };
  });

  let always = $state(false);
  let answer = $state('');
  let busy = $state<string | null>(null);
  let denying = $state(false);

  async function run(action: AssistantTaskAction, body: AssistantDecisionReq, label: string): Promise<void> {
    busy = label;
    try {
      await assistant.act(task.id, action, body);
    } catch (e) {
      toasts.error(action === 'deny' ? 'Couldn’t deny the request' : `Couldn’t ${label.toLowerCase()}`, describeError(e));
    } finally {
      busy = null;
    }
  }
  function deny(reason: string | null): void {
    denying = false;
    void run('deny', reason ? { reason } : {}, 'deny');
  }
  // Keyboard users can scroll long payloads; this is a named scroll region.
  function focusablePreview(node: HTMLElement): void { node.tabIndex = 0; }
  const suggestion = $derived(ny?.suggestion?.provider ?? null);
</script>

<ActionCard icon={ICON[kind] ?? 'hand'} kind={headKind} summary={pending ? (kind === 'approval' ? ny?.prompt || task.title : task.title) : task.title} attention={pending} testid={`card-needs-${kind}`}>
  {#snippet pill()}
    {#if pending}
      <StatePill tone="warn" label="Needs you" />
    {:else if outcome}
      <StatePill tone={outcome.tone} label={outcome.label} />
    {/if}
  {/snippet}

  <p class="asker">
    <Icon name="assistant" size={12} />
    <span>Asked by <strong>Otto</strong></span>
    <span class="dim">· <time datetime={task.created_at} title={new Date(task.created_at).toLocaleString()}>{rel(task.created_at)}</time></span>
  </p>

  {#if kind === 'approval' && a}
    <dl class="kv">
      <dt>Where</dt>
      <dd>{a.where}</dd>
      <dt>Who sees</dt>
      <dd>{a.who_sees}</dd>
      <dt>Why</dt>
      <dd>{a.reason}</dd>
    </dl>
    {#if a.what}
      {#if compact}
        <details class="payload">
          <summary>Review exactly what is sent</summary>
          <div class="preview" role="region" use:focusablePreview aria-label="Exactly what is sent">{a.what}</div>
        </details>
      {:else}
        <div class="preview" role="region" use:focusablePreview aria-label="Exactly what is sent">{a.what}</div>
      {/if}
    {/if}
  {:else if kind === 'limit'}
    <p class="q">{limitNotice(ny?.limit?.provider ?? 'claude', ny?.limit?.until ?? null, pending ? suggestion : null, clock)}</p>
    {#if ny?.limit?.message}<p class="dim small">{ny.limit.message}</p>{/if}
  {:else if kind === 'takeover'}
    <p class="q">{pending ? 'Otto is paused while you use it. Hand it back when you’re done.' : ny?.prompt || task.title}</p>
  {:else}
    <p class="q">{ny?.prompt || task.detail || task.title}</p>
    {#if kind === 'question' && pending}
      {#if ny?.options?.length}
        <div class="opts" role="group" aria-label="Answers">
          {#each ny.options as o (o)}
            <button class="btn small" onclick={() => void run('approve', { answer: o }, 'answer')} disabled={busy !== null}>{o}</button>
          {/each}
        </div>
      {:else}
        <form class="answer" onsubmit={(e) => { e.preventDefault(); if (answer.trim()) void run('approve', { answer: answer.trim() }, 'answer'); }}>
          <label class="sr-only" for={`answer-${task.id}`}>Your answer</label>
          <input id={`answer-${task.id}`} class="input" bind:value={answer} placeholder="Your answer" />
          <button class="btn small" type="submit" disabled={!answer.trim() || busy !== null}>Answer</button>
        </form>
      {/if}
    {/if}
  {/if}
  {#if !pending && typeof task.result?.reason === 'string'}
    <p class="dim small">Reason: {task.result.reason as string}</p>
  {/if}

  {#snippet footer()}
    {#if pending}
      {#if kind === 'approval'}
        {#if canAlwaysAllow}
          <label class="always checkbox-row">
            <input type="checkbox" bind:checked={always} />
            Always allow for {a?.destination ?? a?.where}
          </label>
        {/if}
        <button class="btn small" onclick={() => (denying = true)} disabled={busy !== null}>{busy === 'deny' ? 'Denying…' : 'Deny…'}</button>
        <button class="btn small primary" onclick={() => void run('approve', canAlwaysAllow && always ? { always_allow: true } : {}, verb)} disabled={busy !== null}>{busy === verb ? 'Working…' : verb}</button>
      {:else if kind === 'limit'}
        <button class="btn small ghost" onclick={() => void run('deny', {}, 'wait')} disabled={busy !== null} title="Stay on this provider and wait for the reset">Wait</button>
        {#if suggestion}
          <button class="btn small primary" onclick={() => void run('approve', { provider: suggestion }, 'switch')} disabled={busy !== null}>
            {busy === 'switch' ? 'Switching…' : `Continue on ${providerLabel(suggestion, ny?.suggestion?.model ?? null)}`}
          </button>
        {/if}
      {:else if kind === 'takeover'}
        <button class="btn small primary" onclick={() => void run('handback', {}, 'hand back')} disabled={busy !== null}>{busy ? 'Handing back…' : 'Hand back'}</button>
      {:else if kind === 'memory'}
        <button class="btn small" onclick={() => void run('deny', {}, 'reject')} disabled={busy !== null}>Reject</button>
        <button class="btn small primary" onclick={() => void run('approve', {}, 'accept')} disabled={busy !== null}>Keep</button>
      {:else if kind === 'question'}
        <button class="btn small ghost" onclick={() => (denying = true)} disabled={busy !== null}>Skip…</button>
      {/if}
    {/if}
  {/snippet}
</ActionCard>
{#if denying}
  {#if kind === 'question'}
    <DenySheet
      action="answer"
      title="Skip question"
      confirmLabel="Skip"
      hint="Otto gets no answer to this question. It sees your reason and can carry on another way."
      onclose={() => (denying = false)}
      ondeny={deny}
    />
  {:else}
    <DenySheet action={verb} onclose={() => (denying = false)} ondeny={deny} />
  {/if}
{/if}

<style>
  .asker {
    margin: 0 0 8px;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    color: var(--text-dim);
  }
  .asker strong {
    color: var(--text);
    font-weight: 600;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
    margin: 6px 0 0;
  }
  .kv {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 4px 16px;
    margin: 0;
  }
  .kv dt {
    color: var(--text-dim);
  }
  .kv dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .preview {
    margin-top: 10px;
    padding: 8px 12px;
    border-inline-start: 3px solid var(--border-strong);
    border-radius: 0 var(--radius-m) var(--radius-m) 0;
    background: var(--surface-2);
    white-space: pre-line;
    overflow-wrap: anywhere;
    max-height: 240px;
    overflow-y: auto;
  }
  :global([dir='rtl']) .preview {
    border-radius: var(--radius-m) 0 0 var(--radius-m);
  }
  .payload { margin-top: 10px; }
  .payload summary { cursor: pointer; color: var(--accent-text); }
  .q {
    margin: 0;
    white-space: pre-line;
  }
  .opts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 8px;
  }
  .answer {
    display: flex;
    gap: 6px;
    margin-top: 8px;
  }
  .answer .input {
    flex: 1;
    min-width: 0;
  }
  .always {
    margin-inline-end: auto;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
