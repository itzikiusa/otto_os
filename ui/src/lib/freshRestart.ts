// "Would this restart lose the conversation?" — the guard every restart Otto
// makes as a side effect of a settings change (model / effort / mode chip,
// folders, MCP tools) runs first. Muse and opencode (and a codex/agy session
// whose id was never captured) start a NEW conversation on every respawn, so
// those restarts ask; everything else keeps today's behaviour. Pure (no Svelte
// state): the dialog and `/meta` are handed in, so unit tests drive it directly.

import type { MetaResp, Session, SessionStatus } from './api/types';
import type { ChoiceOption } from './confirm.svelte';
import { harnessLabel } from './providerRegistry.ts';

export type RestartChoice = 'restart' | 'save' | 'cancel';

type ResumeMeta = Pick<MetaResp, 'provider_resumable'> | null | undefined;
type SessionLike = Pick<Session, 'kind' | 'provider' | 'provider_session_id'>;

/** The dialog surface used here (`confirmer` in the app). */
export interface RestartDialog {
  ask(message: string, opts?: { title?: string; confirmLabel?: string; danger?: boolean }): Promise<boolean>;
  choose(
    message: string,
    opts: { title?: string; options: ChoiceOption[] },
  ): Promise<{ value: string | null; remember: boolean }>;
}

/** `/meta` as the auth store holds it. Registered by `stores/auth.svelte`, so
 *  the session store needs no auth import; unset (tests) = unknown. */
let metaSource: () => ResumeMeta = () => null;
export function registerMetaSource(get: () => ResumeMeta): void {
  metaSource = get;
}
export function currentMeta(): ResumeMeta {
  return metaSource();
}

/** Whether restarting `s` reopens its conversation. `null` = unknown (an
 *  older daemon without `provider_resumable`, a provider it doesn't list, a
 *  shell): callers keep today's behaviour. */
export function restartResumes(s: SessionLike, meta: ResumeMeta): boolean | null {
  if (s.kind !== 'agent' || s.provider === 'shell') return null;
  const flag = meta?.provider_resumable?.[s.provider];
  if (flag === undefined) return null;
  return flag && Boolean(s.provider_session_id);
}

export const FRESH_RESTART_TITLE = 'Restart and start a new conversation?';

export function freshRestartMessage(provider: string, canSaveOnly: boolean): string {
  const base = `${harnessLabel(provider)} can’t resume this session’s current conversation, so restarting it starts a new one.`;
  return canSaveOnly ? `${base} Or save the change without restarting — it applies to the next session.` : base;
}

/** Ask before a restart that would start a new conversation. `canSaveOnly`
 *  offers "Save without restarting" (the setting applies to the next session).
 *  Resolves `'restart'` at once when the restart keeps the conversation (or
 *  that is unknown). */
export async function confirmFreshRestart(
  s: SessionLike,
  dialog: RestartDialog,
  opts: { canSaveOnly?: boolean; meta?: ResumeMeta } = {},
): Promise<RestartChoice> {
  const meta = opts.meta === undefined ? currentMeta() : opts.meta;
  if (restartResumes(s, meta) !== false) return 'restart';
  const message = freshRestartMessage(s.provider, Boolean(opts.canSaveOnly));
  if (!opts.canSaveOnly) {
    const ok = await dialog.ask(message, { title: FRESH_RESTART_TITLE, confirmLabel: 'Restart', danger: true });
    return ok ? 'restart' : 'cancel';
  }
  const { value } = await dialog.choose(message, {
    title: FRESH_RESTART_TITLE,
    options: [
      { label: 'Restart', value: 'restart', kind: 'danger' },
      { label: 'Save without restarting', value: 'save', kind: 'normal' },
    ],
  });
  return value === 'restart' || value === 'save' ? value : 'cancel';
}

/** `updateSessionConfig`'s gate: only a change that WOULD respawn the
 *  session (a live, idle/running agent) is asked about. */
export async function respawnChoice(
  s: (SessionLike & Pick<Session, 'archived' | 'status'>) | null,
  status: SessionStatus | undefined,
  wantsRespawn: boolean,
  dialog: RestartDialog,
): Promise<RestartChoice> {
  if (!s || !wantsRespawn || s.archived) return 'restart';
  const st = status ?? s.status;
  if (st !== 'running' && st !== 'idle') return 'restart';
  return confirmFreshRestart(s, dialog, { canSaveOnly: true });
}

/** A "Save & restart" button: the caller's working-guard first, then the
 *  can't-resume ask. `true` = restart, `false` = save only, `null` = cancel. */
export async function saveAndRestartChoice(
  s: SessionLike | null,
  workingGuard: () => Promise<boolean>,
  dialog: RestartDialog,
): Promise<boolean | null> {
  if (!(await workingGuard())) return null;
  if (!s) return true;
  const choice = await confirmFreshRestart(s, dialog, { canSaveOnly: true });
  return choice === 'cancel' ? null : choice === 'restart';
}
