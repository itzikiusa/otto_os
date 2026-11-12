<!--
  Linked threads in an embed (Personal Agents' chat, a PR, Connections'
  diagnose): the group an agent built with `otto.thread_open`, as one small
  chip that shows only while the thread has linked threads. It opens a
  popover listing them with their status: a click switches the host's view
  (`onswitch`), "Open in Threads" opens one there. Inside Threads the
  header's Related threads chip does this job (RelatedThreadsChip.svelte).
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { harnessLabel } from '../../../lib/providers';
  import { router } from '../../../lib/router.svelte';
  import { relatedStatus } from '../relatedThreads';
  import { sameCouncil } from './council';
  import ThreadPopover from '../ThreadPopover.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let { sessionId, onswitch }: { sessionId: string; onswitch?: (id: string) => void } = $props();
  // A council seat's group is its council: the other seats sit beside it in
  // their own columns, so only threads an agent opened count as linked.
  const linked = $derived.by(() => {
    const self = runtimeThreads.get(sessionId);
    return runtimeThreads.group(sessionId).filter((t) => t.id === sessionId || !sameCouncil(self, t));
  });
  let open = $state(false);
  let btn = $state<HTMLButtonElement | null>(null);

  function pick(id: string, inThreads: boolean): void {
    open = false;
    if (onswitch && !inThreads) onswitch(id);
    else router.go(`threads/${id}`);
  }
</script>

{#if linked.length > 1}
  <div class="rt-linked">
    <button
      bind:this={btn}
      type="button"
      class="rt-chip"
      aria-haspopup="dialog"
      aria-expanded={open}
      aria-label="Linked threads: {linked.length}"
      title="Linked threads"
      data-testid="runtime-linked"
      onclick={() => (open = !open)}
    >
      <Icon name="people" size={12} />{linked.length} linked
    </button>
  </div>
{/if}
{#if open && btn}
  <ThreadPopover anchor={btn} label="Linked threads" onclose={() => (open = false)}>
    <ul class="rt-list" aria-label="Linked threads">
      {#each linked as t (t.id)}
        {@const current = t.id === sessionId}
        {@const st = relatedStatus(t.status)}
        <li class="rt-row">
          <button
            type="button"
            class="rt-link"
            aria-current={current ? 'page' : undefined}
            disabled={current}
            title="{t.title} — {harnessLabel(t.provider)}"
            onclick={() => pick(t.id, false)}
          >
            <span class="rt-link-dot" data-tone={st.tone} aria-hidden="true"></span>
            {#if hasProviderIcon(t.provider)}<ProviderIcon provider={t.provider} size={12} />{/if}
            <span class="rt-title ellipsis" dir="auto">{t.title}</span>
            <span class="rt-meta">{current ? 'Shown' : st.text}</span>
          </button>
          <button type="button" class="icon-btn" aria-label="Open {t.title} in Threads" title="Open in Threads" onclick={() => pick(t.id, true)}>
            <Icon name="external" size={12} />
          </button>
        </li>
      {/each}
    </ul>
  </ThreadPopover>
{/if}

<style>
  .rt-linked {
    display: flex;
    justify-content: flex-end;
    padding: 4px 8px 0;
    flex: none;
  }
  .rt-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    block-size: 24px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .rt-chip:hover,
  .rt-chip[aria-expanded='true'] {
    background: var(--hover);
    color: var(--text);
  }
  .rt-chip:focus-visible,
  .rt-link:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .rt-list {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .rt-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .rt-link {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
    min-block-size: 32px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .rt-link:hover:not(:disabled) {
    background: var(--hover);
  }
  .rt-link:disabled {
    cursor: default;
  }
  .rt-title {
    flex: 1;
    min-inline-size: 0;
  }
  .rt-meta {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  /* Phone: a linked thread is a primary hop — a full tap target. */
  @media (pointer: coarse) {
    .rt-chip,
    .rt-link {
      min-block-size: var(--hit-min);
    }
  }
  .rt-link-dot {
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex: none;
  }
  .rt-link-dot[data-tone='live'] {
    background: var(--info);
  }
  .rt-link-dot[data-tone='ask'] {
    background: var(--warning);
  }
  .rt-link-dot[data-tone='idle'] {
    background: var(--success);
  }
</style>
