// SQL dialects for CodeMirror's lang-sql, shared by CodeEditor (tokenizing)
// and the DB query editor (which dialect a connection's engine speaks). The
// dialect decides what the editor sees as a string or a comment: StandardSQL
// has no `\'` escapes and no `#` comments, so MySQL data like `'O\'Brien'`
// ends the string early and inverts every later string/code boundary.

import { SQLDialect, MySQL, PostgreSQL, StandardSQL } from '@codemirror/lang-sql';

/** SQL dialects the editor can tokenize `.sql` docs with. */
export type SqlDialectName = 'standard' | 'mysql' | 'postgres' | 'clickhouse';

// lang-sql's own MySQL dialect leaves `backslashEscapes` off, so it still
// ends `'O\'Brien'` at the `\'`. MySQL (default sql_mode) and ClickHouse both
// treat a backslash as an escape inside string literals.
const MySQLEscapes = SQLDialect.define({ ...MySQL.spec, backslashEscapes: true });

// ClickHouse: MySQL-like strings (`\'` escapes) and `#` comments, but a double
// quote quotes an IDENTIFIER (as in standard SQL), not a string.
const ClickHouseSQL = SQLDialect.define({
  ...MySQL.spec,
  backslashEscapes: true,
  doubleQuotedStrings: false,
  identifierQuotes: '`"',
});

const DIALECTS: Record<SqlDialectName, SQLDialect> = {
  standard: StandardSQL,
  mysql: MySQLEscapes,
  postgres: PostgreSQL,
  clickhouse: ClickHouseSQL,
};

/** The lang-sql dialect object for a dialect name (StandardSQL when unknown). */
export function sqlDialect(name: SqlDialectName): SQLDialect {
  return DIALECTS[name] ?? StandardSQL;
}

/** The dialect a DB connection kind speaks (`standard` for anything else). */
export function sqlDialectForKind(kind: string | null | undefined): SqlDialectName {
  switch (kind) {
    case 'mysql':
      return 'mysql';
    case 'postgres':
      return 'postgres';
    case 'clickhouse':
      return 'clickhouse';
    default:
      return 'standard';
  }
}
