// A runtime thread's queued messages above its composer (RuntimeQueueBar):
// which actions each row offers, and the composer's words when the daemon
// refuses a send. A message whose delivery failed STAYS queued with its
// `error` and the drain skips it: it shows the reason with Retry (re-queue),
// Edit and Remove. Pure (unit/threadsTaskStop.test.ts).
import type { QueuedPrompt } from '../../../lib/api/types.ts';

export type QueuedAction = 'retry' | 'steer' | 'edit' | 'remove';

export interface QueuedRow {
  /** "Couldn’t send: <why>", or null. */
  error: string | null;
  actions: QueuedAction[];
}

export function queuedRow(q: Pick<QueuedPrompt, 'error'>, ctx: { running: boolean; canSteer: boolean }): QueuedRow {
  const why = q.error?.trim();
  if (why) return { error: `Couldn’t send: ${why}`, actions: ['retry', 'edit', 'remove'] };
  return { error: null, actions: [...(ctx.running && ctx.canSteer ? (['steer'] as const) : []), 'edit', 'remove'] };
}

/** The composer's line for a refused send (400 / 409 / 413), in words. The
 *  queue holds at most 20 messages and 64 MiB of attachments; one message
 *  at most 32 MB. The daemon's own reason wins when it names the queue. */
export function sendRefusal(status: number, message: string): string {
  if (status === 413) {
    return /queue/i.test(message) ? 'The queue is full of attachments (64 MiB). Send or remove a queued message first.' : 'Message is over the 32 MB send limit. Remove an attachment and send again.';
  }
  if (status === 409 && /queue/i.test(message) && /full|20|limit|cap/i.test(message)) {
    return 'The queue already holds 20 messages. Send or remove one first.';
  }
  return message;
}
