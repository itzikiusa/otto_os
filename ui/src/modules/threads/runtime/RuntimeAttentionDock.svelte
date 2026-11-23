<script lang="ts">
  // "The agent is waiting for you", docked directly above the composer: what
  // it wants in one line (who, what, when), and the answer controls right
  // there — Decline / Approve / More for an approval (⌘↵ / ⌘⌫ while the
  // composer is empty or the dock has focus), the question form with Send
  // answer kept in view, or the secret field. Several pending requests step
  // through "1 of 3". The live controls exist only here; the timeline keeps
  // the record ("Waiting for your approval · Answer"), and its Answer brings
  // that request up here. The dock's height is capped to a share of its own
  // pane (a Grid pane, a phone with the keyboard up), never the window, and
  // always leaves the composer room.
  import { announceEdge } from '../../../lib/announce.svelte';
  import type { ApprovalDecision, RuntimeRequest } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import { isMacPlatform } from '../../../lib/keys';
  import { rel } from '../../../lib/stores/now.svelte';
  import type { SecretRequest } from '../../../lib/stores/secretRequests.svelte';
  import {
    attentionHeadline, currentIndex, dockKeyAction, keyDecision, stepId, type AttentionItem,
  } from './attentionDock';
  import RuntimeApprovalRow from './RuntimeApprovalRow.svelte';
  import { isMcpAsk } from './mcpAsks';
  import RuntimeQuestionCard from './RuntimeQuestionCard.svelte';
  import RuntimeSecretCard from './RuntimeSecretCard.svelte';
  import type { PendingApproval } from './timeline';

  interface Props {
    sessionId: string;
    requests: RuntimeRequest[];
    approvals: ReadonlyMap<string, PendingApproval>;
    secret: SecretRequest | null;
    /** "Claude", "Codex" — who asks. */
    agent: string;
    onrespond: (requestId: string, decision: ApprovalDecision) => Promise<void>;
    /** After an answer: focus goes back to the composer. */
    onanswered: () => void;
    /** Scroll the timeline to this request's row. */
    onjump?: (requestId: string) => void;
    /** What waits, in order (attentionDock.ts `attentionItems`; the host
     *  also uses it for the follow pill, the placeholder and Stop's weight). */
    items: AttentionItem[];
  }
  let { sessionId, requests, approvals, secret, agent, onrespond, onanswered, onjump, items: list }: Props = $props();
  let picked = $state<string | null>(null);
  const idx = $derived(currentIndex(list, picked));
  const cur = $derived(idx >= 0 ? list[idx] : null);
  const approval = $derived(cur?.kind === 'approval' ? (approvals.get(cur.id) ?? null) : null);
  const question = $derived(cur?.kind === 'question' ? (requests.find((r) => r.id === cur.id) ?? null) : null);
  const headline = $derived(cur ? attentionHeadline(cur, agent) : '');
  const mac = isMacPlatform();
  let hover = $state(true);
  $effect(() => {
    if (typeof matchMedia !== 'function') return;
    const mq = matchMedia('(hover: hover) and (pointer: fine)');
    hover = mq.matches;
    const on = (): void => void (hover = mq.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  });

  // Screen readers hear a new request once, assertively (it blocks the agent).
  const say = announceEdge();
  $effect(() => {
    const first = list[0];
    say(first ? `${attentionHeadline(first, agent)}${first.kind === 'approval' ? ' — approval needed' : ''}` : '', { assertive: true });
  });

  let root = $state<HTMLDivElement | null>(null);
  let rowRef = $state<{ respond: (d: ApprovalDecision) => Promise<void> } | null>(null);
  let maxH = $state<number | null>(null);

  // Cap: a share of the pane (60 % on touch, 45 % with a pointer), and never
  // more than leaves the composer and a sliver of timeline on screen.
  $effect(() => {
    const el = root;
    const pane = el?.closest<HTMLElement>('.rt');
    if (!el || !pane) return;
    const composer = pane.querySelector<HTMLElement>('.rt-composer');
    const measure = (): void => {
      const share = matchMedia?.('(pointer: coarse)').matches ? 0.6 : 0.45;
      const h = pane.clientHeight;
      const room = h - (composer?.offsetHeight ?? 0) - 48;
      maxH = Math.max(120, Math.round(Math.min(h * share, room)));
    };
    measure();
    // Measured on the next frame: setting the cap inside the observer's own
    // callback would resize what it observes ("ResizeObserver loop").
    let frame = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    });
    ro.observe(pane);
    if (composer) ro.observe(composer);
    // The timeline's "Answer" (RuntimeApprovalRow) picks a request here.
    const onPick = (e: Event): void => {
      picked = (e as CustomEvent<string>).detail;
      void Promise.resolve().then(() => root?.querySelector<HTMLElement>('button:not([disabled]), input, textarea')?.focus());
    };
    pane.addEventListener('rt-attention-pick', onPick);
    // Keys answer from the dock's own buttons (a listener, not a handler on
    // the region: the region itself is not a control).
    el.addEventListener('keydown', onDockKey);
    return () => {
      cancelAnimationFrame(frame);
      ro.disconnect();
      pane.removeEventListener('rt-attention-pick', onPick);
      el.removeEventListener('keydown', onDockKey);
    };
  });

  /** Answer; a keyboard person whose focus was in the dock lands back in the
   *  composer (never on touch: that would open the keyboard). */
  async function answer(requestId: string, decision: ApprovalDecision): Promise<void> {
    const hadFocus = hover && !!root?.contains(document.activeElement);
    await onrespond(requestId, decision);
    if (hadFocus) onanswered();
  }

  function act(action: ReturnType<typeof dockKeyAction>, e: KeyboardEvent): boolean {
    if (!action || !cur) return false;
    e.preventDefault();
    e.stopPropagation();
    if (action === 'next' || action === 'prev') picked = stepId(list, picked, action === 'next' ? 1 : -1);
    else if (action === 'send') root?.querySelector('form')?.requestSubmit();
    else if (approval) {
      const d = keyDecision(approval.options, action);
      if (d) void (rowRef ? rowRef.respond(d) : answer(approval.requestId, d));
    }
    return true;
  }

  /** The composer's keydown, before its own: true when the dock answered. */
  export function onKey(e: KeyboardEvent, composerEmpty: boolean): boolean {
    return act(dockKeyAction(e, { kind: cur?.kind ?? null, composerEmpty, inDock: false, inTextField: true, count: list.length }), e);
  }

  function onDockKey(e: KeyboardEvent): void {
    const t = e.target as HTMLElement | null;
    const inTextField = !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
    act(dockKeyAction(e, { kind: cur?.kind ?? null, composerEmpty: false, inDock: true, inTextField, count: list.length }), e);
  }
