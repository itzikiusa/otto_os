// Query tab labels (lib/stores/database/statement.ts `tabLabel`): a Mongo
// `db.<collection>.op(…)` tab is named after its collection, not "db".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tabLabel } from '../src/lib/stores/database/statement.ts';

const tab = (statement: string, name: string | null = null) => ({ name, statement });

test('Mongo tabs are named after their collection', () => {
  assert.equal(tabLabel(tab('db.orders.find({ status: "paid" })'), 0), 'orders');
  assert.equal(tabLabel(tab('db.users.aggregate([])'), 1), 'users');
  assert.equal(tabLabel(tab('db.getCollectionNames()'), 2), 'db');
});

test('SQL tabs keep their table and verb labels', () => {
  assert.equal(tabLabel(tab('SELECT * FROM shop.orders'), 0), 'orders');
  assert.equal(tabLabel(tab('DELETE FROM t WHERE id = 1'), 0), 'DELETE t');
  assert.equal(tabLabel(tab('SHOW PROCESSLIST'), 0), 'SHOW');
  assert.equal(tabLabel(tab('  '), 3), 'Query 4');
  assert.equal(tabLabel(tab('select 1', 'mine'), 0), 'mine');
});
