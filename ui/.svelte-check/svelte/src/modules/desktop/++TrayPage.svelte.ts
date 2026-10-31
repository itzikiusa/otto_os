///<reference types="svelte" />
;// Menu-bar popover (`#/tray`, the desktop shell's `otto-tray` panel, 360×520
// over native popover vibrancy). Loaded once, hidden, at app start: it reads
// the existing endpoints — sessions, pending MCP approvals, notifications —
// again only when their events say they changed (see below), lists
// Needs you · Running · Today, and reports the counts to the shell so
// the menu-bar glyph shows a dot while work runs and turns amber when
// something needs you. Rows open the full Otto window at the right route.
// The assistant's own threads/reminders arrive with the assistant API.

import { onMount } from 'svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
// StatusDot + sessionState directly, not LiveWorkingDot: the tray is in the
// entry bundle and LiveWorkingDot's default reads the main window's events
// client, which would drag the whole events/store graph into every window.
import StatusDot from '../../lib/components/StatusDot.svelte';
// Badge is a leaf (type-only import of lib/status): safe in the tray bundle.
import Badge from '../../lib/components/Badge.svelte';
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
import {
    daemonSupervisor,
    offlineDetail,
    supervisorErrorText,
    type DaemonReport,
  } from './daemonState';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const POLL_MS = 20_000;
  const TODAY_MAX = 6;

  // The supervisor's diagnosis + Restart, mirrored from the offline screen
  // (error-handling-5-01): a wedged daemon is recoverable from the menu bar.
  let daemonReport = $state<DaemonReport | null>(null);
  const daemonDetail = $derived(offlineDetail(daemonReport));
  let restarting = $state(false);
  let restartError = $state('');
  async function restartDaemon(): Promise<void> {
    if (restarting) return;
    restarting = true;
    restartError = '';
    try {
      await daemonSupervisor.restart();
      daemonReport = null;
      await auth.boot(true);
    } catch (e) {
      restartError = `Restart failed: ${supervisorErrorText(e)}`;
    } finally {
      restarting = false;
    }
  }

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
    let unlistenDaemon: (() => void) | null = null;
    void onDesktopEvent('otto://tray-shown', refreshAll).then((fn) => (unlisten = fn));
    void daemonSupervisor
      .onState((report) => (daemonReport = report))
      .then((fn) => (unlistenDaemon = fn))
      .catch(() => {});
    void daemonSupervisor
      .status()
      .then((report) => (daemonReport ??= report))
      .catch(() => {});
    return () => {
      unlisten?.();
      unlistenDaemon?.();
    };
  });
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onKey,});}

 { svelteHTML.createElement("div", {     "class":`tray`,"role":`dialog`,"aria-label":`Otto`,});
   { svelteHTML.createElement("header", { "class":`head`,});
     { svelteHTML.createElement("span", { "class":`brand`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":14,}});}  }
    if(running.length > 0){
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}}); { const $$_toDsutatS3C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS3C({ target: __sveltets_2_any(), props: {    "state":workingDot,"size":6,}});} running.length;  Badge}
    }
   }

   { svelteHTML.createElement("button", {   "class":`ask`,"onclick":ask,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":13,}});}
     { svelteHTML.createElement("span", { "class":`ask-label`,});  }
    if(askChord){ { svelteHTML.createElement("kbd", {});askChord; }}
   }

   { svelteHTML.createElement("div", { "class":`body`,});
    if(loadError){
       { svelteHTML.createElement("div", {   "class":`state`,"role":`alert`,});
         { svelteHTML.createElement("p", {});    loadError; }
        if(loaded){ { svelteHTML.createElement("p", {});     }}
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":refreshAll,});  }
       }
    }
    if(auth.phase === 'loading' || (auth.phase === 'ready' && !loaded && !loadError)){
       { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {      "rows":3,"height":32,"label":`your work`,}});}
    } else if (auth.phase === 'offline'){
       { svelteHTML.createElement("div", { "class":`state`,});
         { svelteHTML.createElement("p", {});      }
        if(daemonDetail){ { svelteHTML.createElement("p", { "class":`detail`,});daemonDetail; }}
        if(restartError){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});restartError; }}
         { svelteHTML.createElement("div", { "class":`state-actions`,});
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void auth.boot(),});  }
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void restartDaemon(),"disabled":restarting,});
            restarting ? 'Restarting…' : 'Restart daemon';
           }
         }
       }
    } else if (auth.phase !== 'ready'){
       { svelteHTML.createElement("div", { "class":`state`,});
         { svelteHTML.createElement("p", {});         }
         { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => open(),});  }
       }
    } else if (loaded){
       { svelteHTML.createElement("section", { "aria-labelledby":`tray-needs`,});
         { svelteHTML.createElement("h2", { "id":`tray-needs`,}); needs.length ? ` · ${needs.length}` : ''; }
           for(let row of __sveltets_2_ensureArray(needs)){row.id;
           { svelteHTML.createElement("button", {     "class":`row`,"onclick":() => open(row.route),"title":row.title,});
             { svelteHTML.createElement("span", { "class":`row-icon needs`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":row.icon,"size":13,}});} }
             { svelteHTML.createElement("span", { "class":`row-text`,});
               { svelteHTML.createElement("span", { "class":`row-title`,});row.title; }
              if(row.detail){ { svelteHTML.createElement("span", { "class":`row-sub`,});row.detail; }}
             }
           }
        }
           { svelteHTML.createElement("p", { "class":`none`,});   }
        
       }

       { svelteHTML.createElement("section", { "aria-labelledby":`tray-running`,});
         { svelteHTML.createElement("h2", { "id":`tray-running`,}); running.length ? ` · ${running.length}` : ''; }
           for(let r of __sveltets_2_ensureArray(running)){r.session.id;
           { svelteHTML.createElement("button", {     "class":`row`,"onclick":() => open(`agents/${r.session.id}`),"title":r.session.title,});
             { svelteHTML.createElement("span", { "class":`row-icon`,}); { const $$_toDsutatS5C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS5C({ target: __sveltets_2_any(), props: {    "state":workingDot,"size":7,}});} }
             { svelteHTML.createElement("span", { "class":`row-text`,});
               { svelteHTML.createElement("span", { "class":`row-title`,});r.session.title; }
              if(r.workspace){ { svelteHTML.createElement("span", { "class":`row-sub`,});r.workspace; }}
             }
           }
        }
           { svelteHTML.createElement("p", { "class":`none`,});    }
        
       }

       { svelteHTML.createElement("section", { "aria-labelledby":`tray-today`,});
         { svelteHTML.createElement("h2", { "id":`tray-today`,});  }
           for(let n of __sveltets_2_ensureArray(today)){n.id;
          const sid = sessionIdOf(n);
           { svelteHTML.createElement("button", {     "class":`row`,"onclick":() => open(sid ? `agents/${sid}` : undefined),"title":n.title,});
             { svelteHTML.createElement("span", { "class":`row-icon`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`bell`,"size":13,}});} }
             { svelteHTML.createElement("span", { "class":`row-text`,});
               { svelteHTML.createElement("span", { "class":`row-title`,});n.title; }
               { svelteHTML.createElement("span", { "class":`row-sub`,});timeOf(n.created_at); }
             }
           }
        }
           { svelteHTML.createElement("p", { "class":`none`,});   }
        
       }
    }
   }

   { svelteHTML.createElement("footer", { "class":`foot`,});
     { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => open(),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}  
     }
     { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => open('settings/appearance'),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":12,}});} 
     }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TrayPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TrayPage__SvelteComponent_ = ReturnType<typeof TrayPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default TrayPage__SvelteComponent_;