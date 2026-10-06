import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { DbColumnDef } from '../src/lib/api/types.ts';
import { designerSql, rowsFromColumns, seedDefault, type DesignerRow } from '../src/modules/database/table-designer-sql.ts';

// The Table Designer's ALTER generators against real `information_schema` /
// catalog shapes (S16-302): MySQL reports defaults UNQUOTED, and its
// CHANGE COLUMN restates the whole column.

const col = (c: Partial<DbColumnDef> & { name: string; data_type: string }): DbColumnDef => ({ nullable: true, ...c });

function alter(engine: 'mysql' | 'postgres' | 'clickhouse', columns: DbColumnDef[], edit: (rows: DesignerRow[]) => void): string {
  const rows = rowsFromColumns(engine, columns);
  edit(rows);
  return designerSql({ engine, tableRef: engine === 'postgres' ? '"public"."t"' : '`shop`.`t`', columns, rows, indexes: [], fks: [] });
}

test('MySQL: string defaults are re-emitted as literals on rename', () => {
  const status = col({ name: 'status', data_type: 'varchar(20)', nullable: false, default: 'pending', collation: 'utf8mb4_0900_ai_ci' });
  const sql = alter('mysql', [status], (r) => (r[0].name = 'state'));
  assert.equal(sql, "ALTER TABLE `shop`.`t`\n  CHANGE COLUMN `status` `state` varchar(20) COLLATE utf8mb4_0900_ai_ci NOT NULL DEFAULT 'pending';");
  // A space and a quote inside the default survive too.
  assert.equal(seedDefault('mysql', col({ name: 'x', data_type: 'varchar(20)', default: "in progress's" })), "'in progress''s'");
});

test('MySQL: an empty-string default is kept, not dropped', () => {
  const note = col({ name: 'note', data_type: 'varchar(50)', nullable: false, default: '' });
  const sql = alter('mysql', [note], (r) => (r[0].notNull = false));
  assert.match(sql, /CHANGE COLUMN `note` `note` varchar\(50\) NULL DEFAULT '';$/m);
});

test('MySQL: expression, timestamp and numeric defaults', () => {
  assert.equal(seedDefault('mysql', col({ name: 'id', data_type: 'char(36)', default: 'uuid()', extra: 'DEFAULT_GENERATED' })), '(uuid())');
  assert.equal(
    seedDefault('mysql', col({ name: 'at', data_type: 'timestamp', default: 'CURRENT_TIMESTAMP', extra: 'DEFAULT_GENERATED on update CURRENT_TIMESTAMP' })),
    'CURRENT_TIMESTAMP',
  );
  assert.equal(seedDefault('mysql', col({ name: 'n', data_type: 'int', default: '0' })), '0');
  assert.equal(seedDefault('mysql', col({ name: 'f', data_type: 'bit(3)', default: "b'101'" })), "b'101'");
  // A numeric-looking default on a text column stays a string.
  assert.equal(seedDefault('mysql', col({ name: 's', data_type: 'varchar(5)', default: '42' })), "'42'");
  assert.equal(seedDefault('mysql', col({ name: 's', data_type: 'varchar(5)', default: null })), '');
  // The expression default + ON UPDATE are restated (DEFAULT_GENERATED is not DDL).
  const at = col({ name: 'at', data_type: 'timestamp', nullable: false, default: 'CURRENT_TIMESTAMP', extra: 'DEFAULT_GENERATED on update CURRENT_TIMESTAMP' });
  assert.equal(
    alter('mysql', [at], (r) => (r[0].name = 'updated_at')),
    'ALTER TABLE `shop`.`t`\n  CHANGE COLUMN `at` `updated_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP on update CURRENT_TIMESTAMP;',
  );
});

test('MySQL: collation and comment are restated; untouched columns emit nothing', () => {
  const code = col({ name: 'code', data_type: 'varchar(10)', default: null, collation: 'utf8mb4_bin', comment: "it's C:\\x" });
  const sql = alter('mysql', [code], (r) => (r[0].name = 'sku'));
  assert.equal(sql, "ALTER TABLE `shop`.`t`\n  CHANGE COLUMN `code` `sku` varchar(10) COLLATE utf8mb4_bin NULL COMMENT 'it''s C:\\\\x';");
  // Changing the type to a non-string type drops the (now invalid) collation.
  assert.doesNotMatch(alter('mysql', [code], (r) => (r[0].type = 'int')), /COLLATE/);
  // Seeding then generating with no edit is a no-op even for '' / quoted defaults.
  assert.equal(alter('mysql', [code, col({ name: 'n', data_type: 'varchar(5)', default: '' })], () => {}), '');
});

test('Postgres and ClickHouse defaults are already expressions', () => {
  const pg = col({ name: 'status', data_type: 'character varying', nullable: false, default: "'pending'::character varying" });
  assert.equal(
    alter('postgres', [pg], (r) => (r[0].name = 'state')),
    'ALTER TABLE "public"."t" RENAME COLUMN "status" TO "state";',
  );
  assert.equal(
    alter('postgres', [pg], (r) => (r[0].def = '')),
    'ALTER TABLE "public"."t" ALTER COLUMN "status" DROP DEFAULT;',
  );
  const ch = col({ name: 'ts', data_type: 'DateTime', default: 'now()' });
  assert.equal(
    alter('clickhouse', [ch], (r) => (r[0].type = "DateTime('UTC')")),
    "ALTER TABLE `shop`.`t`\n  MODIFY COLUMN `ts` DateTime('UTC') DEFAULT now();",
  );
});
