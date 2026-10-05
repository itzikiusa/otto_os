<script lang="ts">
  // Menu-bar popover (`#/tray`, the desktop shell's `otto-tray` panel, 360×520
  // over native popover vibrancy). Loaded once, hidden, at app start: it reads
  // the existing endpoints — sessions, pending MCP approvals, notifications —
  // again only when their events say they changed (see below), lists
  // Needs you · Running · Today, and reports the counts to the shell so
  // the menu-bar glyph shows a dot while work runs and turns amber when
  // something needs you. Rows open the full Otto window at the right route.
  // The assistant's own threads/reminders arrive with the assistant API.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  // StatusDot + sessionState directly, not LiveWorkingDot: the tray is in the
  // entry bundle and LiveWorkingDot's default reads the main window's events
  // client, which would drag the whole events/store graph into every window.
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { sessionState } from '../../lib/status';
  import { api, isAbortError } from '../../lib/api/client';
  import type { Poller } from '../../lib/poll';
  import { liveQuery } from '../../lib/live';
  import { TopicSocket } from '../../lib/topicSocket';
  import type { McpApproval, Notice, Session, WorkspaceWithRole } from '../../lib/api/types';
  import { auth } from '../../lib/stores/auth.svelte';
  import { isForeground, SCRATCH_WORKSPACE_ID, visibleOnThisDevice } from '../../lib/stores/workspace.svelte';
  import {
    bar,
    onDesktopEvent,
    openInOtto,
    shortcuts,
    tray,
    useGlassWindow,
  } from '../../lib/desktop';

  const POLL_MS = 20_000;
  const TODAY_MAX = 6;

  interface RunningRow {
    session: Session;
    workspace: string;
  }
  interface NeedsRow {
    id: string;
    title: string;
    detail: string;
    icon: 'shield' | 'terminal' | 'warning';
    route: string;
  }

  // Event-fed (TRANSPORT_PLAN stage 2): the tray opens its OWN topic-filtered
  // `/ws/events` socket (lib/topicSocket.ts) and re-reads a list only when an
  // event says it changed — each list on its own, so a session flipping
  // working↔idle costs one small query, not four. While the socket is up a
  // 5-min safety net replaces the old 20 s × 4-request poll (which ran while
  // hidden too); while it is down the old 20 s cadence comes back.
  const TOPICS = [
    'session_status',
    'session_created',
    'session_removed',
    'notification',
    'notifications_changed',
    'mcp_approval_changed',
  ] as const;

  /** The tray's own events socket is up (drives the stale-aware live dots). */
  let sockUp = $state(false);
  /** The "working" dot goes stale (Reconnecting…) while the tray socket is down. */
  const workingDot = $derived(sessionState(null, 'working', false, { stale: !sockUp }));
  let workspaces: WorkspaceWithRole[] | null = $state(null);
  let working: Session[] | null = $state(null);
  let approvals: McpApproval[] | null = $state(null);
  let notices: Notice[] | null = $state(null);
  let errors: Record<string, string> = $state({});
  let askChord = $state('');

  const loaded = $derived(workspaces !== null && working !== null && approvals !== null && notices !== null);
  const loadError = $derived(Object.values(errors)[0] ?? null);

  function sessionIdOf(n: Notice): string | null {
    if (n.action?.type === 'open_session') return n.action.session_id;
    const key = n.source_key ?? '';
    return key.startsWith('session:') ? (key.split(':')[1] ?? null) : null;
  }

  function isToday(iso: string): boolean {
    const d = new Date(iso);
    const now = new Date();
    return (
      d.getFullYear() === now.getFullYear() &&
      d.getMonth() === now.getMonth() &&
      d.getDate() === now.getDate()
    );
  }

  function timeOf(iso: string): string {
    return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  /** `Alt+Space` → `⌥Space` (menu-bar glyph style). */
  function prettyChord(accel: string): string {
    const map: Record<string, string> = {
      cmd: '⌘', command: '⌘', super: '⌘', ctrl: '⌃', control: '⌃',
      alt: '⌥', option: '⌥', shift: '⇧',
    };
    return accel
      .split('+')
      .map((p) => map[p.trim().toLowerCase()] ?? p.trim())
      .join('');
  }

  const running: RunningRow[] = $derived.by(() => {
    if (!working) return [];
    const names = new Map((workspaces ?? []).map((w) => [w.id, w.name]));
    names.set(SCRATCH_WORKSPACE_ID, 'No workspace');
    return working
      .filter(visibleOnThisDevice)
      .filter((s) => !s.archived && s.status === 'working' && isForeground(s))
      .map((s) => ({ session: s, workspace: names.get(s.workspace_id) ?? '' }));
  });

  const needs: NeedsRow[] = $derived.by(() => {
    const list = notices ?? [];
    const waiting = list.filter(
      (n) => !n.read && n.kind === 'session' && (n.source_key ?? '').endsWith(':waiting'),
    );
    const alerts = list.filter((n) => !n.read && n.severity !== 'info' && !waiting.includes(n));
    return [
      ...(approvals ?? []).map((a) => ({
        id: `approval:${a.id}`,
        title: a.title,
        detail: a.server_name ? `Approval · ${a.server_name}` : 'Approval',
        icon: 'shield' as const,
        route: 'mcp/activity',
      })),
      ...waiting.map((n) => {
        const sid = sessionIdOf(n);
        return {
          id: `notice:${n.id}`,
          title: n.title,
          detail: 'Waiting for your input',
          icon: 'terminal' as const,
          route: sid ? `agents/${sid}` : 'agents',
        };
      }),
      ...alerts.map((n) => {
        const sid = sessionIdOf(n);
        return {
          id: `notice:${n.id}`,
          title: n.title,
          detail: n.body,
          icon: 'warning' as const,
          route: sid ? `agents/${sid}` : 'settings/notifications',
        };
      }),
    ];
  });

  const today: Notice[] = $derived.by(() => {
    const needIds = new Set(needs.map((r) => r.id));
    return (notices ?? [])
      .filter((n) => isToday(n.created_at) && !needIds.has(`notice:${n.id}`))
      .slice(0, TODAY_MAX);
  });

  // The glyph reports only a COMPLETE, error-free picture: an unavailable
  // list must never read as "no agents need attention".
  $effect(() => {
    if (!loaded || loadError) return;
    void tray.setStatus(running.length, needs.length).catch(() => {});
  });

  /** One list: fetch on the background lane, keep the last good value on
   *  failure (the error shows with Retry). */
  function part<T>(key: string, path: string, set: (v: T) => void) {
    return async (signal: AbortSignal): Promise<boolean> => {
      if (auth.phase !== 'ready') return true;
      try {
        set(await api.bg.get<T>(path, signal));
        if (key in errors) {
          const rest = { ...errors };
          delete rest[key];
          errors = rest;
        }
        return true;
      } catch (e) {
        if (isAbortError(e)) return true;
        errors = { ...errors, [key]: e instanceof Error ? e.message : String(e) };
        return false;
      }
    };
  }

  let pollers: Poller[] = [];
  const refreshAll = (): void => {
    for (const p of pollers) p.now();
  };

  function open(route?: string): void {
    void openInOtto(route).catch(() => {});
  }

  function ask(): void {
    void tray.hidePopover().then(() => bar.show()).catch(() => {});
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') void tray.hidePopover().catch(() => {});
  }

  // Live only once signed in; the boot flow (App.svelte) moves the phase.
  $effect(() => {
    if (auth.phase !== 'ready') return;
    const sock = new TopicSocket(TOPICS);
    sock.start();
    // The tray has its own socket — its dots go stale when THAT one drops.
    sockUp = sock.registry.connected();
    const offConn = sock.registry.onConnection((c) => (sockUp = c));
    // Hidden most of its life, yet its counts drive the menu-bar glyph: with
    // the socket down it keeps the old cadence while hidden (`hidden`).
    const q = (on: readonly string[], run: (s: AbortSignal) => Promise<boolean>) =>
      liveQuery({ run, on, fallbackMs: POLL_MS, hidden: POLL_MS }, sock.registry);
    const ps = [
      q([], part<WorkspaceWithRole[]>('workspaces', '/workspaces', (v) => (workspaces = v))),
      q(
        ['session_status', 'session_created', 'session_removed'],
        part<Session[]>('working', '/sessions?archived=false&status=working', (v) => (working = v)),
      ),
      q(['mcp_approval_changed'], part<McpApproval[]>('approvals', '/mcp/approvals?status=pending', (v) => (approvals = v))),
      q(['notification', 'notifications_changed'], part<Notice[]>('notices', '/notifications', (v) => (notices = v))),
    ];
    pollers = ps;
    return () => {
      for (const p of ps) p.stop();
      offConn();
      sock.stop();
      if (pollers === ps) pollers = [];
    };
  });

  onMount(() => {
    useGlassWindow();
    void shortcuts
      .list()
      .then((all) => {
        const a = all.find((s) => s.id === 'assistant');
        askChord = a?.accel ? prettyChord(a.accel) : '';
      })
      .catch(() => {});
    let unlisten: (() => void) | null = null;
    void onDesktopEvent('otto://tray-shown', refreshAll).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="tray" role="dialog" aria-label="Otto">
  <header class="head">
    <span class="brand"><Icon name="sparkle" size={14} /> Otto</span>
    {#if running.length > 0}
      <span class="pill"><StatusDot state={workingDot} size={6} /> {running.length} running</span>
    {/if}
  </header>

  <button class="ask" onclick={ask}>
    <Icon name="sparkle" size={13} />
    <span class="ask-label">Ask Otto…</span>
    {#if askChord}<kbd>{askChord}</kbd>{/if}
  </button>

  <div class="body">
    {#if loadError}
      <div class="state" role="alert">
        <p>Couldn’t refresh your work: {loadError}</p>
        {#if loaded}<p>Showing the last loaded activity.</p>{/if}
        <button class="btn small" onclick={refreshAll}>Retry</button>
      </div>
    {/if}
    {#if auth.phase === 'loading' || (auth.phase === 'ready' && !loaded && !loadError)}
      <p class="state">Loading your work…</p>
    {:else if auth.phase === 'offline'}
      <div class="state">
        <p>The Otto daemon isn't running yet.</p>
        <button class="btn small" onclick={() => void auth.boot()}>Retry</button>
      </div>
    {:else if auth.phase !== 'ready'}
      <div class="state">
        <p>Sign in to Otto to see your work here.</p>
        <button class="btn small primary" onclick={() => open()}>Open Otto</button>
      </div>
    {:else if loaded}
      <section aria-labelledby="tray-needs">
        <h2 id="tray-needs">Needs you{needs.length ? ` · ${needs.length}` : ''}</h2>
        {#each needs as row (row.id)}
          <button class="row" onclick={() => open(row.route)} title={row.title}>
            <span class="row-icon needs"><Icon name={row.icon} size={13} /></span>
            <span class="row-text">
              <span class="row-title">{row.title}</span>
              {#if row.detail}<span class="row-sub">{row.detail}</span>{/if}
            </span>
          </button>
        {:else}
          <p class="none">Nothing needs you.</p>
        {/each}
      </section>

      <section aria-labelledby="tray-running">
        <h2 id="tray-running">Running{running.length ? ` · ${running.length}` : ''}</h2>
        {#each running as r (r.session.id)}
          <button class="row" onclick={() => open(`agents/${r.session.id}`)} title={r.session.title}>
            <span class="row-icon"><StatusDot state={workingDot} size={7} /></span>
            <span class="row-text">
              <span class="row-title">{r.session.title}</span>
              {#if r.workspace}<span class="row-sub">{r.workspace}</span>{/if}
            </span>
          </button>
        {:else}
          <p class="none">No agents are working.</p>
        {/each}
      </section>

      <section aria-labelledby="tray-today">
        <h2 id="tray-today">Today</h2>
        {#each today as n (n.id)}
          {@const sid = sessionIdOf(n)}
          <button class="row" onclick={() => open(sid ? `agents/${sid}` : undefined)} title={n.title}>
            <span class="row-icon"><Icon name="bell" size={13} /></span>
            <span class="row-text">
              <span class="row-title">{n.title}</span>
              <span class="row-sub">{timeOf(n.created_at)}</span>
            </span>
          </button>
        {:else}
          <p class="none">Nothing yet today.</p>
        {/each}
      </section>
    {/if}
  </div>

  <footer class="foot">
    <button class="btn small primary" onclick={() => open()}>
      <Icon name="external" size={12} /> Open Otto
    </button>
    <button class="btn small ghost" onclick={() => open('settings/appearance')}>
      <Icon name="gear" size={12} /> Settings
    </button>
  </footer>
</div>

<style>
  /* Chrome over the native popover vibrancy: the sidebar glass tint
     (foundations §7), which turns opaque under reduced transparency (the
     system setting or Settings → Appearance) — tokens.css. */
  .tray {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--glass-tint-native);
    color: var(--text);
    font-size: var(--fs-m);
    border-radius: var(--radius-l);
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-block: 12px 8px;
    padding-inline: 14px;
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    color: var(--accent-text);
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ask {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-inline: 10px;
    padding-block: 8px;
    padding-inline: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .ask:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .ask-label {
    flex: 1;
  }
  kbd {
    font-family: var(--font-ui);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-block: 6px;
    padding-inline: 6px;
  }
  section + section {
    margin-block-start: 6px;
  }
  h2 {
    margin: 0;
    padding-block: 8px 4px;
    padding-inline: 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    padding-block: 6px;
    padding-inline: 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .row:hover {
    background: var(--hover);
  }
  .row:focus-visible,
  .ask:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .row-icon {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 18px;
    color: var(--text-dim);
  }
  .row-icon.needs {
    color: var(--warning);
  }
  .row-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .row-title,
  .row-sub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .none {
    margin: 0;
    padding-block: 2px 6px;
    padding-inline: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .state {
    margin: 0;
    padding: 16px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    color: var(--text-dim);
  }
  .state p {
    margin: 0;
  }
  .foot {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding-block: 10px;
    padding-inline: 12px;
    border-block-start: 1px solid var(--separator);
  }
</style>
