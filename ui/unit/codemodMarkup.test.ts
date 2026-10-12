// `attrValue` builds a RegExp from the attribute name, so every regex
// metacharacter in the name must be escaped — a backslash included — or a
// name could match (or break on) text that is not that attribute.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { attrValue } from '../scripts/codemods/markup.mjs';

test('attrValue reads namespaced and dotted names literally', () => {
  assert.equal(attrValue(' bind:value="x" type="text"', 'bind:value'), 'x');
  assert.equal(attrValue(' on:click.once={go}', 'on:click.once'), 'go');
  assert.equal(attrValue(' onXclickYonce={go}', 'on:click.once'), null);
  assert.equal(attrValue(' {disabled}', 'disabled'), 'disabled');
});

test('attrValue escapes every regex metacharacter, backslash included', () => {
  for (const name of ['a\\', 'a\\d', 'a+', 'a(b', 'a[b', 'a|b', 'a*', 'a?', 'a^b', 'a$b', 'a{1}']) {
    assert.doesNotThrow(() => attrValue(' x="1"', name), name);
    assert.equal(attrValue(` ${name}="v"`, name), 'v', name);
  }
  // `a\d` must not match `a1` (it would if the backslash were left as an escape).
  assert.equal(attrValue(' a1="v"', 'a\\d'), null);
});
