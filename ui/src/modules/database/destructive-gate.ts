// The query editor's run-time confirm for destructive SQL on connections the
// daemon does not guard. Guarded (Prod / read-only) connections are left to
// their existing typed confirmation — asking twice would teach people to click
// through both. The classifier lives in `destructive-sql.ts`.
import { confirmer } from '../../lib/confirm.svelte';
import { database } from '../../lib/stores/database.svelte';
import { runSafety } from '../../lib/stores/database/runSafety.svelte';
import { destructiveMessage, destructiveStatements } from './destructive-sql';

/** True when `sql` may run: nothing destructive, the preference is off, the
 *  connection is guarded (typed confirm later), or the person confirmed. */
export async function confirmDestructiveRun(sql: string): Promise<boolean> {
  if (!runSafety.confirmDestructive || database.isGuarded) return true;
  const conn = database.selectedConn;
  const risky = destructiveStatements(sql, conn?.kind ?? null);
  if (risky.length === 0) return true;
  return confirmer.ask(destructiveMessage(risky, conn?.name ?? 'this connection'), {
    title: risky.length === 1 ? 'Run a destructive statement?' : 'Run destructive statements?',
    confirmLabel: 'Run',
    cancelLabel: 'Cancel',
    danger: true,
  });
}
