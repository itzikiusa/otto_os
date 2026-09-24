// The floating bar's one door to "Ask Otto". Today a request goes through the
// ⌘I plain-English engine (lib/orchestrate.ts: close / send / deterministic
// plan / AI planner). The assistant-threads work swaps the implementation by
// changing the single `ask` binding at the bottom — the bar, its thread UI
// and both hosts only ever see `AskReply`.

import { api } from './api/client';
import type { Action, Id, Session } from './api/types';
import type { BarSpace, TurnTone } from './floatingBar';
import {
  applyClose,
  executePlan,
  plural,
  runEnglish,
  type EnglishOutcome,
  type OrchestrateCtx,
} from './orchestrate';
import { lsGet } from './storage';
import { applyTileOrder } from './stores/splitLayout';
import { layout } from './stores/splitLayout.svelte';
import { isForeground, ws } from './stores/workspace.svelte';

export interface AskReply {
  tone: TurnTone;
  text: string;
  detail?: string;
  /** Where "Open in Otto" goes. */
  route?: string;
  /** Attribution line ("Ask Otto · planner"). */
  source: string;
  /** A plan the person must confirm (Run / Cancel). */
  plan?: Action[];
  /** Sessions a permanent delete would remove (Delete / Cancel). */
  closeIds?: Id[];
}

/** What differs between the main window and the ⌥Space panel. */
export interface AskHost {
  /** Workspace to ask in when the space doesn't pin one. */
  currentWorkspace(): string | null;
  /** Session state for the engine. */
  context(workspaceId: string): Promise<OrchestrateCtx>;
  /** After a plan ran: surface what it created (open tabs, …). */
  afterExecute?(workspaceId: string, created: Id[]): Promise<void> | void;
}

export type AskFn = (text: string, space: BarSpace, host?: AskHost) => Promise<AskReply>;

const SOURCE = 'Ask Otto';
const PLANNER = 'Ask Otto · planner';

/** Orchestrator options the ⌘I sheet toggles (shared, per device). */
function orchestratorPrefs(): { optimize: boolean; aiFallback: boolean } {
  return {
    optimize: lsGet('otto_orch_optimize') === '1',
    aiFallback: lsGet('otto_orch_fallback') !== '0',
  };
}

/** A context built from the API alone — for a window without the workspace
 *  store (the ⌥Space panel) or a space pinned to another workspace. Positions
 *  ("session 2") count foreground agent sessions in list order. */
export async function apiContext(workspaceId: string): Promise<OrchestrateCtx> {
  const sessions = await api.get<Session[]>(`/workspaces/${workspaceId}/sessions`);
  const nameable = sessions.filter((s) => !s.archived && s.kind === 'agent' && isForeground(s));
  return {
    workspaceId,
    focusedSessionId: null,
    sessions,
    nameable,
    order: nameable.map((s) => s.id),
    archive: async (id) => {
      await api.post(`/sessions/${id}/archive`);
    },
    kill: async (id) => {
      await api.del(`/sessions/${id}`);
    },
    ...orchestratorPrefs(),
    confirmDestructive: true,
  };
}

/** On-screen session order in the main window (how the user counts
 *  positions): the tiled grid when tiled, else the side-by-side panes. */
export function paneOrder(): string[] {
  return ws.viewMode === 'tiled' && !ws.maximizedId
    ? applyTileOrder(ws.mainSessions, layout.tileOrder).map((s) => s.id)
    : ws.panes.filter((id) => ws.sessions.some((s) => s.id === id));
}

/** A context from the main window's workspace store (current workspace). */
export function storeContext(workspaceId: string): OrchestrateCtx {
  return {
    workspaceId,
    focusedSessionId: ws.activeSessionId,
    sessions: ws.sessions,
    nameable: ws.plainAgentSessions,
    order: paneOrder(),
    archive: (id) => ws.archiveSession(id),
    kill: (id) => ws.killSession(id),
    ...orchestratorPrefs(),
    confirmDestructive: true,
  };
}

/** The main window's host: the live store for the current workspace, the API
 *  for a space pinned elsewhere; spawned sessions open like a ⌘I plan's. */
export const appHost: AskHost = {
  currentWorkspace: () => ws.currentId,
  context: async (wsId) => (wsId === ws.currentId ? storeContext(wsId) : apiContext(wsId)),
  afterExecute: async (wsId, created) => {
    if (wsId !== ws.currentId || created.length === 0) return;
    await ws.refreshSessions();
    const live = created.filter((id) => ws.sessions.some((s) => s.id === id && !s.archived));
    if (live.length > 1) ws.setViewMode('tiled');
    for (const id of live.slice(0, -1)) ws.openSession(id);
    if (live.length > 0) ws.navigateToSession(live[live.length - 1]);
  },
};

/** The ⌥Space panel's host: everything through the API. */
export const windowHost: AskHost = {
  currentWorkspace: () => lsGet('otto_workspace'),
  context: apiContext,
};

