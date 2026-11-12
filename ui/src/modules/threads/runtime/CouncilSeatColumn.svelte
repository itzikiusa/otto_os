<!--
  One council column (a seat or the synthesis): a header with the provider,
  model, effort, status, elapsed time and tokens (tokens, not cost — most
  people run on subscriptions), "Continue here", and the thread itself as a
  compact embed. A seat whose turn failed says so and offers to send the
  question again.
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
  import { council } from './councilStore.svelte';
  import EmbeddedRuntimeThread from './EmbeddedRuntimeThread.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { latestTurn } from './timeline';

  interface Props {
    session: RuntimeSession;
    /** "Seat 2" / "Synthesis". */
    label: string;
    /** The council's question, for Retry on a failed seat. */
    question: string;
    /** Hidden on a phone unless it is the picked seat. */
    active: boolean;
    wide?: boolean;
    onstatus: (status: RuntimeEmbedStatus) => void;
  }
  let { session: s, label, question, active, wide = false, onstatus }: Props = $props();

  let status = $state<RuntimeEmbedStatus>('idle');
  let retrying = $state(false);
  const meta = $derived(councilOf(s));
  const proj = $derived(runtimeViews.projections[s.id] ?? null);
  const stats = $derived(seatStats(proj?.turns ?? [], now()));
  const failed = $derived.by(() => {
    const t = proj ? latestTurn(proj) : null;
    return t?.status === 'failed' || proj?.session?.status === 'error';
  });
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
      await runtimeThreads.prompt(s.id, { text: question, ...(meta?.effort ? { effort: meta.effort } : {}) });
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
    <span class="cs-name ellipsis" title="{label} · {model}"><span class="cs-label">{label}</span> {model}{#if meta?.effort} · {formatEffortShort(meta.effort)}{/if}</span>
    <StatusDot state={dot} />
    {#if stats.elapsedMs !== null}<span class="cs-num" title="Time since this seat started">{formatElapsed(stats.elapsedMs)}</span>{/if}
    {#if stats.tokens > 0}<span class="cs-num" title="Tokens used">{formatTokens(stats.tokens)}</span>{/if}
    <button
      type="button"
      class="btn small ghost"
      title="Open this {label.toLowerCase()} as a normal thread"
      data-testid="council-continue-{s.id}"
      onclick={() => council.continueHere(s.id)}
    >Continue here</button>
  </header>
  {#if failed && meta?.role === 'seat'}
    <p class="cs-failed" use:announceShown={{ text: `${label} didn’t answer` }}>
      <Icon name="warning" size={12} />
      <span>{label} ({harnessLabel(s.provider)}) didn’t answer.</span>
      <button type="button" class="btn small" disabled={retrying || status === 'working'} onclick={() => void retry()}>
        {retrying ? 'Asking…' : 'Ask again'}
      </button>
    </p>
  {/if}
  <div class="cs-body">
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
