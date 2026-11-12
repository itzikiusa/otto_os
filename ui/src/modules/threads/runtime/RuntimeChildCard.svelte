<!--
  A thread opened from this one, live in the parent's timeline (T3 Code's
  SubagentTimelineLink in V2LifecycleRow.tsx, MIT, Copyright (c) 2026 T3
  Tools Inc.): provider · model · effort · status · elapsed · its latest
  activity while it works and its answer's first line once it ends, with
  Stop, Open beside and Open. The same card for a child a person asked, one
  the agent opened (`otto.thread_open`), on every runtime provider. It
  follows the child's own session events (runtimeViews keeps its projection
  live while the card is mounted; no polling). Clicking the row shows the
  child's last answer here, read-only.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { formatEffortShort, formatModelLabel } from '../../../lib/providerModels';
  import { harnessLabel } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { toastError } from '../../../lib/toastError';
  import { formatDuration } from './workLog';
  import { activityOf, answerOf, childState, elapsedMs, firstLine, rowTone } from './childThreads';
  import { openChildThread } from './childNav';
  import RuntimeAgentRow from './RuntimeAgentRow.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';

  interface Props {
    id: string;
    /** What the opener said about it (an agent's `thread_open` input), until its row loads. */
    hint?: { provider?: string | null; title?: string | null; model?: string | null; effort?: string | null };
  }
  let { id, hint = {} }: Props = $props();

  let gone = $state(false);
  let open = $state(false);
  let stopping = $state(false);

  $effect(() => {
    const child = id;
    return untrack(() => runtimeViews.open(child));
  });
  $effect(() => {
    const child = id;
    if (untrack(() => runtimeThreads.get(child))) return;
    untrack(() => void runtimeThreads.loadOne(child).catch(() => (gone = true)));
  });

  const s = $derived(runtimeThreads.get(id));
  const p = $derived(runtimeViews.projections[id] ?? null);
  const st = $derived(childState(s, p));
  const provider = $derived(s?.provider ?? hint.provider ?? null);
  const model = $derived.by(() => {
    const m = s?.model ?? hint.model ?? null;
    return m && provider ? formatModelLabel(provider, m, auth.meta) : null;
  });
  const sub = $derived([model, hint.effort ? formatEffortShort(hint.effort) : null].filter(Boolean).join(' · ') || null);
  const name = $derived(s?.title ?? hint.title ?? (provider ? harnessLabel(provider) : 'Thread'));
  const answer = $derived(answerOf(p));
  const live = $derived(st.tone === 'working' || st.tone === 'starting' || st.tone === 'needs-you');
  const line = $derived(gone ? 'This thread was deleted' : live ? activityOf(p) : firstLine(answer) || null);
  const elapsed = $derived.by(() => {
    const ms = elapsedMs(p, now());
    return ms === null ? null : formatDuration(ms);
  });

  async function stop(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await runtimeThreads.interrupt(id);
    } catch (e) {
      toastError('Couldn’t stop that thread', e);
    } finally {
      stopping = false;
    }
  }
</script>

<div class="rc" data-testid="runtime-child-{id}" data-state={st.tone}>
  <RuntimeAgentRow
    {provider}
    {name}
    {sub}
    tone={gone ? 'off' : rowTone(st.tone)}
    status={gone ? 'Deleted' : st.label}
    {elapsed}
    result={line}
    expanded={open}
    ontoggle={answer ? () => (open = !open) : undefined}
    onstop={live && !gone && s?.status !== 'closed' && !stopping ? () => void stop() : undefined}
    onbeside={gone ? undefined : () => openChildThread(id, true)}
    onopen={gone ? undefined : (e) => openChildThread(id, e.shiftKey)}
    openLabel="Open"
    testid="runtime-child-row-{id}"
  />
  {#if open && answer}
    <div class="rc-answer" dir="auto" aria-label="{name}: last answer" data-testid="runtime-child-answer-{id}">
      <Markdown md={answer} />
    </div>
  {/if}
</div>

<style>
  .rc {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .rc-answer {
    max-block-size: 320px;
    overflow-y: auto;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
</style>
