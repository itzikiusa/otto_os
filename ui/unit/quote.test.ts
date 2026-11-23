import { test } from 'node:test';
import assert from 'node:assert/strict';
import { markdownQuote, placeSelectionQuote, withQuote } from '../src/lib/quote/quote.ts';

test('a quoted passage prefixes every line and keeps blank lines inside it', () => {
  assert.equal(markdownQuote('  one\r\n\r\ntwo  '), '> one\n>\n> two');
  assert.equal(markdownQuote('   '), '');
});

test('a quote goes below the draft with a free line for the reply', () => {
  assert.equal(withQuote('', 'a'), '> a\n\n');
  assert.equal(withQuote('Hi there \n', 'a\nb'), 'Hi there\n\n> a\n> b\n\n');
  assert.equal(withQuote('keep', '  '), 'keep');
});

test('placeSelectionQuote aligns with selection in LTR and mirrors inlineStart in RTL', () => {
  const vp = { width: 1000, height: 800 };
  // Selection on the left side (center x = 130, pill width = 84 → physical left = 88, right = 172).
  const ltr = placeSelectionQuote({ top: 200, left: 100, width: 60 }, vp, false, 84);
  assert.deepEqual(ltr, { top: 166, inlineStart: 88 });

  // Same physical selection in RTL: inset-inline-start measures from the RIGHT edge (1000 - 172 = 828).
  const rtl = placeSelectionQuote({ top: 200, left: 100, width: 60 }, vp, true, 84);
  assert.deepEqual(rtl, { top: 166, inlineStart: 828 });

  // Selection near the right edge clamps 8px inside the right edge in both directions.
  const rtlRight = placeSelectionQuote({ top: 20, left: 960, width: 30 }, vp, true, 84);
  assert.deepEqual(rtlRight, { top: 8, inlineStart: 8 });
});
