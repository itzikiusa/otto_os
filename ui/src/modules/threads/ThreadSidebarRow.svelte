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
  import { agentThreadBadge } from '../../lib/personalAgentBadges';
  import { harnessLabel, sessionBranch } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';
  import { runtimeThreads } from './runtime/runtimeThreads.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { councilMembers, councilOf } from './runtime/council';
  import { council } from './runtime/councilStore.svelte';

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
  /** Finished a turn while you looked elsewhere (cleared when focused). */
  const unread = $derived(!selected && !needs && (ws.unread[s.id] === true || runtimeThreads.unread[s.id] === true));
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

  /** A personal agent's thread says what started it: Chat / Scheduled ·
   *  routine / Manual run / Proactive / Delegated by … */
  const badge = $derived(agentThreadBadge(s.meta as Record<string, unknown> | undefined));

  // A council (one prompt, several agents) is ONE row — "Council · 3" with
  // its seats' provider icons — on seat 1; it unfolds into the seats (and
  // the synthesis), which otherwise hide under it.
  const cmeta = $derived(councilOf(runtimeThreads.get(s.id)));
  const members = $derived(cmeta ? councilMembers(Object.values(runtimeThreads.byId), s.id) : null);
  const head = $derived(variant === 'active' && cmeta?.role === 'seat' && !runtimeThreads.get(s.id)?.parent_id);
  const unfolded = $derived(!!cmeta && council.expanded[cmeta.id] === true);
  const folded = $derived(!!cmeta && child && !head && !unfolded && !selected);
  const councilState = $derived.by(() => {
    const ids = members ? [...members.seats, ...(members.synthesis ? [members.synthesis] : [])].map((m) => m.id) : [s.id];
    const working = ids.some((id) => runtimeThreads.get(id)?.status === 'working' || runtimeThreads.get(id)?.status === 'starting');
    return sessionState(s, working ? 'working' : 'idle', ids.some((id) => ws.needsYou[id] === true), { stale: events.state !== 'connected' });
  });

  const testid = $derived(
    variant === 'pinned' ? `thread-pinned-${s.id}` : variant === 'history' ? `thread-history-${s.id}` : `thread-row-${s.id}`,
  );
</script>

{#if head && cmeta && members}
  <div class="tr-row tr-council" class:selected={selected && council.showsCouncil(s.id)} data-testid="thread-council-{cmeta.id}">
    <button
      type="button"
      class="icon-btn tr-fold"
      aria-expanded={unfolded}
      aria-label="{unfolded ? 'Fold' : 'Show'} the council’s threads"
      title={unfolded ? 'Fold the seats' : 'Show the seats'}
      data-testid="thread-council-fold-{cmeta.id}"
      onclick={() => council.toggleExpanded(cmeta.id)}
    >
      <Icon name={unfolded ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
    <button
      type="button"
      class="tr-main"
      title="Council · {members.seats.length}: {s.title}"
      aria-current={selected ? 'true' : undefined}
      onclick={() => {
        council.showCouncil(s.id);
        onselect?.();
      }}
      oncontextmenu={(e) => onmenu(e, s)}
    >
      <StatusDot state={councilState} />
      <span class="tr-stack" aria-hidden="true">
        {#each members.seats.slice(0, 4) as m (m.id)}
          {#if hasProviderIcon(m.provider)}<ProviderIcon provider={m.provider} size={12} />{/if}
        {/each}
      </span>
      <span class="tr-title ellipsis" dir="auto">{s.title.replace(/ · [^·]*$/, '')}</span>
      <span class="tr-badge">Council · {members.seats.length}</span>
      <span class="tr-time">{rel(s.last_active_at)}</span>
    </button>
  </div>
  {#if unfolded}{@render row(true)}{/if}
{:else if !folded}
  {@render row(child)}
{/if}

{#snippet row(nested: boolean)}
<div
  class="tr-row reveal-host"
  class:child={nested}
  class:needs-you={needs}
  class:unread
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
    {#if badge}<span class="tr-badge tr-badge-{badge.kind} ellipsis" title={badge.title} data-testid="thread-badge-{s.id}">{badge.label}</span>{/if}
    {#if unread}<span class="tr-unread" role="img" aria-label="Unread" title="New since you last looked" data-testid="thread-unread-{s.id}"></span>{/if}
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
{/snippet}

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
  .tr-row.unread .tr-title,
  .tr-row.needs-you .tr-title {
    color: var(--text);
  }
  .tr-badge {
    flex-shrink: 1;
    max-inline-size: 45%;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: var(--fs-xs);
    line-height: 16px;
    color: var(--text-dim);
  }
  .tr-badge-chat {
    color: var(--accent-text);
    border-color: var(--accent-line);
  }
  .tr-badge-scheduled {
    color: var(--info);
  }
  .tr-badge-delegated {
    color: var(--success);
  }
  .tr-unread {
    flex-shrink: 0;
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    background: var(--accent-solid);
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
  /* The revealed actions overlay the row's end. Reserve their width instead
     of painting over the title: an RTL title STARTS at that end, so the
     overlay hid its first letters. */
  .tr-row:hover .tr-time,
  .tr-row:focus-within .tr-time {
    display: none;
  }
  .tr-row:hover .tr-main,
  .tr-row:focus-within .tr-main {
    padding-inline-end: 80px;
  }
  @media (hover: none) {
    .tr-time {
      display: none;
    }
    .tr-main {
      padding-inline-end: 80px;
    }
  }
  .tr-council .tr-main {
    padding-inline-start: 0;
  }
  .tr-fold {
    flex: none;
  }
  .tr-stack {
    display: inline-flex;
    flex: none;
  }
  .tr-stack > :global(*:not(:first-child)) {
    margin-inline-start: -4px;
  }
  /* The council row has no hover actions: keep its time, no reserved end. */
  .tr-row.tr-council .tr-main {
    padding-inline-end: 8px;
  }
  .tr-row.tr-council .tr-time {
    display: inline;
  }
  .tr-pin-on {
    color: var(--warning);
  }
  .tr-pin-on :global(svg path) {
    fill: currentColor;
  }
</style>
