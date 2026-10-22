// The reset dry-run's refusal warning (api-contracts-5-02). The server returns
// the plan PLUS `warning` (the same text as the 409) when the real reset would
// be refused — e.g. the group still has live consumers. The preview shows it
// and the Reset action stays off while it holds, so nobody types the
// confirmation only to be refused.
import type { DryRunResp } from './types';

/** The warning that blocks resetting `group` from `preview`, or null. A
 *  preview for another group (stale after a switch) never blocks. */
export function resetWarning(preview: DryRunResp | null, group: string | null): string | null {
  if (!preview || preview.group !== group) return null;
  const w = preview.warning?.trim();
  return w ? w : null;
}
