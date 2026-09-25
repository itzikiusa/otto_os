// Agent UI control — this document's view of it (docs/features/agent-ui-control.md).
//
//   • The GRANT lives on the server (`session.meta.ui_control`, set only by
//     `POST /sessions/{id}/ui-control` with a human credential). The session
//     pane's toggle and the inline request prompt read it from the workspace
//     store and flip it here.
//   • REQUESTS: `ui_control_requested` (owner scope) — an agent called a
//     `ui_*` tool without the grant. The session's pane shows "Allow for this
//     session / Deny" until it is answered. A Deny is remembered per session
//     on this device, so a retrying agent can't re-nag: later requests only
//     mark the toggle.
//   • DRIVING: the commands this document ran for an agent (the runtime in
//     lib/uiCommands.ts reports begin/note/end). `AgentDrivingBar` shows who
//     is driving and what they just did for a minute after the last action,
//     plus the last 20 actions.
//   • WRITE MEMORY: "Allow writes on ‹where› for this session", ticked in an
//     agent's write confirm — keyed (session, scope) and dropped with the grant.
//
// Standalone on purpose (no runtime import): lib/uiCommands.ts imports this
// store and installs the Stop hook, never the other way round.

import { untrack } from 'svelte';
import { api } from '../api/client';
import type { Id, OttoEvent, Session, UiAgentRef, UiControlGrant } from '../api/types';
import { lsGet, lsSet } from '../storage';
import { toasts } from '../toast.svelte';
import { ws } from './workspace.svelte';
import { router } from '../router.svelte';
import { isEmbedded } from '../desktop';
import { moduleLabel } from '../sidebar';
import { DRIVING_LINGER_MS, RECENT_MAX, commandLabel, providerName } from '../uiCommands/frames';

export interface UiControlRequest {
  session_id: Id;
  session_title: string;
  module: string;
  command: string;
  /** Epoch ms of the latest request. */
  at: number;
  /** How many times the agent asked since the prompt appeared. */
  count: number;
}

export interface UiRecentAction {
  id: string;
  agent: UiAgentRef;
  command: string;
  label: string;
  module: string;
  at: number;
  /** null while running. */
  ok: boolean | null;
  note?: string;
  error?: string;
}

export interface UiDriving {
  agent: UiAgentRef;
  module: string;
  /** The running action's label (or its latest progress note), null when idle. */
  current: string | null;
  /** Commands running for this agent right now. */
  running: number;
  /** Waiting on a person (a confirm is open). */
  awaitingHuman: boolean;
  /** Epoch ms of the last begin/end. */
  lastAt: number;
}

const DENIED_KEY = 'otto_ui_control_denied';
const WRITES_KEY = 'otto_ui_write_allow';

