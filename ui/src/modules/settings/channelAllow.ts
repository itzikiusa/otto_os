// Settings → Channels: the "recently turned away" senders a listener reports
// (`ListenerStatus.rejected_senders`) and adding one to an integration's
// comma-separated `allowed_users`. Pure — unit-tested (unit/channelAllow.test.ts).

import type { RejectedSender } from '../../lib/api/types';

/** The ids in a comma-separated allow-list (trimmed, blanks dropped). */
export function allowedIds(allowed: string): string[] {
  return allowed
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s !== '');
}

/** Rejected senders not already on the allow-list (a sender allowed since
 *  they were turned away stays listed by the daemon until it restarts). */
export function pendingRejected(rejected: RejectedSender[] | undefined, allowed: string): RejectedSender[] {
  const ids = new Set(allowedIds(allowed));
  return (rejected ?? []).filter((r) => r.user.trim() !== '' && !ids.has(r.user.trim()));
}

/** `allowed` with `user` appended (no duplicate, normalized `a, b` spacing). */
export function withAllowed(allowed: string, user: string): string {
  const ids = allowedIds(allowed);
  const id = user.trim();
  if (id !== '' && !ids.includes(id)) ids.push(id);
  return ids.join(', ');
}

/** "@alice (123456)" — or just the id when the platform sent no name. */
export function senderLabel(r: RejectedSender): string {
  const name = r.name?.trim();
  return name ? `${name} (${r.user})` : r.user;
}
