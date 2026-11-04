<script lang="ts">
  // One quiet single-line thread row (T3-style): status dot, title, time —
  // the time gives way to pin / beside / ⋯ on hover or focus. A thread that
  // needs you gets a warning dot and a soft tint; nothing else shouts.
  import Icon from '../../lib/components/Icon.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { events } from '../../lib/events.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import { sessionState } from '../../lib/status';
  import { git } from '../../lib/stores/git.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { Session } from '../../lib/api/types';
  import { harnessLabel, sessionBranch } from './threadsHelpers';
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

  let { session: s, variant, child = false, onrowclick, onreopen, onmenu, onselect }: Props = $props();

  const needs = $derived(ws.needsYou[s.id] === true);
  const st = $derived(sessionState(s, ws.statusMap[s.id] ?? s.status, needs, { stale: events.state !== 'connected' }));
  const pinned = $derived(threads.isPinned(s.id));
  const selected = $derived(threads.focusedSessionId === s.id && !threads.newDraftMode);
  const inPane = $derived(threads.paneIds.includes(s.id) && !threads.newDraftMode);
  const history = $derived(variant === 'history');
  // The tooltip (branch lookup over every repo's status) is only built while
  // the row is hovered or focused: a git status change no longer re-derives it
  // for every row of the sidebar.
  let hot = $state(false);
  const tip = $derived.by(() => {
    if (!hot) return s.title;
    const br = sessionBranch(s, git.repos, git.statusById);
    return [s.title, harnessLabel(s.provider), br ? `on ${br}` : null, needs ? 'needs your approval' : null]
      .filter(Boolean)
      .join(' · ');
  });

  const testid = $derived(
    variant === 'pinned' ? `thread-pinned-${s.id}` : variant === 'history' ? `thread-history-${s.id}` : `thread-row-${s.id}`,
  );
</script>

<div
  class="tr-row reveal-host"
  class:child
  class:needs-you={needs}
  class:selected
  class:in-pane={inPane && !selected}
  data-testid={testid}
>
  <button
    type="button"
    class="tr-main"
    title={tip}
    onpointerenter={() => (hot = true)}
    onpointerleave={() => (hot = false)}
    onfocus={() => (hot = true)}
    onblur={() => (hot = false)}
    aria-current={selected ? 'true' : undefined}
    onclick={(e) => (history ? onreopen(s, e.metaKey || e.ctrlKey) : onrowclick(e, s))}
    oncontextmenu={(e) => onmenu(e, s)}
    use:rowMenu
  >
    {#if history}
      <Icon name="archive" size={12} />
    {:else}
      <StatusDot state={st} />
    {/if}
    <span class="tr-title ellipsis" dir="auto">{s.title}</span>
    <span class="tr-time">{rel(s.last_active_at)}</span>
  </button>
  <div class="tr-actions reveal-on-hover">
    {#if variant !== 'pinned'}
      <button
        type="button"
        class="icon-btn"
        class:tr-pin-on={pinned}
        aria-label="{pinned ? 'Unpin' : 'Pin'} {s.title}"
        title={pinned ? 'Unpin thread' : 'Pin thread'}
        data-testid="thread-pin-{s.id}"
        onclick={() => threads.togglePin(s.id)}
      >
        <Icon name="star" size={12} />
      </button>
    {/if}
    <button
      type="button"
      class="icon-btn"
      aria-label="Open {s.title} beside"
      title="Open beside (Split)"
      data-testid="thread-split-{s.id}"
      onclick={() => {
        if (history) onreopen(s, true);
        else {
          threads.openBeside(s.id);
          onselect?.();
        }
      }}
    >
      <Icon name="split" size={12} />
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

<style>
  .tr-row {
    --tr-tint: transparent;
    position: relative;
    display: flex;
    align-items: center;
    min-width: 0;
    border-radius: var(--radius-m);
    background: var(--tr-tint);
  }
  .tr-row:hover {
    --tr-tint: var(--hover);
  }
  .tr-row.in-pane {
    --tr-tint: var(--accent-faint);
  }
  .tr-row.selected {
    --tr-tint: var(--accent-soft);
  }
  .tr-row.needs-you {
    --tr-tint: var(--warning-soft);
  }
  .tr-row.child {
    margin-inline-start: 14px;
  }
  .tr-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    block-size: 30px;
    padding-inline: 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .tr-main :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .tr-title {
    flex: 1;
    min-width: 0;
  }
  .tr-row:not(.selected) .tr-title {
    color: var(--text-dim);
  }
  .tr-row:hover .tr-title,
  .tr-row.needs-you .tr-title {
    color: var(--text);
  }
  .tr-time {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .tr-actions {
    position: absolute;
    inset-block: 0;
    inset-inline-end: 2px;
    display: flex;
    align-items: center;
    gap: 0;
    padding-inline-start: 6px;
    border-radius: var(--radius-m);
    /* Opaque so the title never shows through the revealed actions. */
    background: linear-gradient(var(--tr-tint), var(--tr-tint)), var(--surface);
  }
  .tr-row:hover .tr-time,
  .tr-row:focus-within .tr-time {
    visibility: hidden;
  }
  @media (hover: none) {
    .tr-time {
      visibility: hidden;
    }
  }
  .tr-pin-on {
    color: var(--warning);
  }
  .tr-pin-on :global(svg path) {
    fill: currentColor;
  }
</style>
