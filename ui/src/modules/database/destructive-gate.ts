// The query editor's run-time confirms, asked BEFORE the text is sent:
//  - a run that opens a transaction and leaves it open (MySQL / PostgreSQL):
//    its session is closed when the run ends, so the transaction is rolled
//    back — asked on every connection (`tx-shape.ts`);
//  - destructive SQL on connections the daemon does not guard. Guarded (Prod /
//    read-only) connections are left to their existing typed confirmation —
//    asking twice would teach people to click through both
//    (`destructive-sql.ts`; the per-device preference in `runSafety`).
import { confirmer } from '../../lib/confirm.svelte';
import { database } from '../../lib/stores/database.svelte';
import { runSafety } from '../../lib/stores/database/runSafety.svelte';
import { destructiveMessage, destructiveStatements } from './destructive-sql';
import { txShape, txUnclosedWarning } from './tx-shape';

/** True when `sql` may run: nothing to warn about, or the person confirmed. */
export async function confirmRun(sql: string): Promise<boolean> {
  const conn = database.selectedConn;
  const kind = conn?.kind ?? null;
  if ((kind === 'mysql' || kind === 'postgres') && txShape(sql, kind)?.unclosed) {
    const ok = await confirmer.ask(txUnclosedWarning(kind), {
      title: 'The transaction will be rolled back',
      confirmLabel: 'Run anyway',
      cancelLabel: 'Cancel',
      danger: false,
    });
    if (!ok) return false;
  }
  if (!runSafety.confirmDestructive || database.isGuarded) return true;
  const risky = destructiveStatements(sql, kind);
  if (risky.length === 0) return true;
  return confirmer.ask(destructiveMessage(risky, conn?.name ?? 'this connection'), {
    title: risky.length === 1 ? 'Run a destructive statement?' : 'Run destructive statements?',
    confirmLabel: 'Run',
    cancelLabel: 'Cancel',
    danger: true,
  });
}
