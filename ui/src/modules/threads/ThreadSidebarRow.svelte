<script lang="ts">
  import Badge from '../../lib/components/Badge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { events } from '../../lib/events.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import { sessionState } from '../../lib/status';
  import { git } from '../../lib/stores/git.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { Session } from '../../lib/api/types';
  import { sessionBranch } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    session: Session;
    variant: 'pinned' | 'active' | 'history';
    child?: boolean;
    onrowclick: (e: MouseEvent, s: Session) => void;
    onreopen: (s: Session, beside: boolean) => void;
    onmenu: (e: MouseEvent, s: Session) => void;
    onselect?: () => void;
  }

  let {
    session: s,
    variant,
    child = false,
    onrowclick,
    onreopen,
    onmenu,
    onselect,
  }: Props = $props();

  const needs = $derived(ws.needsYou[s.id] === true);
  const rawSt = $derived(ws.statusMap[s.id] ?? s.status);
  const st = $derived(sessionState(s, rawSt, needs, { stale: events.state !== 'connected' }));
  const br = $derived(variant === 'active' ? sessionBranch(s, git.repos, git.statusById) : null);
  const pinned = $derived(threads.isPinned(s.id));
  const selected = $derived(threads.focusedSessionId === s.id && !threads.newDraftMode);
  const inPane = $derived(threads.paneIds.includes(s.id) && !threads.newDraftMode);
</script>

{#if variant === 'pinned'}
  <div
    class="th-row reveal-host"
    class:selected
    class:in-pane={inPane}
    data-testid="thread-pinned-{s.id}"
  >
    <button
      type="button"
      class="th-row-main"
      onclick={(e) => onrowclick(e, s)}
      oncontextmenu={(e) => onmenu(e, s)}
      use:rowMenu
    >
      <StatusDot state={st} />
      {#if hasProviderIcon(s.provider)}
        <ProviderIcon provider={s.provider} size={13} />
      {/if}
      <span class="th-row-title ellipsis" dir="auto">{s.title}</span>
    </button>
    <div class="th-row-actions">
      <button
        type="button"
        class="icon-btn th-pin-on"
        aria-label="Unpin {s.title}"
        title="Unpin thread"
        onclick={() => threads.togglePin(s.id)}
      >
        <Icon name="star" size={12} />
      </button>
      <button
        type="button"
        class="icon-btn reveal-on-hover"
        aria-label="Open {s.title} beside"
        title="Open beside (Split)"
        onclick={() => {
          threads.openBeside(s.id);
          onselect?.();
        }}
      >
        <Icon name="split" size={12} />
      </button>
    </div>
  </div>
{:else if variant === 'active'}
  <div
    class="th-row th-row-rich reveal-host"
    class:child
    class:needs-you={needs}
    class:selected
    class:in-pane={inPane}
    data-testid="thread-row-{s.id}"
  >
    <button
      type="button"
      class="th-row-card"
      onclick={(e) => onrowclick(e, s)}
      oncontextmenu={(e) => onmenu(e, s)}
      use:rowMenu
    >
      <div class="th-row-top">
        <StatusDot state={st} />
        {#if hasProviderIcon(s.provider)}
          <ProviderIcon provider={s.provider} size={13} />
        {:else}
          <Icon name="terminal" size={13} />
        {/if}
        <span class="th-row-title ellipsis" dir="auto">{s.title}</span>
        <span class="th-row-time">{rel(s.last_active_at)}</span>
      </div>
      <div class="th-row-sub">
        {#if needs}
          <Badge tone="warn" label="Needs approval" dot />
        {:else if rawSt === 'working'}
          <Badge tone="ok" variant="outline" label="Working…" live />
        {/if}
        {#if child}
          <span class="th-sub-meta">Worker</span>
        {/if}
        {#if br}
          <span class="th-branch mono ellipsis" dir="ltr" title={br}>
            <Icon name="branch" size={12} />
            {br}
          </span>
        {/if}
      </div>
    </button>
    <div class="th-row-actions reveal-on-hover">
      <button
        type="button"
        class="icon-btn"
        class:th-pin-on={pinned}
        aria-label="{pinned ? 'Unpin' : 'Pin'} {s.title}"
        title={pinned ? 'Unpin thread' : 'Pin thread'}
        data-testid="thread-pin-{s.id}"
        onclick={() => threads.togglePin(s.id)}
      >
        <Icon name="star" size={12} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Open {s.title} beside in Split"
        title="Open beside (Split)"
        data-testid="thread-split-{s.id}"
        onclick={() => {
          threads.openBeside(s.id);
          onselect?.();
        }}
      >
        <Icon name="split" size={12} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Settle {s.title} to History"
        title="Settle to History"
        data-testid="thread-settle-{s.id}"
        onclick={() => threads.toggleSettle(s.id)}
      >
        <Icon name="check" size={12} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="More actions for {s.title}"
        title="More actions"
        aria-haspopup="menu"
        onclick={(e) => onmenu(e, s)}
      >
        <Icon name="more" size={12} />
      </button>
    </div>
  </div>
{:else}
  <div class="th-row reveal-host" data-testid="thread-history-{s.id}">
    <button
      type="button"
      class="th-row-main"
      onclick={() => onreopen(s, false)}
      oncontextmenu={(e) => onmenu(e, s)}
      use:rowMenu
    >
      {#if hasProviderIcon(s.provider)}
        <ProviderIcon provider={s.provider} size={13} />
      {:else}
        <Icon name="archive" size={12} />
      {/if}
      <span class="th-row-title ellipsis" dir="auto">{s.title}</span>
      <span class="th-row-time">{rel(s.last_active_at)}</span>
    </button>
    <div class="th-row-actions reveal-on-hover">
      <button
        type="button"
        class="icon-btn"
        aria-label="Reopen {s.title}"
        title="Reopen / Resume"
        onclick={() => onreopen(s, false)}
      >
        <Icon name="refresh" size={12} />
      </button>
      <button
        type="button"
        class="icon-btn"
        class:th-pin-on={pinned}
        aria-label="{pinned ? 'Unpin' : 'Pin'} {s.title}"
        title={pinned ? 'Unpin' : 'Pin'}
        onclick={() => threads.togglePin(s.id)}
      >
        <Icon name="star" size={12} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="View {s.title} beside"
        title="View transcript beside"
        onclick={() => onreopen(s, true)}
      >
        <Icon name="split" size={12} />
      </button>
    </div>
  </div>
{/if}

<style>
  .th-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: var(--radius-m);
    border: 1px solid transparent;
    transition: background var(--dur-fast) var(--ease-out), border-color var(--dur-fast) var(--ease-out);
  }
  .th-row:hover {
    background: var(--hover);
  }
  .th-row.in-pane {
    background: var(--accent-faint);
    border-color: var(--border);
  }
  .th-row.selected {
    background: var(--accent-soft);
    border-color: var(--accent-line);
  }
  .th-row.needs-you {
    background: var(--warning-soft);
    border-color: var(--border-strong);
  }
  .th-row.child {
    margin-inline-start: 16px;
    border-inline-start: 2px solid var(--border-strong);
  }
  .th-row-main,
  .th-row-card {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .th-row-main {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
  }
  .th-row-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px;
  }
  .th-row-top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .th-row-title {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text);
  }
  .th-row-time {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .th-row-sub {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding-inline-start: 14px;
    min-width: 0;
  }
  .th-sub-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .th-branch {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 140px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .th-row-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-inline-end: 4px;
    flex-shrink: 0;
  }
  .th-pin-on {
    color: var(--warning);
  }
  .th-pin-on :global(svg path) {
    fill: currentColor;
  }
</style>
