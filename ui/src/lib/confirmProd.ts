// One confirm shape for actions that WRITE to (or delete from) a production
// target — a prod queue, bucket, topic, host or SFTP server. The twin of
// `confirmOutward`: the same Where / What lines, plus ONE consistent
// "PRODUCTION" line (the EnvBadge wording), danger styling, and — for the
// destructive verbs — an opt-in typed confirmation (design guidelines patterns.md §5:
// the most dangerous dialogs ask for the most deliberate answer).
//
// Non-prod targets still get the same shape (no PRODUCTION line, no typed
// gate), so a call site can use it unconditionally instead of branching.

import { confirmer } from './confirm.svelte';
import { envTone } from './status';
import { toasts } from './toast.svelte';

export interface ProdConfirm {
  /** The target's environment label ("prod", "staging"…); see `envTone`. */
  env: string | null | undefined;
  /** Destination: "SQS queue orders-dlq · eu-west-1", "Topic payments.v1 · kafka-prod". */
  where: string;
  /** The action as the confirm button reads it: "Delete message", "Produce". */
  verb: string;
  /** What changes — a short preview or a size/key summary. */
  what?: string;
  /** Text the user must type to proceed, e.g. the queue / topic / object name.
   *  Omit for reversible actions; pass it only when prod to gate prod alone. */
  typed?: string;
  /** Dialog title; defaults to `${verb} on production?` (prod) or `${verb}?`. */
  title?: string;
  /** Red confirm button. Defaults to true on prod, false elsewhere. */
  danger?: boolean;
}

const PREVIEW_MAX = 400;

/** True when `env` names a production target. */
export function isProdEnv(env: string | null | undefined): boolean {
  return envTone(env).key === 'prod';
}

/** Build the dialog body (exported for tests / previews). */
export function prodMessage(o: Pick<ProdConfirm, 'env' | 'where' | 'what' | 'typed'>): string {
  const prod = isProdEnv(o.env);
  const lines: string[] = [];
  if (prod) lines.push('PRODUCTION — this changes live data.');
  lines.push(`Where: ${o.where}`);
  if (o.what) {
    const w = o.what.trim();
    lines.push(`What: ${w.length > PREVIEW_MAX ? `${w.slice(0, PREVIEW_MAX - 1).trimEnd()}…` : w}`);
  }
  if (o.typed) lines.push(`Type “${o.typed}” to confirm.`);
  return lines.join('\n\n');
}

/** Ask before a write against a (possibly) production target. Resolves true
 *  when the user confirms — and, for a typed confirm, typed the name exactly. */
export async function confirmProd(o: ProdConfirm): Promise<boolean> {
  const prod = isProdEnv(o.env);
  const danger = o.danger ?? prod;
  const title = o.title ?? (prod ? `${o.verb} on production?` : `${o.verb}?`);
  const message = prodMessage(o);
  if (o.typed) {
    const v = await confirmer.promptText(message, {
      title,
      confirmLabel: o.verb,
      placeholder: o.typed,
      danger,
    });
    if (v === null) return false;
    if (v !== o.typed) {
      toasts.info('Nothing was changed', 'What you typed didn’t match, so the action was canceled.');
      return false;
    }
    return true;
  }
  return confirmer.ask(message, { title, confirmLabel: o.verb, danger });
}
