<script lang="ts">
  // Agent Mode: the inbox (AgentsInbox — every session, needs you first) beside
  // the canvas (tabbed split panes or the tiled grid), as a ListDetail (design.md
  // §2–§4): the list pane hides under its ⋯ or ⌘B ("Show Sessions" then sits at
  // the start of the tab strip), and with nothing at all to list the canvas's
  // EmptyState / first-run coach owns the page. While it is open the app's
  // navigator folds to its icon rail (the person's preference, as on Threads).
  // A phone pushes: `#/agents` is the list (its header is this page's one row),
  // `#/agents/<id>` the session (its pane header carries ‹ Sessions).
  import { onMount, untrack } from 'svelte';
  import AgentsInbox from './AgentsInbox.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { foldNavWhileMounted } from '../../lib/foldNavWhileMounted.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { router } from '../../lib/router.svelte';
  import { lsGet, lsSet } from '../../lib/storage';
  import Splits from './Splits.svelte';
  import TiledView from './TiledView.svelte';
  import FirstRunCoach from './FirstRunCoach.svelte';
  import { startThreadFeeds, threadFeeds } from './threadFeeds.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { pageKeyOf } from '../../shell/pages.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { rightPanel } from '../../lib/stores/rightPanel.svelte';

  // The focused session's details card (Threads' card: folder, branch, changes
  // → commit → push → PR, context, MCP, subagents) is this page's Side panel
  // tab "Session" (design.md §3: a page registers its tab, as the Vault does).
  // A chat pane's folder comes from its thread row (read by the card's chunk,
  // loaded only when the tab is shown).
  const SessionDetails = lazyComponent(() => import('./SessionDetailsCard.svelte'));
  const SESSION_TAB = 'page:agents-session' as const;
  const sessionDetails = $derived.by(() => {
    if (router.module !== 'agents' && router.module !== '') return null; // kept alive under another page
    const id = ws.activeSessionId;
    if (id && ws.threadPanes.includes(id)) return { threadId: id, cwd: null };
    const s = rightPanel.session ? ws.getSession(id) : null;
    return s?.cwd ? { threadId: s.id, cwd: s.cwd } : null;
  });
  // Until the person picks a Side panel tab, it opens on this one here (A5).
  onMount(() => {
    if (lsGet('otto_right_tab') === null) ui.rightTab = SESSION_TAB;
  });
  $effect(() => {
    const props = sessionDetails;
    if (!props) return;
    return untrack(() =>
      rightPanel.register({
        id: SESSION_TAB,
        label: 'Session',
        icon: 'terminal',
        component: LazyMount,
        props: { lazy: SessionDetails, what: 'the session details', variant: 'compact', props },
      }),
    );
  });

  const tiled = $derived(ws.viewMode === 'tiled');

  /** Phone: `#/agents` is the list, `#/agents/<id>` the session (pushed). */
  const sessionRouted = $derived(!!router.parts[1]);
  onMount(() =>
    foldNavWhileMounted({
      get: () => lsGet('otto_agents_fold_nav') !== '0',
      set: (on) => lsSet('otto_agents_fold_nav', on ? '1' : '0'),
    }),
  );
  // The per-session task roll-up (row tooltips, pane header); live from events after.
  $effect(() => {
    const w = ws.currentId;
    if (w) activity.ensureSummary(w);
  });

  // First-run coach: a guided path from a fresh account to a launched agent.
  // Shown only on a truly empty Agents view — no agent session in this
  // workspace, under "No workspace" or in another loaded workspace (the inbox
  // lists those too) — and only until the user dismisses it or launches a
  // session (remembered per-machine).
  let coachDismissed = $state(
    (() => {
      try {
        return localStorage.getItem('otto_firstrun_dismissed') === '1';
      } catch {
        return false;
      }
    })(),
  );
  const coachMaybe = $derived(
    !coachDismissed && ws.agentSessions.length === 0 && ws.scratchSessions.length === 0 && ws.otherWsSessions.length === 0,
  );
  // …decided only once every list it rests on has answered — on a cold boot
  // they are all empty first: this workspace's sessions, the other
  // workspaces' (the inbox lists them) and the structured threads (someone
  // with threads only is no first-timer). Once shown it stays while the
  // coach's own "create workspace" re-selects (its draft is kept).
  const ptyKnown = $derived(
    ws.layoutReady && (ws.otherWsLoaded || !ws.allWorkspaces || ws.workspaces.every((w) => w.id === ws.currentId)),
  );
  let rt = $state<{ byId: Record<string, unknown> } | null>(null);
  let rtFailed = $state(false); // its chunk didn't load: no coach, but never a skeleton for good
  // Nothing at all to list (no session here or elsewhere, none in History, no
  // thread): no list pane — the canvas's EmptyState or the coach owns the page.
  const ptyNone = $derived(
    ws.agentSessions.length === 0 && ws.scratchSessions.length === 0 && ws.otherWsSessions.length === 0 &&
      ws.telegramSessions.length === 0 && ws.slackSessions.length === 0 && !ws.hasArchived,
  );
  $effect(() => {
    if ((coachMaybe || ptyNone) && ptyKnown && !rt) void startThreadFeeds().then(([m]) => (rt = m.runtimeThreads), () => (rtFailed = true));
  });
  const coachKnown = $derived(ptyKnown && ((threadFeeds.settled && !!rt) || rtFailed));
  const coachReady = $derived(coachMaybe && coachKnown && !!rt && Object.keys(rt.byId).length === 0);
  const listEmpty = $derived(ptyNone && coachKnown && (rtFailed || Object.keys(rt?.byId ?? {}).length === 0));
  let coachLatched = $state(false);
  $effect(() => {
    if (coachReady) coachLatched = true;
  });
  const showCoach = $derived(coachMaybe && (coachReady || coachLatched));
  const coachPending = $derived(coachMaybe && !coachKnown && !coachLatched);
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
    // Read the hash, not the router: it already names the next page (or
    // session) while that hashchange is queued or the page's chunk loads, and
    // opening a session then overwrote it and pulled the person back.
    const target = window.location.hash.replace(/^#\/?/, '').split('/');
    // A named session not open yet is not the arrival: a remembered one
    // deleted since is dropped for `#/agents`, which then opens the latest.
    if (target[1] && router.parts[1] === target[1] && ws.activeSessionId !== target[1]) return;
    autoOpenedFor = key;
    if (untrack(() => pageKeyOf(target)) !== 'agents' || target[1] || viewport.isPhone) return;
    if (ws.panes.length > 0 || tiled) return;
    const latest = [...ws.mainSessions].sort(
      (a, b) => Date.parse(b.last_active_at) - Date.parse(a.last_active_at),
    )[0];
    if (latest) ws.navigateToSession(latest.id);
  });
</script>

<div class="agents-page">
{#if viewport.navDrawer && !sessionRouted}
  <!-- Phone: the list page's one header row (☰ · Sessions N · New · ⋯ · Side panel). -->
  <PageHeader title="Agents" />
{/if}
<div class="agents">
  <ListDetail
    module="agents"
    name="Sessions"
    open={sessionRouted}
    onback={() => router.backTo('agents')}
    empty={listEmpty}
    legacy={{ w: 'agents.inboxW', hidden: () => (lsGet('otto_agents_inbox_hidden') === '1' ? true : null) }}
  >
    {#snippet list()}<AgentsInbox />{/snippet}
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
    {:else if coachPending}
      <div style="padding: 16px"><Skeleton rows={3} height={48} /></div>
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
        body="Pick a session in the list, switch to tiled view, or start a new one (⌘T)."
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
  </ListDetail>
</div>
</div>

<style>
  .agents-page {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .agents {
    flex: 1;
    min-height: 0;
    display: flex;
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
