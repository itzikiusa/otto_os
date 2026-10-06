// Otto School — the headmaster's powers, as PURE wiring with injected effects
// (unit/school-actions.test.ts drives it with fakes):
//
//  • Kick out = DELETE the session through the app's one delete path
//    (`ws.killSession` → `DELETE /sessions/{id}`), after a danger confirm that
//    names the session, its workspace and that the history goes with it —
//    stronger when the agent is mid-turn or owned by an engine. The walk-out
//    plays while the delete is in flight; a failed delete walks the kid back.
//  • Detention = ARCHIVE (`ws.requestArchive`: confirms a working agent, toasts
//    its own Undo); the kid walks to the bench at the back of the room.
//  • Release = UNARCHIVE (`ws.unarchiveSession`): back to a desk.
//
// All refuse outright when the caller can't manage the session (viewer role or
// no Agents:Edit) — the UI disables them with the reason; the daemon re-checks.

import { sourceLabel, type Kid } from './model.ts';

export interface ConfirmOpts {
  title: string;
  confirmLabel: string;
  danger: boolean;
}

type Target = Pick<Kid, 'id' | 'title' | 'workspaceName' | 'pose' | 'canManage' | 'background' | 'source'>;

export interface KickDeps {
  ask(message: string, opts: ConfirmOpts): Promise<boolean>;
  kill(id: string): Promise<void>;
  /** Walk-out animation (resolves at once under reduced motion). */
  animate?(id: string): Promise<void>;
  restore?(id: string): void;
  done(title: string, body?: string): void;
  failed(title: string, e: unknown): void;
}

export interface ArchiveHint {
  working: boolean;
  title: string;
  engine: string | null;
}

export interface DetentionDeps {
  /** `ws.requestArchive` — resolves false when the user cancelled its guard. */
  archive(id: string, hint: ArchiveHint): Promise<boolean | void>;
  animate?(id: string): Promise<void>;
  restore?(id: string): void;
  failed(title: string, e: unknown): void;
}

export interface ReleaseDeps {
  unarchive(id: string): Promise<void>;
  done(title: string): void;
  failed(title: string, e: unknown): void;
}

export function kickOutPrompt(s: Target): { message: string; opts: ConfirmOpts } {
  const where = s.workspaceName ? ` from “${s.workspaceName}”` : '';
  const midTurn = s.pose === 'working' ? ' It is mid-turn right now: the agent is stopped immediately and its in-flight work is lost.' : '';
  const engine = s.background
    ? ` It is a running ${sourceLabel(s.source)} session, not one you started: the ${sourceLabel(s.source)} run that owns it loses it and may fail.`
    : '';
  return {
    message: `Kick “${s.title}” out${where}? The session is deleted together with its entire history. This can’t be undone — there is no Undo.${midTurn}${engine}`,
    opts: { title: s.pose === 'working' ? 'Kick out a working agent' : 'Kick out of school', confirmLabel: 'Kick out', danger: true },
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

/** Returns true when the session was archived. The walk to the bench plays
 *  while the archive runs; a cancelled guard or a failure walks the kid back. */
export async function sendToDetention(s: Target, deps: DetentionDeps): Promise<boolean> {
  if (!s.canManage) return false;
  const hint: ArchiveHint = { working: s.pose === 'working', title: s.title, engine: s.background ? sourceLabel(s.source) : null };
  const [res] = await Promise.all([
    deps.archive(s.id, hint).then(
      (r) => (r === false ? 'cancelled' : 'ok'),
      (e: unknown) => e ?? new Error('archive failed'),
    ),
    (deps.animate?.(s.id) ?? Promise.resolve()).catch(() => undefined),
  ]);
  if (res === 'ok') return true;
  deps.restore?.(s.id);
  if (res !== 'cancelled') deps.failed(`Couldn’t send “${s.title}” to detention`, res);
  return false;
}

/** Returns true when the session left detention (unarchived). */
export async function release(s: Pick<Kid, 'id' | 'title' | 'canManage'>, deps: ReleaseDeps): Promise<boolean> {
  if (!s.canManage) return false;
  try {
    await deps.unarchive(s.id);
    deps.done(`${s.title} is back at a desk`);
    return true;
  } catch (e) {
    deps.failed(`Couldn’t release “${s.title}” from detention`, e);
    return false;
  }
}
