import { test, expect } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Unit-level coverage of the PURE query-filter module, exercised in the real
// browser by importing it straight from the Vite dev server (no DB needed — just
// the page origin). This covers the WHERE / find-filter splicer branches that
// power the "Query by value" / "Add to query" cell actions (set/and, tail
// preservation, OR-precedence incl. no-space `OR(`, escaping, IS NULL,
// multi-statement decline, Mongo merge/$oid/regex), so the integrated UI spec
// (desktop-db-context-actions) can stay focused on wiring.
//
// The SQL path reuses the store's `splitStatement`/`rewriteWhere` (the same
// parser the quick-filter chips use), so its output is the house multi-line
// `\nWHERE …` form — these assertions pin that exactly.
//
// Desktop-browser project only (one run is enough for pure logic).
// ─────────────────────────────────────────────────────────────────────────────

// Local structural type for the module under test. We deliberately do NOT use
// `typeof import('../src/modules/database/query-filter')` here: that would make
// the e2e tsconfig's plain `tsc` type-check query-filter.ts and, transitively,
// the `.svelte.ts` store it composes — whose `$state`/`$derived` runes plain tsc
// can't resolve. The runtime `import()` below still loads the real module via
// Vite (app-side svelte-check type-checks it with rune support).
type FilterMode = 'set' | 'and';
type FilterEngine = 'mysql' | 'clickhouse' | 'mongodb';
interface QF {
  applySqlFilter(sql: string, column: string, value: unknown, mode: FilterMode): string | null;
  applyMongoFilter(sql: string, col: string, value: unknown, mode: FilterMode): string | null;
  buildFilteredQuery(
    engine: FilterEngine,
    sql: string,
    col: string,
    value: unknown,
    mode: FilterMode,
  ): string | null;
}
const MOD = '/src/modules/database/query-filter.ts';