function workspaceFor(space: BarSpace, host: AskHost): string | null {
  return space.workspaceId ?? host.currentWorkspace();
}

async function toReply(
  out: EnglishOutcome,
  workspaceId: string,
  ctx: OrchestrateCtx | null,
  host: AskHost,
): Promise<AskReply> {
  switch (out.kind) {
    case 'empty':
      return { tone: 'info', text: 'Type something to ask.', source: SOURCE };
    case 'closed':
      return {
        tone: 'ok',
        text: out.permanent
          ? `Deleted ${plural(out.count, 'session')}.`
          : `Closed ${plural(out.count, 'session')} — they stay under Agents › Archived.`,
        route: 'agents',
        source: SOURCE,
      };
    case 'confirm-close':
      return {
        tone: 'pending',
        text: `Delete ${plural(out.ids.length, 'session')} for good? Their history goes too.`,
        detail: out.titles.join(', '),
        closeIds: out.ids,
        source: SOURCE,
      };
    case 'nothing-to-close':
      return { tone: 'warn', text: 'No open session matches that.', source: SOURCE };
    case 'no-session':
      return { tone: 'warn', text: 'I couldn’t find that session.', source: SOURCE };
    case 'not-delivered':
      return {
        tone: 'warn',
        text: 'Nobody got it — no running session accepted the message.',
        source: SOURCE,
      };
    case 'sent':
      return {
        tone: 'ok',
        text: `${out.broadcast ? 'Broadcast to' : 'Sent to'} ${plural(out.count, 'session')}.`,
        detail: out.message,
        route: 'agents',
        source: SOURCE,
      };
    case 'unparsed':
      return {
        tone: 'warn',
        text: 'I couldn’t turn that into an action.',
        detail: 'Try “open 2 claude sessions”, or turn on AI fallback in the ⌘I sheet.',
        source: SOURCE,
      };
    case 'plan':
      return {
        tone: out.plan.length > 0 ? 'pending' : 'info',
        text: out.plan.length > 0 ? 'Here’s the plan — run it?' : 'The planner found nothing to do.',
        detail: out.optimizedText ? `Optimized: ${out.optimizedText}` : undefined,
        plan: out.plan,
        source: PLANNER,
      };
    case 'executed': {
      const created = out.results.flatMap((r) => r.session_ids);
      await host.afterExecute?.(workspaceId, created);
      const details = out.results.map((r) => r.detail).filter((d) => d.trim() !== '');
      return {
        tone: out.fail === 0 ? 'ok' : 'warn',
        text:
          out.fail === 0
            ? `Done — ${plural(out.ok, 'action')} completed.`
            : `${out.ok} done, ${out.fail} failed.`,
        detail: details.join(' · ') || undefined,
        route: created.length > 0 ? `agents/${created[created.length - 1]}` : undefined,
        source: ctx ? SOURCE : PLANNER,
      };
    }
  }
}

function failure(e: unknown): AskReply {
  return {
    tone: 'error',
    text: 'That didn’t work.',
    detail: e instanceof Error ? e.message : String(e),
    source: SOURCE,
  };
}

const noWorkspace: AskReply = {
  tone: 'warn',
  text: 'Pick a workspace first.',
  detail: 'Choose one for this space (click its number), or select one in Otto.',
  source: SOURCE,
};

/** Today's implementation: the ⌘I orchestrator. `space.provider/model` are
 *  carried for the assistant threads; the orchestrator plans with its own. */
async function askViaOrchestrator(text: string, space: BarSpace, host: AskHost = windowHost): Promise<AskReply> {
  const wsId = workspaceFor(space, host);
  if (!wsId) return noWorkspace;
  try {
    const ctx = await host.context(wsId);
    return await toReply(await runEnglish(text, ctx), wsId, ctx, host);
  } catch (e) {
    return failure(e);
  }
}

/** Run a plan the person confirmed in the thread. */
export async function confirmPlan(plan: Action[], space: BarSpace, host: AskHost = windowHost): Promise<AskReply> {
  const wsId = workspaceFor(space, host);
  if (!wsId) return noWorkspace;
  try {
    return await toReply(await executePlan(wsId, plan), wsId, null, host);
  } catch (e) {
    return failure(e);
  }
}

/** Run a permanent delete the person confirmed in the thread. */
export async function confirmClose(ids: Id[], space: BarSpace, host: AskHost = windowHost): Promise<AskReply> {
  const wsId = workspaceFor(space, host);
  if (!wsId) return noWorkspace;
  try {
    const ctx = await host.context(wsId);
    const count = await applyClose(ctx, ids, true);
    return await toReply({ kind: 'closed', count, permanent: true }, wsId, ctx, host);
  } catch (e) {
    return failure(e);
  }
}

/** THE adapter. Swap this one line to route the bar to assistant threads. */
export const ask: AskFn = askViaOrchestrator;
