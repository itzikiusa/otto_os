<script lang="ts">
  // The agent's own `otto.thread_open` / `otto.thread_message` call as the
  // same compact row as a subagent (RuntimeAgentRow): the thread it opened or
  // messaged, its provider, model and effort, its live status, the opening
  // prompt / message, and Open (⇧-click: beside). A failed or gated call keeps
  // the ordinary work row, so its error or approval stays in sight.
  import type { TurnItem } from '../../../lib/api/types';
  import { harnessLabel } from '../../../lib/providers';
  import { router } from '../../../lib/router.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { relatedStatus } from '../relatedThreads';
  import { threads } from '../threadsStore.svelte';
  import { elapsedBetween, type ThreadCall } from './agentRows';
  import RuntimeAgentRow from './RuntimeAgentRow.svelte';
  import RuntimeChildCard from './RuntimeChildCard.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let { item, call }: { item: TurnItem; call: ThreadCall } = $props();

  const target = $derived(call.targetId ? runtimeThreads.get(call.targetId) : null);
  const running = $derived(item.status === 'running' || item.status === 'pending');
  const provider = $derived(target?.provider ?? call.provider);
  const name = $derived(`${call.verb} ${target?.title ?? call.title ?? (provider ? `a ${harnessLabel(provider)} thread` : 'a thread')}`);
  const sub = $derived([target?.model ?? call.model, call.effort].filter(Boolean).join(' · ') || null);
  const st = $derived.by(() => {
    if (running) return { tone: 'live' as const, text: call.verb === 'Opened' ? 'Opening' : 'Sending' };
    if (!target) return { tone: 'done' as const, text: call.verb === 'Opened' ? 'Opened' : 'Sent' };
    const s = relatedStatus(target.status);
    return { tone: s.tone === 'live' ? ('live' as const) : s.tone === 'off' ? ('off' as const) : ('done' as const), text: s.text };
  });
  const elapsed = $derived(running ? elapsedBetween(item.started_at, now()) : null);

  function open(e: MouseEvent): void {
    if (!call.targetId) return;
    if (router.module !== 'threads') router.go(`threads/${call.targetId}`);
    else threads.selectThread(call.targetId, { split: e.shiftKey });
  }
</script>

{#if call.verb === 'Opened' && call.targetId && !running}
  <!-- The thread it opened, live: the same card as a child a person asked. -->
  <RuntimeChildCard id={call.targetId} hint={{ provider: call.provider, title: call.title, model: call.model, effort: call.effort }} />
{:else}
<RuntimeAgentRow
  {provider}
  {name}
  {sub}
  tone={st.tone}
  status={st.text}
  {elapsed}
  result={call.text}
  onopen={call.targetId ? open : undefined}
  openLabel="Open thread"
  testid="runtime-thread-call-{item.id}"
/>
{/if}
