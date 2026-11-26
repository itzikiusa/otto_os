// "Condition on run data" (modules/workflows/editor/conditionRule.ts): a rule
// becomes an expression in the engine's language and reads back from it;
// anything else stays a free expression.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseRule, ruleExpr, ruleFields } from '../src/modules/workflows/editor/conditionRule.ts';

test('a rule is written as an expression; numbers and booleans bare, text quoted', () => {
  assert.equal(ruleExpr({ field: 'len(run.repos)', op: 'gt', value: '1' }), 'len(run.repos) > 1');
  assert.equal(ruleExpr({ field: 'run.input.base', op: 'is', value: 'main' }), 'run.input.base == "main"');
  assert.equal(ruleExpr({ field: 'output.passed', op: 'is_not', value: 'true' }), 'output.passed != true');
  assert.equal(ruleExpr({ field: 'run.input.msg', op: 'contains', value: 'say "hi"' }), 'run.input.msg contains "say \\"hi\\""');
  assert.equal(ruleExpr({ field: 'run.input.pr', op: 'empty', value: 'x' }), '!run.input.pr');
  assert.equal(ruleExpr({ field: '  ', op: 'is', value: '1' }), '');
});

test('an expression written by a rule reads back as that rule', () => {
  for (const r of [
    { field: 'len(run.repos)', op: 'gt', value: '1' },
    { field: 'run.input.base', op: 'is', value: 'main' },
    { field: 'output.result', op: 'is', value: 'true' },
    { field: 'run.input.msg', op: 'contains', value: 'say "hi"' },
    { field: 'run.input.pr', op: 'empty', value: '' },
  ] as const) assert.deepEqual(parseRule(ruleExpr(r)), r);
});

test('a free expression stays free', () => {
  for (const e of ['passed == true && score > 80', 'lower(input.x) == "a"', 'output.result ==', '']) assert.equal(parseRule(e), null, e);
});

test('fields: repositories, each start input, the previous step', () => {
  assert.deepEqual(ruleFields(['pr']).map((f) => f.field), ['len(run.repos)', 'run.input.pr', 'output.result']);
});
