<script lang="ts">
  // Menu-bar popover (`#/tray`, the desktop shell's `otto-tray` panel, 360×520
  // over native popover vibrancy). Loaded once, hidden, at app start: it polls
  // the existing endpoints — sessions, pending MCP approvals, notifications —
  // lists Needs you · Running · Today, and reports the counts to the shell so
  // the menu-bar glyph shows a dot while work runs and turns amber when
  // something needs you. Rows open the full Otto window at the right route.
  // The assistant's own threads/reminders arrive with the assistant API.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { api } from '../../lib/api/client';
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

  let running: RunningRow[] = $state([]);
  let needs: NeedsRow[] = $state([]);
  let today: Notice[] = $state([]);
  let loaded = $state(false);
  let loadError: string | null = $state(null);
  let askChord = $state('');
  let inflight = false;

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

  async function sessionsOf(wsId: string): Promise<Session[]> {
    try {
      // Honour "Isolate sessions to this device", like the sidebar does.
      return (await api.get<Session[]>(`/workspaces/${wsId}/sessions`)).filter(visibleOnThisDevice);
    } catch {
      return [];
    }
  }

  async function refresh(): Promise<void> {
    if (inflight || auth.phase !== 'ready') return;
    inflight = true;
    try {
      const workspaces = await api.get<WorkspaceWithRole[]>('/workspaces');
      const names = new Map(workspaces.map((w) => [w.id, w.name]));
      names.set(SCRATCH_WORKSPACE_ID, 'No workspace');
      const [lists, approvals, notices] = await Promise.all([
        Promise.all([...names.keys()].map((id) => sessionsOf(id))),
        api.get<McpApproval[]>('/mcp/approvals?status=pending').catch(() => [] as McpApproval[]),
        api.get<Notice[]>('/notifications').catch(() => [] as Notice[]),
      ]);

      running = lists
        .flat()
        .filter((s) => !s.archived && s.status === 'working' && isForeground(s))
        .map((s) => ({ session: s, workspace: names.get(s.workspace_id) ?? '' }));

      const waiting = notices.filter(
        (n) => !n.read && n.kind === 'session' && (n.source_key ?? '').endsWith(':waiting'),
      );
      const alerts = notices.filter(
        (n) => !n.read && n.severity !== 'info' && !waiting.includes(n),
      );
      needs = [
        ...approvals.map((a) => ({
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
      const needIds = new Set(needs.map((r) => r.id));
      today = notices
        .filter((n) => isToday(n.created_at) && !needIds.has(`notice:${n.id}`))
        .slice(0, TODAY_MAX);

      loadError = null;
      loaded = true;
      void tray.setStatus(running.length, needs.length).catch(() => {});
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      inflight = false;
    }
  }

  function open(route?: string): void {
    void openInOtto(route).catch(() => {});
  }

  function ask(): void {
    void tray.hidePopover().then(() => bar.show()).catch(() => {});
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') void tray.hidePopover().catch(() => {});
  }

  // Poll only once signed in; the boot flow (App.svelte) moves the phase.
  $effect(() => {
    if (auth.phase !== 'ready') return;
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
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
    void onDesktopEvent('otto://tray-shown', () => void refresh()).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="tray" role="dialog" aria-label="Otto">
  <header class="head">
    <span class="brand"><Icon name="sparkle" size={14} /> Otto</span>
    {#if running.length > 0}
      <span class="pill"><StatusDot status="working" size={6} /> {running.length} running</span>
    {/if}
  </header>

  <button class="ask" onclick={ask}>
    <Icon name="sparkle" size={13} />
    <span class="ask-label">Ask Otto…</span>
    {#if askChord}<kbd>{askChord}</kbd>{/if}
  </button>

  <div class="body">
    {#if auth.phase === 'loading' || (auth.phase === 'ready' && !loaded && !loadError)}
      <p class="state">Loading…</p>
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
    {:else if loadError && !loaded}
      <div class="state">
        <p>Couldn't load your work: {loadError}</p>
        <button class="btn small" onclick={() => void refresh()}>Retry</button>
      </div>
    {:else}
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
            <span class="row-icon"><StatusDot status="working" size={7} /></span>
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
    letter-spacing: 0.04em;
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
    outline: 2px solid var(--accent);
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