test.describe('query-filter (pure)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
    // Land on the Vite-served module URL itself (correct origin, but NO SPA
    // bootstrap) so the page never client-side-navigates and destroys the
    // evaluate execution context mid-`import()`.
    await page.goto(MOD);
  });

  test('applySqlFilter: set adds WHERE to a no-WHERE query (the canonical example)', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      return m.applySqlFilter('select * from x', 'a', 'b', 'set');
    }, MOD);
    expect(out).toBe("select * from x\nWHERE `a` = 'b'");
  });

  test('applySqlFilter: and appends onto an existing WHERE (the chaining example)', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      const step1 = m.applySqlFilter('select * from x', 'a', 'b', 'set')!;
      const step2 = m.applySqlFilter(step1, 'b', 'c', 'and');
      return { step1, step2 };
    }, MOD);
    expect(out.step1).toBe("select * from x\nWHERE `a` = 'b'");
    expect(out.step2).toBe("select * from x\nWHERE `a` = 'b' AND `b` = 'c'");
  });

  test('applySqlFilter: preserves ORDER BY / LIMIT / GROUP BY / FORMAT tails', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      return {
        orderLimit: m.applySqlFilter('SELECT * FROM t ORDER BY x LIMIT 10', 'a', '1', 'set'),
        andTail: m.applySqlFilter('SELECT * FROM t WHERE a=1 ORDER BY x LIMIT 10', 'b', 2, 'and'),
        group: m.applySqlFilter('SELECT a, count(*) FROM t GROUP BY a', 'a', '1', 'set'),
        chFormat: m.applySqlFilter('SELECT * FROM t LIMIT 5 FORMAT JSON', 'a', '1', 'set'),
      };
    }, MOD);
    expect(out.orderLimit).toBe("SELECT * FROM t\nWHERE `a` = '1'\nORDER BY x LIMIT 10");
    expect(out.andTail).toBe("SELECT * FROM t\nWHERE a=1 AND `b` = 2\nORDER BY x LIMIT 10");
    expect(out.group).toBe("SELECT a, count(*) FROM t\nWHERE `a` = '1'\nGROUP BY a");
    expect(out.chFormat).toBe("SELECT * FROM t\nWHERE `a` = '1'\nLIMIT 5 FORMAT JSON");
  });

  test('applySqlFilter: parenthesizes an existing OR before AND (precedence safety)', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      return {
        spaced: m.applySqlFilter('SELECT * FROM t WHERE a=1 OR a=2', 'b', 3, 'and'),
        // OR with no surrounding spaces must still be detected (word boundary, not whitespace).
        noSpace: m.applySqlFilter('SELECT * FROM t WHERE a=1 OR(b=2)', 'c', 3, 'and'),
        // A column whose name merely CONTAINS "or" must NOT be mistaken for an OR.
        colWithOr: m.applySqlFilter('SELECT * FROM t WHERE color=1', 'c', 2, 'and'),
      };
    }, MOD);
    expect(out.spaced).toBe('SELECT * FROM t\nWHERE (a=1 OR a=2) AND `b` = 3');
    expect(out.noSpace).toBe('SELECT * FROM t\nWHERE (a=1 OR(b=2)) AND `c` = 3');
    expect(out.colWithOr).toBe('SELECT * FROM t\nWHERE color=1 AND `c` = 2');
  });

  test('applySqlFilter: ignores keywords inside strings and subqueries', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      return {
        str: m.applySqlFilter("SELECT * FROM t WHERE name = 'ORDER BY hack'", 'a', '1', 'and'),
        sub: m.applySqlFilter('SELECT * FROM t WHERE id IN (SELECT id FROM u WHERE x=1) ORDER BY y', 'a', 2, 'and'),
      };
    }, MOD);
    expect(out.str).toBe("SELECT * FROM t\nWHERE name = 'ORDER BY hack' AND `a` = '1'");
    expect(out.sub).toBe('SELECT * FROM t\nWHERE id IN (SELECT id FROM u WHERE x=1) AND `a` = 2\nORDER BY y');
  });

  test('applySqlFilter: escaping (apostrophe), number vs string, NULL → IS NULL', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      return {
        quote: m.applySqlFilter('select * from x', 'name', "O'Brien", 'set'),
        num: m.applySqlFilter('select * from x', 'age', 30, 'set'),
        isNull: m.applySqlFilter('select * from x', 'a', null, 'set'),
      };
    }, MOD);
    expect(out.quote).toBe("select * from x\nWHERE `name` = 'O''Brien'");
    expect(out.num).toBe('select * from x\nWHERE `age` = 30');
    expect(out.isNull).toBe('select * from x\nWHERE `a` IS NULL');
  });

  test('applySqlFilter: declines (null) on unparseable / multi-statement input', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      return {
        noFrom: m.applySqlFilter('DESCRIBE t', 'a', '1', 'set'),
        multi: m.applySqlFilter('SET x=1; SELECT * FROM t', 'a', '1', 'set'),
      };
    }, MOD);
    expect(out.noFrom).toBeNull();
    expect(out.multi).toBeNull();
  });

  test('applyMongoFilter: set/and on empty, non-empty, no-arg find; _id, number, non-find', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      return {
        emptySet: m.applyMongoFilter('db.coll.find({})', 'a', 'b', 'set'),
        emptyAnd: m.applyMongoFilter('db.coll.find({})', 'a', 'b', 'and'),
        noArg: m.applyMongoFilter('db.coll.find()', 'a', 'b', 'set'),
        merge: m.applyMongoFilter('db.coll.find({ "a": "b" })', 'b', 'c', 'and'),
        replace: m.applyMongoFilter('db.coll.find({ "a": "b" })', 'a', 'x', 'set'),
        num: m.applyMongoFilter('db.coll.find({})', 'age', 30, 'set'),
        oid: m.applyMongoFilter('db.coll.find({})', '_id', '507f1f77bcf86cd799439011', 'set'),
        regex: m.applyMongoFilter('db.coll.find({ a: /}/ })', 'b', 'c', 'and'),
        notFind: m.applyMongoFilter('db.coll.aggregate([])', 'a', 'b', 'set'),
      };
    }, MOD);
    expect(out.emptySet).toBe('db.coll.find({ "a": "b" })');
    expect(out.emptyAnd).toBe('db.coll.find({ "a": "b" })');
    expect(out.noArg).toBe('db.coll.find({ "a": "b" })');
    expect(out.merge).toBe('db.coll.find({ "a": "b", "b": "c" })');
    expect(out.replace).toBe('db.coll.find({ "a": "x" })');
    expect(out.num).toBe('db.coll.find({ "age": 30 })');
    expect(out.oid).toBe('db.coll.find({ "_id": {"$oid": "507f1f77bcf86cd799439011"} })');
    expect(out.regex).toBe('db.coll.find({ a: /}/, "b": "c" })');
    expect(out.notFind).toBeNull();
  });

  test('buildFilteredQuery: dispatches per engine; declines on empty / non-find', async ({ page }) => {
    const out = await page.evaluate(async (src) => {
      const m = (await import(/* @vite-ignore */ src)) as QF;
      return {
        mysql: m.buildFilteredQuery('mysql', 'select * from x', 'a', 'b', 'set'),
        clickhouse: m.buildFilteredQuery('clickhouse', 'select * from x', 'a', 1, 'set'),
        mongo: m.buildFilteredQuery('mongodb', 'db.coll.find({})', 'a', 'b', 'set'),
        emptyBase: m.buildFilteredQuery('mysql', '   ', 'a', 'b', 'set'),
        mongoAgg: m.buildFilteredQuery('mongodb', 'db.coll.aggregate([])', 'a', 'b', 'set'),
      };
    }, MOD);
    expect(out.mysql).toBe("select * from x\nWHERE `a` = 'b'");
    expect(out.clickhouse).toBe('select * from x\nWHERE `a` = 1');
    expect(out.mongo).toBe('db.coll.find({ "a": "b" })');
    expect(out.emptyBase).toBeNull();
    expect(out.mongoAgg).toBeNull();
  });
});
