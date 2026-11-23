// Stop a runtime thread's background tasks (one, or every one) and remember
// each row's answer for this tab: Agents & tasks, the queue bar's "Stop all"
// and a child-thread card read it. The words live in taskStop.ts.
import { api } from '../../../lib/api/client';
import type { FamilyStopResp, RuntimeInterruptResp, RuntimeTaskStopResp, RuntimeTasksStopResp } from '../../../lib/api/types';
import { toasts } from '../../../lib/toast.svelte';
import { stopFailure, stopStateOf, stopSummary, type StopState } from './taskStop';

const key = (sessionId: string, taskId: string): string => `${sessionId}\u0000${taskId}`;
const base = (sessionId: string): string => `/runtime-sessions/${encodeURIComponent(sessionId)}/tasks`;

export interface StopTally {
  stopping: number;
  asked: number;
  failed: number;
}

class TaskStops {
  byKey = $state<Record<string, StopState>>({});
  /** This daemon has no per-task stop route (an older build): no Stop shows. */
  unsupported = $state(false);

  get(sessionId: string, taskId: string): StopState | null {
    return this.byKey[key(sessionId, taskId)] ?? null;
  }

  private set(sessionId: string, taskId: string, s: StopState): void {
    this.byKey = { ...this.byKey, [key(sessionId, taskId)]: s };
  }

  private noteUnsupported(): void {
    if (this.unsupported) return;
    this.unsupported = true;
    toasts.info('This Otto build can’t stop background tasks', 'Ask the agent to stop it, or close the thread to end all of its work.');
  }

  /** Stop one task. The row shows the answer; nothing throws. */
  async stop(sessionId: string, taskId: string): Promise<void> {
    if (this.get(sessionId, taskId)?.phase === 'stopping') return;
    this.set(sessionId, taskId, { phase: 'stopping' });
    try {
      const r = await api.post<RuntimeTaskStopResp>(`${base(sessionId)}/${encodeURIComponent(taskId)}/stop`);
      this.set(sessionId, taskId, stopStateOf(r.outcome));
    } catch (e) {
      const f = stopFailure(e);
      if (f.unsupported) {
        this.forget(sessionId, taskId);
        this.noteUnsupported();
      } else this.set(sessionId, taskId, f.state);
    }
  }

  /** Stop every task in `taskIds` (the thread's running background list) in
   *  one call; returns the tally (null when this daemon cannot stop tasks). */
  async stopAll(sessionId: string, taskIds: readonly string[]): Promise<StopTally | null> {
    for (const id of taskIds) this.set(sessionId, id, { phase: 'stopping' });
    try {
      const r = await api.post<RuntimeTasksStopResp>(`${base(sessionId)}/stop`);
      const tally: StopTally = { stopping: 0, asked: 0, failed: 0 };
      for (const res of r.results ?? []) {
        if ('error' in res) {
          tally.failed++;
          this.set(sessionId, res.task_id, { phase: 'error', error: res.error });
        } else {
          if (res.outcome === 'asked') tally.asked++;
          else tally.stopping++;
          this.set(sessionId, res.task_id, stopStateOf(res.outcome));
        }
      }
      return tally;
    } catch (e) {
      const f = stopFailure(e);
      for (const id of taskIds) {
        if (f.unsupported) this.forget(sessionId, id);
        else this.set(sessionId, id, f.state);
      }
      if (f.unsupported) {
        this.noteUnsupported();
        return null;
      }
      return f.state.phase === 'gone' ? { stopping: 0, asked: 0, failed: 0 } : { stopping: 0, asked: 0, failed: taskIds.length };
    }
  }

  forget(sessionId: string, taskId: string): void {
    const k = key(sessionId, taskId);
    if (!(k in this.byKey)) return;
    const { [k]: _gone, ...rest } = this.byKey;
    this.byKey = rest;
  }
}

export const taskStops = new TaskStops();

/** Stop what a thread is actually doing (a child card's Stop, an Agents &
 *  tasks child row): its turn through interrupt, its background work through
 *  the task stop — and say what happened, never a silent no-op. */
export async function stopThreadWork(id: string, work: { turnRunning: boolean; taskIds: readonly string[] }): Promise<void> {
  let turn = false;
  if (work.turnRunning) {
    const r = await api.post<RuntimeInterruptResp | undefined>(`/runtime-sessions/${encodeURIComponent(id)}/interrupt`);
    turn = r?.interrupted !== false;
  }
  const tally = work.taskIds.length ? await taskStops.stopAll(id, work.taskIds) : { stopping: 0, asked: 0, failed: 0 };
  if (tally === null) return; // the unsupported note already showed
  const s = stopSummary({ ...tally, turn });
  if (s.ok) toasts.info(s.title, s.body);
  else toasts.error(s.title, s.body);
}

/** One Stop for a thread and its subagents (`POST …/stop-family`): its
 *  turn and every open subagent's turn and background work; `background`
 *  (the waiting line's Stop) also its own background work. The composer's
 *  Stop uses it; a Stop that found nothing running says so. */
export async function stopFamily(id: string, background = false): Promise<void> {
  const r = await api.post<FamilyStopResp | undefined>(`/runtime-sessions/${encodeURIComponent(id)}/stop-family`, { background });
  if (r && r.turns === 0 && r.tasks === 0) toasts.info('Nothing was running');
}

/** The composer's Stop. */
export const interruptTurn = (id: string): Promise<void> => stopFamily(id);
