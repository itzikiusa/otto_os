// The headmaster's powers over a student, as PURE wiring with injected
// effects (unit/classrooms.test.ts drives it with fakes):
//
//  • Kick out = DELETE the session through the app's one delete path
//    (`ws.killSession` → `DELETE /sessions/{id}`), after a danger confirm that
//    names the session, its workspace and that the history goes with it —
//    stronger when the agent is mid-turn. The walk-out animation plays while
//    the delete is in flight; a failed delete puts the student back.
//  • Detention = ARCHIVE (`ws.archiveSession`, which toasts its own Undo):
//    resumable, so it asks nothing.
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
  confirm(message: string, opts: ConfirmOpts): Promise<boolean>;
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
  /** The app's archive path (`ws.archiveSession`, toasts its own Undo). */
  archive(id: string): Promise<void>;
  failed(title: string, e: unknown): void;
}

type Target = Pick<Student, 'id' | 'title' | 'workspaceName' | 'visual' | 'canManage'>;

/** Confirm copy for kicking `s` out (exported for the tests and the list). */
export function kickOutPrompt(s: Target): { message: string; opts: ConfirmOpts } {
  const where = s.workspaceName ? ` from “${s.workspaceName}”` : '';
  const midTurn =
    s.visual === 'working'
      ? ' It is mid-turn right now: the agent is stopped immediately and its in-flight work is lost.'
      : '';
  return {
    message: `Kick “${s.title}” out${where}? The session is deleted together with its entire history. This can’t be undone — there is no Undo.${midTurn}`,
    opts: { title: s.visual === 'working' ? 'Kick out a working agent' : 'Kick out student', confirmLabel: 'Kick out', danger: true },
  };
}

/** Returns true when the session was deleted. */
export async function kickOut(s: Target, deps: KickDeps): Promise<boolean> {
  if (!s.canManage) return false;
  const { message, opts } = kickOutPrompt(s);
  if (!(await deps.confirm(message, opts))) return false;
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
    await deps.archive(s.id);
    return true;
  } catch (e) {
    deps.failed(`Couldn’t send “${s.title}” to detention`, e);
    return false;
  }
}
