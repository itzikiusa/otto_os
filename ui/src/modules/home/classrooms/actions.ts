// The headmaster's powers over a student, as PURE wiring with injected
// effects (unit/classrooms.test.ts drives it with fakes):
//
//  • Kick out = DELETE the session through the app's one delete path
//    (`ws.killSession` → `DELETE /sessions/{id}`), after a danger confirm that
//    names the session, its workspace and that the history goes with it —
//    stronger when the agent is mid-turn. The walk-out animation plays while
//    the delete is in flight; a failed delete puts the student back.
//  • Detention = ARCHIVE (`ws.requestArchive`, which toasts its own Undo):
//    resumable, so it asks nothing — EXCEPT for a working agent, whose
//    in-flight turn archive stops (Undo can't bring that back), where the
//    store's working-guard confirms like closing a busy tab does.
//
// Both refuse outright when the caller can't manage the session (viewer role
// or no Agents:Edit) — the UI hides the actions too; the daemon re-checks.

import type { Student } from './model.ts';

export interface ConfirmOpts {
  title: string;
  confirmLabel: string;
  danger: boolean;
}

export interface KickDeps {
  /** `confirmer.ask` — the in-app danger confirm. */
  ask(message: string, opts: ConfirmOpts): Promise<boolean>;
  /** The app's delete path (`ws.killSession`). */
  kill(id: string): Promise<void>;
  /** Walk-out animation (resolves at once under reduced motion). */
  animate?(id: string): Promise<void>;
  /** Undo the animation when the delete failed. */
  restore?(id: string): void;
  done(title: string, body?: string): void;
  failed(title: string, e: unknown): void;
}

export interface DetentionDeps {
  /** The app's guarded archive path (`ws.requestArchive`: confirms a working
   *  agent, toasts its own Undo). Resolves false when the user cancelled. */
  archive(id: string): Promise<boolean | void>;
  failed(title: string, e: unknown): void;
}

type Target = Pick<Student, 'id' | 'title' | 'workspaceName' | 'visual' | 'canManage'> & Partial<Pick<Student, 'background' | 'source'>>;

/** Confirm copy for kicking `s` out (exported for the tests and the list). */
export function kickOutPrompt(s: Target): { message: string; opts: ConfirmOpts } {
  const where = s.workspaceName ? ` from “${s.workspaceName}”` : '';
  const midTurn =
    s.visual === 'working'
      ? ' It is mid-turn right now: the agent is stopped immediately and its in-flight work is lost.'
      : '';
  // A back-row student belongs to an engine (workflow step, swarm, review…):
  // deleting it pulls the session out from under that run — say so up front.
  const engine = s.background
    ? ` It is a running ${s.source ?? 'engine'} session, not one you started: the ${s.source ?? 'engine'} run that owns it loses it and may fail.`
    : '';
  return {
    message: `Kick “${s.title}” out${where}? The session is deleted together with its entire history. This can’t be undone — there is no Undo.${midTurn}${engine}`,
    opts: { title: s.visual === 'working' ? 'Kick out a working agent' : 'Kick out student', confirmLabel: 'Kick out', danger: true },
  };
}

/** Returns true when the session was deleted. */
export async function kickOut(s: Target, deps: KickDeps): Promise<boolean> {
  if (!s.canManage) return false;
  const { message, opts } = kickOutPrompt(s);
  if (!(await deps.ask(message, opts))) return false;
  const [err] = await Promise.all([
    deps.kill(s.id).then(
      () => null,
      (e: unknown) => e ?? new Error('delete failed'),
    ),
    (deps.animate?.(s.id) ?? Promise.resolve()).catch(() => undefined),
  ]);
  if (err) {
    deps.restore?.(s.id);
    deps.failed(`Couldn’t kick out “${s.title}”`, err);
    return false;
  }
  deps.done(`Kicked out ${s.title}`, 'The session and its history were deleted.');
  return true;
}

/** Returns true when the session was archived. */
export async function sendToDetention(s: Target, deps: DetentionDeps): Promise<boolean> {
  if (!s.canManage) return false;
  try {
    return (await deps.archive(s.id)) !== false;
  } catch (e) {
    deps.failed(`Couldn’t send “${s.title}” to detention`, e);
    return false;
  }
}
