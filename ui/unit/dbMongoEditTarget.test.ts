import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { loadSource } from './sourceHarness.ts';

// Compile the production adapter, including its target parser. The tiny formatting
// dependencies do not participate in target detection or write identity.
// The JSON-tree helpers (expansion-plan, bson) live in lib/jsonTree/.
const JSON_TREE = /^(?:\.\/|\.\.\/\.\.\/lib\/jsonTree\/)(expansion-plan|bson)$/;
function pureModule(name: string, dir = '../src/modules/database/'): Record<string, any> {
  const context = { exports: {} as Record<string, any>, require: (path: string) =>
    JSON_TREE.test(path)
      ? pureModule(JSON_TREE.exec(path)![1], '../src/lib/jsonTree/')
      : path === './edit-sql'
      ? pureModule('edit-sql', dir)
      : path === '../../lib/database/sql-dialect.ts'
      ? pureModule('sql-dialect', '../src/lib/database/')
      : ({cellStr:String, isComplex:(v: unknown)=>v!==null && typeof v==='object', compactJson:JSON.stringify}) };
  runInNewContext(ts.transpileModule(readFileSync(new URL(`${dir}${name}.ts`, import.meta.url), 'utf8'),
    { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return context.exports;
}
function adapter(): Record<string, any> { return pureModule('edit-mongo'); }

test('Mongo editing rejects transformed identities and a later operation in a method chain', () => {
  const { mongoAdapter } = adapter();
  for (const statement of [
    'db.users.find({}, {_id: {$literal: "other-user"}, name: 1})',
    'db.users.find({}).projection({_id: "$other_id", name: 1})',
    'db.users.find({}).aggregate([{$project: {_id: "$other_id", name: 1}}])',
    'db.users.find({}).find({}, {_id: {$literal: "other-user"}})',
    'db.users.find({}).explain()',
    'db.users.find({}, {"profile.name": 1})',
    'db.users.find({}, {"items.$": 1})',
    'db.users.find({}); db.others.find({})',
    'SELECT other_id AS _id, name FROM users',
    'SELECT _id, upper(name) AS name FROM users',
  ]) assert.equal(mongoAdapter.target(statement, ['_id', 'name'], {activeDb:'app'}).target, null, statement);
});

test('Mongo editing keeps plain finds, scalar projections and sort/limit controls', () => {
  const { mongoAdapter } = adapter();
  for (const statement of [
    'db.users.find({})',
    'db.users.find({name: "comma, paren)"}, {_id: 1, name: true})',
    'db.users.find({_id: ObjectId("507f1f77bcf86cd799439011")}).sort({name: 1}).limit(20)',
    'db.users.find({}).projection({"name": 1, "email": 1}).limit(5);',
    'db.users.find({}, {privateField: 0})',
    'SELECT _id, name FROM users',
    "SELECT * FROM users WHERE name = 'before'",
  ]) assert.equal(mongoAdapter.target(statement, ['_id', 'name'], {activeDb:'app'}).target?.table, 'users', statement);
});

test('document edits preserve fields absent from a projected result', () => {
  const { mongoAdapter } = adapter();
  const ctx = {target:{table:'users'},columns:[{name:'_id'},{name:'name'},{name:'old'}],liveRows:[['source','Before','remove']]};
  const result = mongoAdapter.buildReplace(0,{_id:'source',name:'After',added:7},ctx);
  assert.equal(result.sql, 'db.users.updateOne({"_id": "source"}, {"$set":{"name":"After","added":7},"$unset":{"old":""}})');
  assert.equal(mongoAdapter.buildReplace(0,{_id:'source',name:'Before',old:'remove'},ctx),null);
});

test('lossy results clear stale edit targets; row pagination and ordinary marker text remain editable', async () => {
  const mongo = adapter();
  const { EditFlow } = loadSource(new URL('../src/modules/database/EditFlow.svelte.ts', import.meta.url), {
    '../../lib/toast.svelte': {}, '../../lib/stores/database.svelte': {database:{}},
    './results-format': {}, './edit-types': {adapterFor: () => mongo.mongoAdapter},
    './edit-sql': pureModule('edit-sql'), './edit-mongo': mongo, '../../lib/jsonTree/expansion-plan': {},
    // "Add row" (InsertFlow) is covered by dbInsertForm.test.ts.
    './insert-flow.svelte': { InsertFlow: class { target = null; form = null; } }, './review-batch': {},
    '../../lib/toastError': {}, '../../lib/plural': {},
  });
  const flow = new EditFlow();
  Object.assign(flow, {
    statement:'db.users.find({})', connectionId:'fixture', engine:'mongodb', ranNode:'app',
    editTable:'stale', editPkCols:['_id'], editDb:'stale',
    result:{columns:[{name:'_id'}, {name:'profile'}], cells_truncated:true},
  });
  await flow.resolveTarget();
  assert.equal(flow.editTable, null);
  assert.equal(flow.editPkCols.length, 0);
  assert.match(flow.editReason, /shortened for display/);
  flow.result = {columns:[{name:'_id'}, {name:'profile'}], truncated:true,
    rows:[['source', {bio:'ordinary…[truncated 7 chars]', name:'safe'}]]};
  await flow.resolveTarget();
  assert.equal(flow.editTable, 'users');
  assert.equal(flow.editReason, null);
});

test('an _id change is a server-side rename ($merge copy, then delete), never a silent no-op', () => {
  const { mongoAdapter } = adapter();
  const id = 'SLEIPNIR INTERACTIVE d.o.o';
  const ctx = {target:{table:'providers'},columns:[{name:'_id'},{name:'check'},{name:'single'}],liveRows:[[id,true,null]]};
  const copy = `db.providers.aggregate([{"$match": {"_id": "${id}"}}, {"$set": {"_id": {"$literal": "${id}."}}}, {"$merge": {"into": "providers", "whenMatched": "fail"}}])`;
  const del = `db.providers.deleteOne({"_id": "${id}"})`;
  // Doc editor: only the _id changed (the reported "Save does nothing").
  const only = mongoAdapter.buildReplace(0,{_id:`${id}.`,check:true,single:null},ctx);
  assert.equal(only.sql, [copy, del].join(';\n'), 'statements are `;`-separated: the runner splits on `;` only');
  // ...plus a field: the update targets the NEW _id, between copy and delete.
  const both = mongoAdapter.buildReplace(0,{_id:`${id}.`,check:false,single:null},ctx);
  assert.equal(both.sql, [copy, `db.providers.updateOne({"_id": "${id}."}, {"$set":{"check":false}})`, del].join(';\n'));
  // Grid: an _id cell draft renames; a null field typed `true` stays a boolean.
  const grid = mongoAdapter.buildUpdate([{rowIdx:0,patch:{cells:new Map([[0,`${id}.`],[2,'true']]),set:new Map(),unset:new Set(),rename:new Map()}}],ctx);
  assert.equal(grid.sql, [copy, del, `db.providers.updateOne({"_id": "${id}."}, {"$set": {"single": true}})`].join(';\n'));
  assert.deepEqual([...grid.stmtRows], [0, 0, 0]);
});

test('typed grid literals keep booleans for null fields and ObjectIds as ObjectIds', () => {
  const { mongoLiteral, mongoStatementId } = adapter();
  assert.equal(mongoLiteral('true', null), 'true');
  assert.equal(mongoLiteral('false', undefined), 'false');
  assert.equal(mongoLiteral('5624', null), '"5624"', 'a null field never guesses a number');
  const oid = '507f1f77bcf86cd799439011';
  assert.equal(mongoLiteral(`ObjectId("${oid}")`, {$oid:'0'.repeat(24)}), `{"$oid": "${oid}"}`);
  assert.equal(mongoLiteral(oid, {$oid:'0'.repeat(24)}), `{"$oid": "${oid}"}`);
  assert.equal(mongoLiteral(oid, 'text'), `"${oid}"`, 'a string _id stays a string');
  assert.equal(mongoStatementId('db.c.updateOne({"_id": "a.b c"}, {"$set": {"x": 1}})'), 'a.b c');
  assert.equal(JSON.stringify(mongoStatementId('db.c.deleteOne({"_id": {"$oid": "' + oid + '"}})')), `{"$oid":"${oid}"}`);
  assert.equal(mongoStatementId('db.c.find({})'), undefined);
});
