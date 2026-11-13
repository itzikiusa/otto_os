<!--
  One council column (a seat or the synthesis): a header with the provider,
  model, effort, status, elapsed time and tokens (tokens, not cost — most
  people run on subscriptions), "Continue here", and the thread itself as a
  compact embed. With rounds, the header shows the seat's state and time in
  the CURRENT round; clicking its name aims the next follow-up at this seat
  only, and "Ask the others" drafts a follow-up quoting its answer. A seat
  that did not finish the round (failed, stopped, cut off by a restart) says
  so and offers to send the round's question again.
-->
<script lang="ts">
  import { announceShown } from '../../../lib/announce.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import StatusDot from '../../../lib/components/StatusDot.svelte';
  import type { RuntimeSession } from '../../../lib/api/types';
  import type { RuntimeEmbedStatus } from '../../../lib/runtimeThreadPort';
  import { formatEffortShort, formatModelLabel } from '../../../lib/providerModels';
  import { harnessLabel } from '../../../lib/providers';
  import { exitState, sessionState } from '../../../lib/status';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { toastError } from '../../../lib/toastError';
  import { councilOf, formatElapsed, formatTokens, seatStats } from './council';
  import { judgePromptSummary, roundStateLabel, type SeatRound } from './councilRounds';
  import { council } from './councilStore.svelte';
  import EmbeddedRuntimeThread from './EmbeddedRuntimeThread.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { latestTurn } from './timeline';

  interface Props {
    session: RuntimeSession;
    /** "Seat 2" / "Synthesis". */
    label: string;
    /** The current round's question, for "Ask again" on a seat that did not finish. */
    question: string;
    /** Hidden on a phone unless it is the picked seat. */
    active: boolean;
    wide?: boolean;
    onstatus: (status: RuntimeEmbedStatus) => void;
    /** The current round's number and this seat's place in it (seats only). */
    roundN?: number;
    round?: SeatRound | null;
    /** The next follow-up goes to this seat only. */
    targeted?: boolean;
    /** Aim the next follow-up at this seat only. */
    ontarget?: () => void;
    /** Draft "Ask the others" with this seat's answer (absent = a lone seat). */
    onaskothers?: (answer: string) => void;
  }
  let { session: s, label, question, active, wide = false, onstatus, roundN = 1, round = null, targeted = false, ontarget, onaskothers }: Props = $props();

  let status = $state<RuntimeEmbedStatus>('idle');
  let retrying = $state(false);
  const meta = $derived(councilOf(s));
  const proj = $derived(runtimeViews.projections[s.id] ?? null);
  const stats = $derived(seatStats(proj?.turns ?? [], now()));
  const failed = $derived.by(() => {
    const t = proj ? latestTurn(proj) : null;
    return t?.status === 'failed' || proj?.session?.status === 'error';
  });
  /** The seat did not finish the current round (a seat, not the judge;
   *  never while its timeline is still loading). */
  const unfinished = $derived(
    meta?.role === 'seat' &&
      !!proj &&
      !!question &&
      (round ? round.state === 'failed' || round.state === 'interrupted' || round.state === 'missing' : failed),
  );
  const elapsed = $derived(round ? round.elapsedMs : stats.elapsedMs);
  /** The judge's prompt (a synthesis's first message): daemon plumbing, not
   *  the person's words — folded to one line so the column opens on the
   *  verdict; "Show prompt" unfolds it. */
  const judgePrompt = $derived.by(() => {
    if (meta?.role !== 'synthesis') return null;
    const first = proj?.items.find((i) => !i.parent_item_id && i.type === 'user_message');
    if (!first || first.type !== 'user_message') return null;
    const summary = judgePromptSummary(first.text);
    return summary ? { summary, text: first.text } : null;
  });
  let promptOpen = $state(false);
  const model = $derived(s.model ? formatModelLabel(s.provider, s.model, auth.meta) : harnessLabel(s.provider));
  const dot = $derived.by(() => {
    if (status === 'needs-you') return sessionState(null, 'idle', true);
    if (status === 'working') return sessionState(null, 'working');
    if (status === 'closed') return exitState(null, false);
    if (failed) return { ...exitState(1, false), label: 'Failed', hint: 'Its turn failed' };
    return sessionState(null, 'idle');
  });

  async function retry(): Promise<void> {
    if (retrying || !question) return;
    retrying = true;
    try {
      // Queued behind a turn still running, like any message sent mid-turn.
      await runtimeThreads.prompt(s.id, { text: question, delivery: 'queue', ...(meta?.effort ? { effort: meta.effort } : {}) });
    } catch (e) {
      toastError(`Couldn’t ask ${label.toLowerCase()} again`, e);
    } finally {
      retrying = false;
    }
  }
</script>

