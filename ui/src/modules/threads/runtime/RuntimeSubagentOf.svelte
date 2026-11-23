<!--
  The first line inside a subagent (Orca's "Task from coordinator", T3
  Code's ProviderSubagentBar "Open parent"): "Subagent of <parent>", its
  task in one line, and Open parent. Fixed — shown whether or not the parent
  is on screen.
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import { threads } from '../threadsStore.svelte';
  import { childOf, firstLine } from './childThreads';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { messageAttribution } from './timeline';

  let { threadId }: { threadId: string } = $props();

  const of = $derived(childOf(runtimeThreads.get(threadId)));
  const parent = $derived(of ? runtimeThreads.get(of.id) : null);
  /** Its task: the first message, without the delegation header. */
  const task = $derived.by(() => {
    const first = runtimeViews.projections[threadId]?.items.find((i) => i.type === 'user_message');
    if (first?.type !== 'user_message') return '';
    return firstLine(messageAttribution(first.text)?.body ?? first.text);
  });
</script>

{#if of}
  <div class="so" data-testid="runtime-subagent-of">
    <Icon name="people" size={13} />
    <span class="so-text ellipsis" dir="auto">
      Subagent of <strong>{parent?.title ?? 'a closed thread'}</strong>{#if task}<span class="so-task"> · {task}</span>{/if}
    </span>
    {#if parent}
      {@const pid = parent.id}
      <button type="button" class="btn small ghost so-open" data-testid="threads-back-to-parent" onclick={() => threads.selectThread(pid)}>Open parent</button>
    {/if}
  </div>
{/if}

<style>
  .so {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .so-text {
    flex: 1;
    min-inline-size: 0;
  }
  .so-text strong {
    color: var(--text);
    font-weight: 600;
  }
  @media (pointer: coarse) {
    .so-open {
      min-block-size: 44px;
    }
  }
</style>