</script>

{#if cur}
  {@const curId = cur.id}
  <div
    class="ad"
    class:ad-approval={cur.kind === 'approval'}
    role="region"
    aria-label="Waiting for you"
    data-testid="runtime-attention"
    data-kind={cur.kind}
    data-count={list.length}
    style:max-block-size={maxH ? `${maxH}px` : undefined}
    bind:this={root}
  >
    <div class="ad-head">
      <Icon name={cur.kind === 'approval' ? 'shield' : cur.kind === 'secret' ? 'lock' : 'comment'} size={13} />
      <span class="ad-title ellipsis" dir="auto">
        <strong>{headline}</strong>{#if question}{@const q = question.question_title ?? question.questions?.[0]?.header}{#if q}<span class="ad-what"> · {q}</span>{/if}{/if}
      </span>
      {#if cur.at}<time class="ad-when" datetime={cur.at}>{rel(cur.at)}</time>{/if}
      {#if list.length > 1}
        <span class="ad-step">
          <button type="button" class="icon-btn ad-nav" aria-label="Previous request" title="Previous request" onclick={() => (picked = stepId(list, picked, -1))}><Icon name="chevronLeft" size={12} /></button>
          <span class="ad-count" data-testid="runtime-attention-count">{idx + 1} of {list.length}</span>
          <button type="button" class="icon-btn ad-nav" aria-label="Next request" title="Next request" onclick={() => (picked = stepId(list, picked, 1))}><Icon name="chevronRight" size={12} /></button>
        </span>
      {/if}
      {#if onjump && cur.kind === 'approval' && !isMcpAsk(cur.id)}
        <button type="button" class="btn small ghost ad-jump" title="Show where the agent asked" data-testid="runtime-attention-jump" onclick={() => onjump?.(curId)}>Show</button>
      {/if}
    </div>
    {#key cur.id}
      <div class="ad-body" class:ad-form={cur.kind === 'question'}>
        {#if approval}
          <RuntimeApprovalRow bind:this={rowRef} {approval} onrespond={answer} docked hints={hover} {mac} />
        {:else if question}
          <RuntimeQuestionCard request={question} {sessionId} {agent} {onanswered} docked />
        {:else if cur.kind === 'secret' && secret}
          <RuntimeSecretCard request={secret} docked />
        {/if}
      </div>
    {/key}
  </div>
{/if}

<style>
  .ad {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-inline-size: 0;
    min-block-size: 0;
    padding: 8px 10px;
    border: 1px solid color-mix(in srgb, var(--warning) 45%, var(--border));
    border-radius: var(--radius-m);
    background: color-mix(in srgb, var(--warning-soft) 60%, var(--surface));
    font-size: var(--fs-s);
    overflow: hidden;
  }
  .ad:focus-within {
    outline: 2px solid var(--accent-line);
    outline-offset: 1px;
  }
  .ad-head {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
    min-inline-size: 0;
    color: var(--warning);
  }
  .ad-title {
    flex: 1;
    min-inline-size: 0;
    color: var(--text);
  }
  .ad-what {
    color: var(--text-dim);
  }
  .ad-when,
  .ad-count {
    flex: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .ad-step {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex: none;
  }
  .ad-jump {
    flex: none;
  }
  /* The body never scrolls as a whole: what is long inside it (a reason, a
     question's fields) scrolls, and its answer buttons stay in view. */
  .ad-body {
    display: flex;
    flex-direction: column;
    flex: 0 1 auto;
    min-block-size: 0;
    min-inline-size: 0;
    overflow: hidden;
  }
  .ad-body.ad-form {
    flex: 1 1 auto;
  }
  @media (pointer: coarse) {
    .ad-nav,
    .ad-jump {
      min-block-size: var(--hit-min);
      min-inline-size: var(--hit-min);
    }
  }
</style>
