<!--
  A thread opened from this one, live in the parent's timeline (T3 Code's
  SubagentTimelineLink in V2LifecycleRow.tsx, MIT, Copyright (c) 2026 T3
  Tools Inc.): provider · model · effort · status · elapsed · its latest
  activity while it works and its answer's first line once it ends, with
  Stop and Open (a tab in the right panel; ⌘-click: the main view). The same card for a child a person asked, one
  the agent opened (`otto.thread_open`), on every runtime provider. It
  follows the child's own session events (runtimeViews keeps its projection
  live while the card is mounted; no polling). Clicking the row shows the
  child's last answer here, read-only. Stop stops what the child actually
  runs (its turn, else its background work) and says what happened. A child
  a PERSON asked hands its answer to them, not to this thread's agent: once
  it answered, "Send the answer to <agent>" passes it on in one click.
-->
<script lang="ts">
  import { getContext, untrack } from 'svelte';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { formatEffortShort, formatModelLabel } from '../../../lib/providerModels';
  import { harnessLabel } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { toastError } from '../../../lib/toastError';
  import { formatDuration } from './workLog';
  import { toasts } from '../../../lib/toast.svelte';
  import { activityOf, answerOf, childOf, childState, elapsedMs, firstLine, removedLine, removedState, rowTone, type RemovedKid } from './childThreads';
  import { answerHandoff } from './askAgents';
  import { stopThreadWork } from './taskStops.svelte';
  import { mainViewKey, OPEN_CHILD_CTX, openChildThread, READ_ONLY_CTX } from './childNav';
  import RuntimeAgentRow from './RuntimeAgentRow.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { threadMeta } from '../threadMeta.svelte';

  interface Props {
    id: string;
    /** What the opener said about it (an agent's `thread_open` input), until its row loads. */
    hint?: { provider?: string | null; title?: string | null; model?: string | null; effort?: string | null };
    /** Its parent's agent removed it: what it left in the parent's timeline (no live row to follow). */
    removed?: RemovedKid;
  }
  let { id, hint = {}, removed }: Props = $props();
  /** In a read-only host (a settled workflow step) the card doesn't stop or hand over;
   *  when the host provides `OPEN_CHILD_CTX`, clicking still opens the sub-thread in-pane. */
  const readOnly = getContext<(() => boolean) | undefined>(READ_ONLY_CTX);
  const openChild = getContext<((id: string, modifier: boolean) => void) | undefined>(OPEN_CHILD_CTX);

  let gone = $state(false);
  let open = $state(false);
  let stopping = $state(false);

  $effect(() => {
    const child = id;
    if (removed) return;
    return untrack(() => runtimeViews.open(child));
  });
  $effect(() => {
    const child = id;
    if (removed || untrack(() => runtimeThreads.get(child))) return;
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
  const live = $derived(st.tone === 'working' || st.tone === 'starting' || st.tone === 'background' || st.tone === 'needs-you');
  /** Asked by a person: its answer is the person's to pass on (the agent path wakes the parent itself). */
  const parent = $derived(childOf(s));
  const parentRow = $derived(parent?.by === 'person' ? runtimeThreads.get(parent.id) : null);
  const parentAgent = $derived(parentRow ? harnessLabel(parentRow.provider).replace(/ Code$/, '') : null);
  let handing = $state(false);
  let handed = $state(false);
  const line = $derived(gone ? 'This thread was deleted' : live ? activityOf(p) : firstLine(answer) || null);
  const elapsed = $derived.by(() => {
    const ms = elapsedMs(p, now());
    return ms === null ? null : formatDuration(ms);
  });

  async function stop(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      // A child busy only on background work has no turn to interrupt.
      await stopThreadWork(id, { turnRunning: st.tone !== 'background', taskIds: (s?.background_tasks ?? []).map((b) => b.id) });
    } catch (e) {
      toastError('Couldn’t stop that thread', e);
    } finally {
      stopping = false;
    }
  }

  async function handOver(): Promise<void> {
    if (!parentRow || handing || !answer) return;
    handing = true;
    try {
      await runtimeThreads.prompt(parentRow.id, { text: answerHandoff(name, answer), delivery: 'queue' });
      handed = true;
      toasts.success(`Sent the answer to ${parentAgent}`);
    } catch (e) {
      toastError(`Couldn’t send the answer to ${parentAgent}`, e);
    } finally {
      handing = false;
    }
  }
</script>

{#if removed}
  {@const rs = removedState(removed)}
  <div class="rc" data-testid="runtime-child-{id}" data-state={rs.tone}>
    <RuntimeAgentRow {provider} name={hint.title ?? removed.title} {sub} tone={rowTone(rs.tone)} status={rs.label} elapsed={null} result={removedLine(removed)} expanded={false} testid="runtime-child-row-{id}" />
  </div>
{:else}
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
    ontoggle={answer ? () => { open = !open; if (open) threadMeta.markSeen(id); } : undefined}
    onstop={live && !gone && s?.status !== 'closed' && !stopping && !readOnly?.() ? () => void stop() : undefined}
    onrow={gone || (!openChild && readOnly?.()) ? undefined : (e) => (openChild ? openChild(id, mainViewKey(e)) : openChildThread(id, mainViewKey(e)))}
    testid="runtime-child-row-{id}"
  />
  {#if parentRow && parentRow.status !== 'closed' && answer && !live && !gone && !readOnly?.()}
    <button
      type="button"
      class="btn small ghost rc-hand"
      disabled={handing || handed}
      title="This answer came back to you, not to {parentAgent}. Send it to {parentAgent} as your next message."
      data-testid="runtime-child-handover-{id}"
      onclick={() => void handOver()}
    >
      {handed ? `Sent to ${parentAgent}` : handing ? 'Sending…' : `Send the answer to ${parentAgent}`}
    </button>
  {/if}
  {#if open && answer}
    <div class="rc-answer" dir="auto" aria-label="{name}: last answer" data-testid="runtime-child-answer-{id}">
      <Markdown md={answer} />
    </div>
  {/if}
</div>
{/if}

<style>
  .rc {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .rc-hand {
    align-self: flex-start;
  }
  /* The settled answer reads as prose under its row, not as a second box. */
  .rc-answer {
    padding-block: 2px 10px;
    padding-inline: 36px 8px;
    color: var(--text);
    overflow-wrap: anywhere;
  }
</style>
