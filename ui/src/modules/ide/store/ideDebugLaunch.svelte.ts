// Debug launch life for the IDE: one live session the person can see, never an
// invisible second one; a start failure that can be shown inline with a next
// step; and a session left behind on project switch stopped on the daemon.
//
// - `ideDebugLaunch.startError`: a failure raised BEFORE a session exists (the
//   start request itself was refused). Failures of a session that did start
//   arrive on the snapshot (`error`, `error_kind`, `error_ecosystem`). The
//   debugger panel renders one inline card from either source.
// - `guardDebugRelaunch`: every debug launch asks first while a session is live.

import { api, ApiError } from '../../../lib/api/client';
import { confirmer, type ChoiceOption } from '../../../lib/confirm.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { toasts } from '../../../lib/toast.svelte';
import type { IdeDebugSessionSnapshot } from './ideTypes';

export type IdeDebugErrorKind = 'adapter_missing' | 'toolchain_missing' | 'build_failed';

export interface IdeDebugStartError {
  /** Human message (no `not found:` / `invalid:` prefix). */
  message: string;
  /** Next-step class, when known. */
  kind: IdeDebugErrorKind | null;
  /** Toolchain ecosystem the failure is about (`go`, `python`, …), when known. */
  ecosystem: string | null;
}

class IdeDebugLaunchState {
  startError = $state<IdeDebugStartError | null>(null);
}

export const ideDebugLaunch = new IdeDebugLaunchState();

const PREFIX = /^(not found|invalid|upstream|conflict|internal|forbidden):\s*/i;

/** The inline-card shape of a refused start request (HTTP 4xx/5xx, no session). */
export function debugStartErrorFrom(e: unknown, ecosystem: string | null = null): IdeDebugStartError {
  const raw = e instanceof ApiError && e.status !== 401 && e.status !== 404 ? e.message : loadErrorText(e);
  return { message: (raw || 'The debugger couldn’t start.').replace(PREFIX, ''), kind: null, ecosystem };
}

export function isLiveDebug(snap: Pick<IdeDebugSessionSnapshot, 'state'> | null | undefined): snap is IdeDebugSessionSnapshot {
  return Boolean(snap && (snap.state === 'starting' || snap.state === 'running' || snap.state === 'paused'));
}

const VERBS: Record<string, string> = {
  continue: 'resume',
  pause: 'pause',
  step_over: 'step over',
  step_into: 'step into',
  step_out: 'step out',
  run_to_cursor: 'run to cursor',
  select_frame: 'switch to that frame',
  restart: 'restart the debug session',
  stop: 'stop the debug session',
};

/** "Couldn’t run to cursor" — the person's verb for a debugger command. */
export function debugActionVerb(action: string): string {
  return VERBS[action] ?? action.replaceAll('_', ' ');
}

const STOP_WAIT_MS = 4000;
const STOP_POLL_MS = 150;

/** Stop session `id` on the daemon and wait (bounded) until it has ended.
 *  Resolves the final snapshot when known; `ended: false` when it is still live. */
export async function stopDebugSessionAndWait(
  id: string,
  opts: { waitMs?: number; pollMs?: number } = {},
): Promise<{ ended: boolean; snap: IdeDebugSessionSnapshot | null }> {
  const path = `/ide/debug/sessions/${encodeURIComponent(id)}`;
  let snap: IdeDebugSessionSnapshot | null = null;
  try {
    snap = (await api.post<{ session: IdeDebugSessionSnapshot }>(`${path}/command`, { action: 'stop' })).session;
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) return { ended: true, snap: null };
    throw e;
  }
  const deadline = Date.now() + (opts.waitMs ?? STOP_WAIT_MS);
  while (isLiveDebug(snap) && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, opts.pollMs ?? STOP_POLL_MS));
    try {
      snap = (await api.get<{ session: IdeDebugSessionSnapshot }>(path)).session;
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) return { ended: true, snap: null };
      throw e;
    }
  }
  return { ended: !isLiveDebug(snap), snap };
}

function sessionLabel(snap: IdeDebugSessionSnapshot): string {
  // "Debug review.py (python-bdb-dap)" → "review.py"
  const t = (snap.title || '').replace(/\s*\([^)]*\)\s*$/, '').replace(/^Debug\s+/, '').trim();
  return t || 'The current debug session';
}

const STOP_AND_RELAUNCH = 'stop-relaunch';

/**
 * The one gate every debug launch goes through (toolbar, ⌃D, gutter Debug,
 * "Save & debug", repo suites, restart of a finished session). With a live
 * session it asks "Stop and debug again" / Cancel; on stop it waits for the
 * daemon to end the old session before saying yes. Resolves whether to launch.
 */
export async function guardDebugRelaunch(
  host: { activeDebug: IdeDebugSessionSnapshot | null; collabIsGuest?: boolean },
  onSnapshot?: (snap: IdeDebugSessionSnapshot) => void,
  deps: {
    choose?: (message: string, opts: { title: string; options: ChoiceOption[] }) => Promise<{ value: string | null }>;
    stop?: typeof stopDebugSessionAndWait;
  } = {},
): Promise<boolean> {
  const live = host.activeDebug;
  if (host.collabIsGuest || !isLiveDebug(live)) return true;
  const choose = deps.choose ?? ((m, o) => confirmer.choose(m, o));
  const where = live.state === 'paused' ? 'is paused' : 'is still running';
  const pick = await choose(`${sessionLabel(live)} ${where}. Stop it and start a new debug session?`, {
    title: 'Debug session running',
    options: [{ label: 'Stop and debug again', value: STOP_AND_RELAUNCH, kind: 'primary' }],
  });
  if (pick.value !== STOP_AND_RELAUNCH) return false;
  try {
    const { ended, snap } = await (deps.stop ?? stopDebugSessionAndWait)(live.id);
    if (snap) onSnapshot?.(snap);
    if (!ended) {
      toasts.error('Couldn’t stop the running debug session', 'It is still running, so a new one was not started.');
      return false;
    }
    return true;
  } catch (e) {
    toasts.error('Couldn’t stop the running debug session', loadErrorText(e));
    return false;
  }
}

/** Project switch: a session left live in the project being closed is stopped
 *  on the daemon (best-effort, no dialog — a paused process holding a port in
 *  a project you left is a leak). A collab guest never stops the host's session. */
export function leaveDebugSessionOnSwitch(snap: IdeDebugSessionSnapshot | null, isGuest: boolean): void {
  ideDebugLaunch.startError = null;
  if (isGuest || !isLiveDebug(snap)) return;
  void api.post(`/ide/debug/sessions/${encodeURIComponent(snap.id)}/command`, { action: 'stop' }).catch(() => {});
}