<section class="cs-col" class:active class:wide aria-label="{label}: {model}" data-testid="council-column-{meta?.role === 'synthesis' ? 'synthesis' : meta?.seat}">
  <header class="cs-head">
    {#if hasProviderIcon(s.provider)}<ProviderIcon provider={s.provider} size={14} />{:else}<Icon name="terminal" size={14} />{/if}
    <span class="cs-name ellipsis" title="{label} · {model}">{#if ontarget}<button
        type="button"
        class="cs-label cs-target"
        aria-pressed={targeted}
        title="Ask only {label.toLowerCase()} next"
        data-testid="council-target-seat-{meta?.seat}"
        onclick={ontarget}
      >{label}</button>{:else}<span class="cs-label">{label}</span>{/if} {model}{#if meta?.effort} · {formatEffortShort(meta.effort)}{/if}</span>
    <StatusDot state={dot} />
    {#if round && roundN > 1}<span class="cs-num" title="This seat in round {roundN}" data-testid="council-seat-round-{meta?.seat}">R{roundN} · {roundStateLabel(round.state)}</span>{/if}
    {#if elapsed !== null}<span class="cs-num" title={round ? `Time on round ${roundN}` : 'Time since this seat started'}>{formatElapsed(elapsed)}</span>{/if}
    {#if stats.tokens > 0}<span class="cs-num" title="Tokens used">{formatTokens(stats.tokens)}</span>{/if}
    {#if onaskothers && round?.state === 'done' && round.answer.trim()}
      <button
        type="button"
        class="btn small ghost"
        title="Ask the other seats where they disagree with this answer (you can edit it first)"
        data-testid="council-ask-others-{meta?.seat}"
        onclick={() => onaskothers?.(round?.answer ?? '')}
      >Ask the others</button>
    {/if}
    <button
      type="button"
      class="btn small ghost"
      title="Open this {label.toLowerCase()} as a normal thread"
      data-testid="council-continue-{s.id}"
      onclick={() => council.continueHere(s.id)}
    >Continue here</button>
  </header>
  {#if unfinished && status !== 'working'}
    <p class="cs-failed" data-testid="council-unfinished-{meta?.seat}" use:announceShown={{ text: `${label} didn’t answer` }}>
      <Icon name="warning" size={12} />
      <span>{label} ({harnessLabel(s.provider)}) didn’t {roundN > 1 ? `finish round ${roundN}` : 'answer'}{round?.state === 'interrupted' ? ' — it was stopped' : ''}.</span>
      <button type="button" class="btn small" disabled={retrying} onclick={() => void retry()}>
        {retrying ? 'Asking…' : 'Ask again'}
      </button>
    </p>
  {/if}
  {#if judgePrompt}
    <div class="cs-prompt" data-testid="council-judge-prompt">
      <span class="ellipsis">{judgePrompt.summary}</span>
      <button
        type="button"
        class="btn small ghost"
        aria-expanded={promptOpen}
        aria-controls="cs-prompt-{s.id}"
        data-testid="council-judge-prompt-toggle"
        onclick={() => (promptOpen = !promptOpen)}
      >{promptOpen ? 'Hide prompt' : 'Show prompt'}</button>
    </div>
    {#if promptOpen}
      <!-- A read-only text area: natively focusable and scrollable from the keyboard. -->
      <textarea class="cs-prompt-text" id="cs-prompt-{s.id}" dir="auto" readonly rows="10" aria-label="The prompt sent to the judge" data-testid="council-judge-prompt-text" value={judgePrompt.text}></textarea>
    {/if}
  {/if}
  <div class="cs-body" class:judge={!!judgePrompt}>
    <EmbeddedRuntimeThread
      sessionId={s.id}
      workspaceId={s.workspace_id}
      compact
      onstatus={(st) => {
        status = st;
        onstatus(st);
      }}
    />
  </div>
</section>

<style>
  .cs-col {
    display: flex;
    flex-direction: column;
    min-inline-size: 280px;
    min-block-size: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow: hidden;
  }
  .cs-col.wide {
    min-inline-size: 0;
  }
  .cs-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    padding: 6px 8px;
    border-block-end: 1px solid var(--border);
    font-size: var(--fs-s);
    color: var(--text-dim);
    flex: none;
  }
  .cs-name {
    flex: 1;
    min-inline-size: 0;
    color: var(--text);
  }
  .cs-label {
    font-weight: 600;
  }
  .cs-target {
    padding: 0 2px;
    border: 0;
    border-radius: var(--radius-s);
    background: none;
    color: inherit;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  .cs-target:hover,
  .cs-target[aria-pressed='true'] {
    color: var(--accent-text);
    text-decoration: underline;
  }
  .cs-num {
    flex: none;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .cs-failed {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 6px 8px;
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .cs-failed span {
    flex: 1;
    min-inline-size: 0;
  }
  .cs-body {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
  }
  /* The thread's first message is the folded judge prompt above: hide its
     bubble in the column (later messages of the person show as usual). */
  .cs-body.judge :global(.rt-col > .rt-user) {
    display: none;
  }
  .cs-body.judge :global(.rt-col > .rt-user ~ .rt-user) {
    display: flex;
  }
  .cs-prompt {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    padding: 4px 8px;
    border-block-end: 1px solid var(--border);
    font-size: var(--fs-s);
    color: var(--text-dim);
    flex: none;
  }
  .cs-prompt > span {
    flex: 1;
    min-inline-size: 0;
  }
  .cs-prompt-text {
    flex: none;
    box-sizing: border-box;
    inline-size: 100%;
    margin: 0;
    max-block-size: 40vh;
    overflow: auto;
    resize: none;
    padding: 8px;
    border: 0;
    border-block-end: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  @media (max-width: 640px) {
    .cs-col {
      min-inline-size: 0;
    }
    .cs-col:not(.active) {
      display: none;
    }
    .cs-head {
      flex-wrap: wrap;
    }
  }
</style>
