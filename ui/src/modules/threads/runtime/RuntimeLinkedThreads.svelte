<!--
  Linked threads: the group an agent built with `otto.thread_open` (an
  orchestrator and the workers it opened), one click apart. Hidden for an
  unlinked thread. Click switches; ⇧-click opens beside.
-->
<script lang="ts">
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { harnessLabel } from '../../../lib/providers';
  import { threads } from '../threadsStore.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let { sessionId }: { sessionId: string } = $props();
  const linked = $derived(runtimeThreads.group(sessionId));
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
        title="{t.title} — {harnessLabel(t.provider)}{current ? '' : ' (⇧-click: open beside)'}"
        onclick={(e) => !current && threads.selectThread(t.id, { split: e.shiftKey })}
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
