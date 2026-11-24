<script lang="ts">
  // Agent Mode: the inbox (AgentsInbox — every session, needs you first) beside
  // the canvas (tabbed split panes or the tiled grid). While it is open the
  // app's navigator folds to its icon rail (foldNavWhileMounted), as on
  // Threads. A phone shows the inbox as the page (`#/agents`) and a session
  // full-height (`#/agents/<id>`, its pane header's ‹ returns to the list).
  import { onMount, untrack } from 'svelte';
  import AgentsInbox from './AgentsInbox.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { loadPaneWidth } from '../../lib/paneResizer';
  import { foldNavWhileMounted } from '../../lib/foldNavWhileMounted.svelte';
  import { pageChrome } from '../../lib/stores/pageChrome.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { router } from '../../lib/router.svelte';
  import { lsGet, lsSet } from '../../lib/storage';
  import Splits from './Splits.svelte';
  import TiledView from './TiledView.svelte';
  import FirstRunCoach from './FirstRunCoach.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { pageKeyOf } from '../../shell/pages.svelte';

  const tiled = $derived(ws.viewMode === 'tiled');

  const INBOX_W_KEY = 'agents.inboxW';
  let inboxW = $state(loadPaneWidth(INBOX_W_KEY, 272, 220, 420));
  let inboxHidden = $state(lsGet('otto_agents_inbox_hidden') === '1');
  function setInboxHidden(on: boolean): void {
    inboxHidden = on;
    lsSet('otto_agents_inbox_hidden', on ? '1' : '0');
  }
  const phone = $derived(viewport.isPhone);
  /** Phone: `#/agents` is the list, `#/agents/<id>` the session. */
  const phoneList = $derived(phone && !router.parts[1]);
  onMount(() => pageChrome.set({ foldNav: true }));
  onMount(() =>
    foldNavWhileMounted({
      get: () => lsGet('otto_agents_fold_nav') !== '0',
      set: (on) => lsSet('otto_agents_fold_nav', on ? '1' : '0'),
    }),
  );
  // The per-session task roll-up (row tooltips, pane header); live from events after.
  $effect(() => {
    const w = ws.currentId;
    if (w) void activity.loadSummary(w);
  });

  // First-run coach: a guided path from a fresh account to a launched agent.
  // Shown only on a truly empty Agents view (no agent sessions at all) and only
  // until the user dismisses it or launches a session (remembered per-machine).
  let coachDismissed = $state(
    (() => {
      try {
        return localStorage.getItem('otto_firstrun_dismissed') === '1';
      } catch {
        return false;
      }
    })(),
  );
  const showCoach = $derived(!coachDismissed && ws.agentSessions.length === 0);
  const preserveCoachDraft = $derived(
    showCoach && ws.currentId === null && !tiled && ws.panes.length === 0,
  );

  // Never open onto a "pick one" void when there are sessions: once per
  // workspace, if its restored layout has no panes, open the most recently
  // active session. Only on ARRIVAL — closing the last tab later leaves the
  // empty state alone (the user just chose that). An arrival that already
  // names its session (`#/agents/<id>` — a Home box, a notification, ⌘K) is
  // never overridden: its pane may not exist yet when this runs, and opening
  // the latest session here used to jump away from the one just clicked.
  let autoOpenedFor: string | null | undefined = undefined;
  $effect(() => {
    if (!ws.layoutReady || ws.sessionsLoading || ws.sessionsError) return;
    const key = ws.currentId;
    if (autoOpenedFor === key) return;
    autoOpenedFor = key;
    // Read the hash, not the router: it already names the next page (or
    // session) while that hashchange is queued or the page's chunk loads, and
    // opening a session then overwrote it and pulled the person back.
    const target = window.location.hash.replace(/^#\/?/, '').split('/');
    if (untrack(() => pageKeyOf(target)) !== 'agents' || target[1] || viewport.isPhone) return;
    if (ws.panes.length > 0 || tiled) return;
    const latest = [...ws.mainSessions].sort(
      (a, b) => Date.parse(b.last_active_at) - Date.parse(a.last_active_at),
    )[0];
    if (latest) ws.navigateToSession(latest.id);
  });
</script>

<div class="agents" class:phone>
  {#if !phone && !inboxHidden}
    <div class="agents-inbox" style:inline-size="{inboxW}px">
      <AgentsInbox onhide={() => setInboxHidden(true)} />
    </div>
    <PaneDivider bind:width={inboxW} storageKey={INBOX_W_KEY} min={220} max={420} defaultWidth={272} label="Resize the session list" />
  {:else if !phone}
    <div class="agents-reveal">
      <button type="button" class="icon-btn" aria-label="Show the session list" title="Show the session list" data-testid="agents-inbox-show" onclick={() => setInboxHidden(false)}>
        <Icon name="sidebar" size={14} />
      </button>
    </div>
  {/if}
  {#if phoneList}
    <div class="agents-body"><AgentsInbox /></div>
  {:else}
  <div class="agents-body">
  <!-- Scratch-session discovery can begin after the no-workspace coach has
       accepted a draft. Keep that form mounted; real workspace navigation
       and the ordinary empty state still show their loading skeleton. -->
  {#if ws.sessionsError}
    <LoadState variant="compact" what="sessions" error={ws.sessionsError} empty onretry={() => void ws.retrySessions()} />
  {/if}
  {#if ws.sessionsError && !ws.layoutReady}
    <!-- A failed selection must not mount another workspace's panes. -->
  {:else if ws.sessionsLoading && ws.sessions.length === 0 && !preserveCoachDraft}
    <div style="padding: 16px">
      <Skeleton rows={3} height={48} />
    </div>
  {:else if tiled}
    <TiledView />
  {:else if ws.panes.length === 0}
    {#if showCoach}
      <FirstRunCoach ondismiss={() => (coachDismissed = true)} />
    {:else if ws.activeSessions.length === 0}
      <EmptyState
        icon="terminal"
        title="No sessions yet"
        body="Start an agent (Claude, Codex) or a plain shell in this workspace (⌘T). Sessions keep running even when you close the app."
        variant="page"
        actionIcon="plus"
        actionLabel="New session…"
        onaction={() => (ui.newSessionOpen = true)}
      />
    {:else}
      <EmptyState
        icon="terminal"
        title="No open tabs"
        body="Pick a session in the sidebar, switch to tiled view, or start a new one (⌘T)."
        variant="page"
        actionIcon="plus"
        actionLabel="New session…"
        onaction={() => (ui.newSessionOpen = true)}
      />
    {/if}
  {:else}
    <Splits />
  {/if}
  </div>
  {/if}
</div>

<style>
  .agents {
    height: 100%;
    min-height: 0;
    display: flex;
  }
  .agents.phone {
    flex-direction: column;
  }
  .agents-inbox {
    flex-shrink: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .agents-reveal {
    flex-shrink: 0;
    padding: var(--sp-2);
    border-inline-end: 1px solid var(--separator);
  }
  .agents-body {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .agents-body > :global(*:last-child) {
    flex: 1;
    min-height: 0;
  }

</style>
