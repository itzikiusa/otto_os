// API client Format JSON / Pretty re-indent the body text (lib/jsonReindent.ts)
// instead of parse → stringify, which rounded big ints and rewrote `1.10` /
// `1e400`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { reindentJson } from '../src/lib/jsonReindent.ts';

test('number and string literals are kept verbatim', () => {
  const body = '{"id":12345678901234567890,"n":1.10,"x":1e400,"s":"a, b: {\\"c\\"}"}';
  assert.equal(
    reindentJson(body),
    '{\n  "id": 12345678901234567890,\n  "n": 1.10,\n  "x": 1e400,\n  "s": "a, b: {\\"c\\"}"\n}',
  );
});

test('nesting, empty containers and existing whitespace', () => {
  assert.equal(
    reindentJson(' { "a" : [ 1 , { } , [ ] ] ,\n"b":{"c":null} } '),
    '{\n  "a": [\n    1,\n    {},\n    []\n  ],\n  "b": {\n    "c": null\n  }\n}',
  );
  assert.equal(reindentJson('[1,2]', 4), '[\n    1,\n    2\n]');
  assert.equal(reindentJson('"x"'), '"x"');
});
