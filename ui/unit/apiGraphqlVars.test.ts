import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseGraphqlVariables } from '../src/modules/api/graphqlVars.ts';

// S16-19: invalid variables JSON used to be sent as `{}` silently.
test('GraphQL variables: blank → {}, object passes, bad JSON names the line', () => {
  assert.deepEqual(parseGraphqlVariables(''), {});
  assert.deepEqual(parseGraphqlVariables('  \n'), {});
  assert.deepEqual(parseGraphqlVariables(undefined), {});
  assert.deepEqual(parseGraphqlVariables('{"id": 1}'), { id: 1 });
  assert.throws(() => parseGraphqlVariables('{\n  "id": 1,\n  oops\n}'), /valid JSON \(line 3\)/);
  assert.throws(() => parseGraphqlVariables('[1]'), /must be a JSON object/);
  assert.throws(() => parseGraphqlVariables('null'), /must be a JSON object/);
});