function readJson<T>(key: string, fallback: T): T {
  const raw = lsGet(key);
  if (!raw) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

/** The grant as the server stored it (anything malformed reads as "not granted"). */
export function grantOf(session: Pick<Session, 'meta'> | null | undefined): UiControlGrant | null {
  const g = session?.meta?.ui_control;
  if (!g || typeof g !== 'object') return null;
  const o = g as Record<string, unknown>;
  return {
    enabled: o.enabled === true,
    granted_at: typeof o.granted_at === 'string' ? o.granted_at : undefined,
    granted_by: typeof o.granted_by === 'string' ? o.granted_by : undefined,
  };
}

/** "Claude · Refactor billing" — how an agent is named in the chrome. */
export function agentName(agent: Pick<UiAgentRef, 'provider' | 'title'>): string {
  const who = providerName(agent.provider);
  const title = agent.title.trim();
  return title ? `${who} · ${title}` : who;
}

class UiControlStore {
  /** Open prompts, by session id. */
  requests: Record<Id, UiControlRequest> = $state({});
  /** Sessions whose request the user denied on this device (no more banners). */
  denied: Id[] = $state(readJson<Id[]>(DENIED_KEY, []).filter((x) => typeof x === 'string'));
  /** A grant change is in flight, by session id. */
  busy: Record<Id, boolean> = $state({});

  /** Who is driving THIS document (null = nobody in the last minute). */
  driving: UiDriving | null = $state(null);
  /** This document's recent agent actions, newest first. */
  recent: UiRecentAction[] = $state([]);

  /** Rows that can host the driving bar (every PageHeader registers); the
   *  first in document order shows it, directly under the page's toolbar. */
  private barHosts: HTMLElement[] = $state([]);

  /** A PageHeader mounted. Call from an $effect (read is untracked). */
  claimBar(el: HTMLElement): () => void {
    this.barHosts = [...untrack(() => this.barHosts), el];
    return () => {
      this.barHosts = untrack(() => this.barHosts).filter((h) => h !== el);
    };
  }

  /** The header hosting the bar; null → the shell shows it above the page. */
  get barOwner(): HTMLElement | null {
    let best: HTMLElement | null = null;
    for (const el of this.barHosts) {
      if (!el.isConnected) continue;
      if (!best || best.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_PRECEDING) best = el;
    }
    return best;
  }

  /** Installed by the runtime: abort this document's commands for a session. */
  private abortSession: ((sessionId: Id, reason: string) => void) | null = null;

  // ── grant ────────────────────────────────────────────────────────────────

  private session(id: Id): Session | null {
    return ws.sessions.find((s) => s.id === id) ?? ws.otherWsSessions.find((s) => s.id === id) ?? null;
  }

  granted(sessionId: Id): boolean {
    return grantOf(this.session(sessionId))?.enabled === true;
  }

  /** The agent asked while the prompt was already answered "Deny" (the toggle
   *  shows a quiet marker instead of a banner). */
  askedAfterDeny(sessionId: Id): boolean {
    return this.denied.includes(sessionId) && !!this.requests[sessionId];
  }

  /** The prompt to show beside a session, if any (never after a Deny). */
  promptFor(sessionId: Id): UiControlRequest | null {
    if (this.denied.includes(sessionId) || this.granted(sessionId)) return null;
    return this.requests[sessionId] ?? null;
  }

  /** Flip the grant. Resolves true when the server accepted it. */
  async setGrant(sessionId: Id, enabled: boolean, opts: { quiet?: boolean } = {}): Promise<boolean> {
    if (this.busy[sessionId]) return false;
    this.busy = { ...this.busy, [sessionId]: true };
    try {
      const updated = await api.post<Session | undefined>(
        `/sessions/${encodeURIComponent(sessionId)}/ui-control`,
        { enabled },
      );
      // The server also broadcasts `session_meta_updated`; apply the returned
      // row now so the toggle flips at once even if the event is late.
      this.patchLocal(sessionId, enabled, updated?.meta);
      if (enabled) {
        this.setDenied(sessionId, false);
        this.dropRequest(sessionId);
      } else {
        this.forgetWrites(sessionId);
        this.abortSession?.(sessionId, 'UI control turned off');
      }
      return true;
    } catch (e) {
      if (!opts.quiet) {
        toasts.error(
          enabled ? "Couldn't allow UI control" : "Couldn't turn off UI control",
          e instanceof Error ? e.message : String(e),
        );
      }
      return false;
    } finally {
      const { [sessionId]: _, ...rest } = this.busy;
      this.busy = rest;
    }
  }

  /** "Allow for this session" on the request prompt. */
  allow(sessionId: Id): Promise<boolean> {
    return this.setGrant(sessionId, true);
  }

  /** "Deny": remembered on this device, and the waiting call is cancelled now
   *  (revoking an absent grant cancels the session's pending commands). */
  async deny(sessionId: Id): Promise<void> {
    this.setDenied(sessionId, true);
    this.dropRequest(sessionId);
    await this.setGrant(sessionId, false, { quiet: true });
  }

  /** The prompt lives in the session's pane; when that pane isn't on screen,
   *  say so once (main document only — the side pane would repeat it). Never
   *  after a Deny, never twice for one open prompt. */
  private announce(ev: Extract<OttoEvent, { type: 'ui_control_requested' }>): void {
    if (isEmbedded || this.denied.includes(ev.session_id) || this.granted(ev.session_id)) return;
    const onScreen = router.module === 'agents' && ws.panes.includes(ev.session_id);
    if (onScreen) return;
    const s = this.session(ev.session_id);
    const who = providerName(s?.provider ?? '');
    const where = ev.module && ev.module !== 'shell' ? ` in ${moduleLabel(ev.module)}` : '';
    toasts.push('info', `${who} wants to drive Otto`, `“${ev.session_title}” asked to ${commandLabel(ev.command).toLowerCase()}${where}.`, 12_000, {
      action: { label: 'Review', run: () => ws.navigateToSession(ev.session_id) },
    });
  }

  private patchLocal(sessionId: Id, enabled: boolean, meta?: Record<string, unknown>): void {
    const patch = (s: Session): Session =>
      s.id !== sessionId
        ? s
        : meta && typeof meta === 'object'
          ? { ...s, meta }
          : { ...s, meta: { ...s.meta, ui_control: { ...(grantOf(s) ?? {}), enabled } } };
    ws.sessions = ws.sessions.map(patch);
    ws.otherWsSessions = ws.otherWsSessions.map(patch);
  }

  private setDenied(sessionId: Id, on: boolean): void {
    const has = this.denied.includes(sessionId);
    if (on === has) return;
    this.denied = on ? [...this.denied, sessionId].slice(-200) : this.denied.filter((x) => x !== sessionId);
    lsSet(DENIED_KEY, JSON.stringify(this.denied));
  }

  private dropRequest(sessionId: Id): void {
    if (!this.requests[sessionId]) return;
    const { [sessionId]: _, ...rest } = this.requests;
    this.requests = rest;
  }

  /** Events this store cares about (the workspace store still applies them). */
  applyEvent(ev: OttoEvent): void {
    if (ev.type === 'ui_control_requested') {
      const prev = this.requests[ev.session_id];
      if (!prev) this.announce(ev);
      this.requests = {
        ...this.requests,
        [ev.session_id]: {
          session_id: ev.session_id,
          session_title: ev.session_title,
          module: ev.module,
          command: ev.command,
          at: Date.now(),
          count: (prev?.count ?? 0) + 1,
        },
      };
    } else if (ev.type === 'session_meta_updated') {
      const g = grantOf({ meta: ev.meta });
      if (g?.enabled) {
        this.dropRequest(ev.session_id);
        this.setDenied(ev.session_id, false);
      } else {
        // Revoked (here, on another device, or by an admin): forget the
        // remembered writes and stop anything still running for it here.
        const had = this.recent.some((r) => r.agent.session_id === ev.session_id && r.ok === null);
        this.forgetWrites(ev.session_id);
        if (had) this.abortSession?.(ev.session_id, 'UI control turned off');
      }
    } else if (ev.type === 'session_removed') {
      this.dropRequest(ev.session_id);
      this.setDenied(ev.session_id, false);
      this.forgetWrites(ev.session_id);
      this.abortSession?.(ev.session_id, 'Session ended');
    }
  }

  // ── write memory ─────────────────────────────────────────────────────────

  private writes(): Record<Id, string[]> {
    const v = readJson<Record<Id, unknown>>(WRITES_KEY, {});
    const out: Record<Id, string[]> = {};
    for (const [k, list] of Object.entries(v ?? {})) {
      if (Array.isArray(list)) out[k] = list.filter((x): x is string => typeof x === 'string');
    }
    return out;
  }

  /** The user ticked "Allow writes on ‹scope› for this session" before. */
  writesAllowed(sessionId: Id, scope: string): boolean {
    return (this.writes()[sessionId] ?? []).includes(scope);
  }

  rememberWrites(sessionId: Id, scope: string): void {
    const all = this.writes();
    const list = all[sessionId] ?? [];
    if (list.includes(scope)) return;
    all[sessionId] = [...list, scope].slice(-100);
    lsSet(WRITES_KEY, JSON.stringify(all));
  }

  forgetWrites(sessionId: Id): void {
    const all = this.writes();
    if (!(sessionId in all)) return;
    delete all[sessionId];
    lsSet(WRITES_KEY, JSON.stringify(all));
  }

  // ── driving (runtime hooks) ──────────────────────────────────────────────

  installAbort(fn: (sessionId: Id, reason: string) => void): void {
    this.abortSession = fn;
  }

  /** Commands counted in `driving.running` (shell commands may not be). */
  private counted = new Set<string>();

  begin(id: string, agent: UiAgentRef, command: string, module: string): void {
    const label = commandLabel(command);
    const at = Date.now();
    this.recent = [{ id, agent, command, label, module, at, ok: null } satisfies UiRecentAction, ...this.recent].slice(0, RECENT_MAX);
    const same = this.driving && this.driving.agent.session_id === agent.session_id;
    // A shell command (look at the window, open a module, focus) doesn't
    // make an agent "drive" this page by itself: opening the side pane from
    // the main document would otherwise flag the Agents page. It shows only
    // while that agent is already driving here.
    if (module === 'shell' && !same) return;
    this.counted.add(id);
    this.driving = {
      agent,
      module: module === 'shell' ? (this.driving?.module ?? module) : module,
      current: label,
      running: (same ? this.driving!.running : 0) + 1,
      awaitingHuman: false,
      lastAt: at,
    };
  }

  note(id: string, note: string, awaitingHuman: boolean): void {
    this.recent = this.recent.map((r) => (r.id === id ? { ...r, note } : r));
    const r = this.recent.find((x) => x.id === id);
    if (this.driving && r && this.driving.agent.session_id === r.agent.session_id) {
      this.driving = { ...this.driving, current: note || this.driving.current, awaitingHuman, lastAt: Date.now() };
    }
  }

  end(id: string, ok: boolean, error?: string): void {
    const r = this.recent.find((x) => x.id === id);
    this.recent = this.recent.map((x) => (x.id === id ? { ...x, ok, error } : x));
    if (!this.counted.delete(id)) return;
    if (!r || !this.driving || this.driving.agent.session_id !== r.agent.session_id) return;
    const running = Math.max(0, this.driving.running - 1);
    this.driving = {
      ...this.driving,
      running,
      awaitingHuman: running > 0 && this.driving.awaitingHuman,
      current: running > 0 ? this.driving.current : null,
      lastAt: Date.now(),
    };
  }

  /** The bar is up: something runs, or the last action was < 1 min ago. Pass
   *  the shared clock (`now()`) so a template re-evaluates as it ticks. */
  visible(nowMs: number): boolean {
    const d = this.driving;
    return !!d && (d.running > 0 || nowMs - d.lastAt < DRIVING_LINGER_MS);
  }

  /** Stop: revoke the grant (the daemon cancels what's pending) and abort
   *  anything still running here. The bar goes away. */
  async stop(): Promise<void> {
    const d = this.driving;
    if (!d) return;
    const sid = d.agent.session_id;
    this.abortSession?.(sid, 'Stopped by you');
    this.driving = null;
    const ok = await this.setGrant(sid, false);
    if (ok) {
      toasts.info('UI control turned off', `${agentName(d.agent)} can't drive Otto until you allow it again.`);
    }
  }

  /** Hide the bar without revoking (the agent keeps its grant). */
  dismiss(): void {
    if (this.driving && this.driving.running === 0) this.driving = null;
  }
}

export const uiControl = new UiControlStore();
