<!--
  Linked threads: the group an agent built with `otto.thread_open` (an
  orchestrator and the workers it opened), one click apart. Hidden for an
  unlinked thread. Click switches; ⇧-click opens beside. Hosted outside
  Threads (a personal agent's chat), a click switches the host's view
  (`onswitch`) and ⇧-click opens the thread in Threads.
-->
<script lang="ts">
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { harnessLabel } from '../../../lib/providers';
  import { router } from '../../../lib/router.svelte';
  import { threads } from '../threadsStore.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let { sessionId, onswitch }: { sessionId: string; onswitch?: (id: string) => void } = $props();
  const linked = $derived(runtimeThreads.group(sessionId));

  function open(id: string, beside: boolean): void {
    if (onswitch && !beside) onswitch(id);
    else if (router.module !== 'threads') router.go(`threads/${id}`);
    else threads.selectThread(id, { split: beside });
  }
</script>

{#if linked.length > 1}
  <nav class="rt-linked" aria-label="Linked threads" data-testid="runtime-linked">
    {#each linked as t (t.id)}
      {@const current = t.id === sessionId}
      <button
        type="button"
        class="rt-link"
        class:current
        aria-current={current ? 'page' : undefined}
        title="{t.title} — {harnessLabel(t.provider)}{current ? '' : onswitch ? ' (⇧-click: open in Threads)' : ' (⇧-click: open beside)'}"
        onclick={(e) => !current && open(t.id, e.shiftKey)}
      >
        <span class="rt-link-dot" data-status={t.status} aria-hidden="true"></span>
        {#if hasProviderIcon(t.provider)}<ProviderIcon provider={t.provider} size={12} />{/if}
        <span class="ellipsis" dir="auto">{t.title}</span>
      </button>
    {/each}
  </nav>
{/if}

<style>
  .rt-linked {
    display: flex;
    gap: 6px;
    padding: 6px 12px;
    overflow-x: auto;
    border-block-end: 1px solid var(--border);
    background: var(--bg);
    flex: none;
  }
  .rt-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 220px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-s);
    cursor: pointer;
    flex: none;
  }
  .rt-link:hover {
    color: var(--text);
  }
  .rt-link.current {
    border-color: var(--accent-solid);
    background: var(--accent-soft);
    color: var(--text);
    cursor: default;
  }
  .rt-link:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  /* Phone: a linked thread is a primary hop — a full 44 px tap target. */
  @media (pointer: coarse) {
    .rt-link {
      min-block-size: calc(var(--hit-min) + 8px);
    }
  }
  .rt-link-dot {
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex: none;
  }
  .rt-link-dot[data-status='working'],
  .rt-link-dot[data-status='starting'] {
    background: var(--info);
  }
  .rt-link-dot[data-status='awaiting_permission'] {
    background: var(--warning);
  }
  .rt-link-dot[data-status='idle'] {
    background: var(--success);
  }
</style>
