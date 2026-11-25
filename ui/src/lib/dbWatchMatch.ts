// Which detected names belong to a DB Watch target's engine — shared by the IDE
// panel (modules/ide/dbwatch) and the agent-debug `db` command (uiCommands/ide.ts).
import type { IdeDbEngine, IdeDetectedDbAccess } from './api/types/ide-db-watch';

const SQL_ENGINES: ReadonlySet<string> = new Set(['mysql', 'postgres', 'clickhouse']);

/** Detection names any SQL string `mysql`; on a SQL target the target's own
 *  verification (`verified` set) decides, whatever SQL engine it is. */
export function matchesEngine(d: Pick<IdeDetectedDbAccess, 'engine' | 'verified'>, engine: IdeDbEngine | string): boolean {
  return d.engine === engine || (SQL_ENGINES.has(d.engine) && SQL_ENGINES.has(engine) && d.verified != null);
}
