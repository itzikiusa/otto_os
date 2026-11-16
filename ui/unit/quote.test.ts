import { test } from 'node:test';
import assert from 'node:assert/strict';
import { markdownQuote, withQuote } from '../src/lib/quote/quote.ts';

test('a quoted passage prefixes every line and keeps blank lines inside it', () => {
  assert.equal(markdownQuote('  one\r\n\r\ntwo  '), '> one\n>\n> two');
  assert.equal(markdownQuote('   '), '');
});

test('a quote goes below the draft with a free line for the reply', () => {
  assert.equal(withQuote('', 'a'), '> a\n\n');
  assert.equal(withQuote('Hi there \n', 'a\nb'), 'Hi there\n\n> a\n> b\n\n');
  assert.equal(withQuote('keep', '  '), 'keep');
});
