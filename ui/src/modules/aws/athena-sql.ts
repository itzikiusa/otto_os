// Athena statement classification for the run gate. Athena runs DDL (`DROP
// TABLE`, `ALTER`, `MSCK REPAIR`, CTAS) and DML (`INSERT INTO`, `UPDATE` /
// `DELETE` / `MERGE` on Iceberg) through the same StartQueryExecution as a
// SELECT, so the editor confirms anything that is not plainly a read — typed on
// a production account. The daemon re-checks with the same rule (`confirm`).

/** Leading keywords of a statement that only reads. */
const READ_KEYWORDS = new Set(['SELECT', 'WITH', 'SHOW', 'DESCRIBE', 'DESC', 'VALUES', 'TABLE']);

/** Drop leading whitespace, `-- …` / `/* … *\/` comments and opening parens. */
function stripLead(sql: string): string {
  let s = sql;
  for (;;) {
    const t = s.replace(/^[\s(]+/, '');
    if (t.startsWith('--')) {
      const nl = t.indexOf('\n');
      s = nl < 0 ? '' : t.slice(nl + 1);
    } else if (t.startsWith('/*')) {
      const end = t.indexOf('*/', 2);
      s = end < 0 ? '' : t.slice(end + 2);
    } else {
      return t;
    }
  }
}

/** The statement's first keyword, upper-cased (`''` for an empty statement). */
export function athenaLeadKeyword(sql: string): string {
  return (/^[A-Za-z_]+/.exec(stripLead(sql))?.[0] ?? '').toUpperCase();
}

/** True when the statement only reads. `EXPLAIN` is classified by what it
 *  explains — `EXPLAIN ANALYZE INSERT …` EXECUTES the insert. Anything not
 *  recognised (empty, unknown keyword) counts as a write: confirm-by-default. */
export function athenaIsRead(sql: string): boolean {
  let rest = stripLead(sql);
  for (;;) {
    const kw = athenaLeadKeyword(rest);
    if (kw === 'EXPLAIN' || kw === 'ANALYZE' || kw === 'VERBOSE') {
      rest = rest.slice(kw.length).trimStart();
      // `EXPLAIN (TYPE LOGICAL, FORMAT JSON) …` — skip the option list (not
      // a parenthesised query, which `stripLead` unwraps).
      if (kw === 'EXPLAIN' && /^\(\s*(TYPE|FORMAT)\b/i.test(rest)) {
        const close = rest.indexOf(')');
        rest = close < 0 ? '' : rest.slice(close + 1);
      }
      rest = stripLead(rest);
      continue;
    }
    return READ_KEYWORDS.has(kw);
  }
}
