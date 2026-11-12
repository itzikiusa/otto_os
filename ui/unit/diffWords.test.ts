// Word-level marks inside changed lines (src/modules/git/diff-words.ts):
// token boundaries, unicode, whitespace-only edits, the size / similarity
// guards, pairing inside a hunk, and composing with highlighted HTML.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { WORD_MAX_LINE, lineWords, markWords, wordDiff } from '../src/modules/git/diff-words.ts';

const at = (text: string, r: [number, number][] | undefined) => (r ?? []).map(([s, e]) => text.slice(s, e));

test('marks whole changed tokens, not characters', () => {
  const a = 'const total = price * qty;';
  const b = 'const total = price * quantity;';
  const d = wordDiff(a, b)!;
  assert.deepEqual(at(a, d.a), ['qty']);
  assert.deepEqual(at(b, d.b), ['quantity']);
});

test('adjacent changed words separated by whitespace become one mark', () => {
  const a = 'return foo bar(x);';
  const b = 'return baz qux(x);';
  const d = wordDiff(a, b)!;
  assert.deepEqual(at(a, d.a), ['foo bar']);
  assert.deepEqual(at(b, d.b), ['baz qux']);
});

test('unicode words (any script, astral chars) keep their boundaries', () => {
  const a = 'label = "שלום עולם" // 😀 ok';
  const b = 'label = "שלום חבר" // 😀 ok';
  const d = wordDiff(a, b)!;
  assert.deepEqual(at(a, d.a), ['עולם']);
  assert.deepEqual(at(b, d.b), ['חבר']);
});

test('a whitespace-only change marks just the whitespace', () => {
  const a = 'if (x)  {';
  const b = 'if (x) {';
  const d = wordDiff(a, b)!;
  assert.deepEqual(at(a, d.a), ['  ']);
  assert.deepEqual(at(b, d.b), [' ']);
});

test('guards: equal, too long, too dissimilar → null (whole-line colour)', () => {
  assert.equal(wordDiff('same', 'same'), null);
  assert.equal(wordDiff('a'.repeat(WORD_MAX_LINE + 1), 'b'), null);
  assert.equal(wordDiff('alpha beta gamma', 'one two three four'), null);
});

test('performance guard: a huge token table is refused quickly', () => {
  const a = Array.from({ length: 249 }, (_, i) => `a${i}`).join(' ');
  const b = Array.from({ length: 249 }, (_, i) => `b${i}`).join(' ');
  const t0 = performance.now();
  assert.equal(wordDiff(a, b), null);
  assert.ok(performance.now() - t0 < 50);
});

test('pairs the k-th deleted line with the k-th added line of the run', () => {
  const del1 = { origin: 'del' as const, content: 'let a = 1;', old_line: 1, new_line: null };
  const del2 = { origin: 'del' as const, content: 'let b = 2;', old_line: 2, new_line: null };
  const add1 = { origin: 'add' as const, content: 'let a = 10;', old_line: null, new_line: 1 };
  const ctx = { origin: 'context' as const, content: 'x', old_line: 3, new_line: 2 };
  const file = { path: 'f.ts', old_path: null, is_binary: false, hunks: [{ header: '@@', lines: [del1, del2, add1, ctx] }] };
  assert.deepEqual(at(del1.content, lineWords(file, 0, del1) ?? undefined), ['1']);
  assert.deepEqual(at(add1.content, lineWords(file, 0, add1) ?? undefined), ['10']);
  // del2 has no partner; context never gets marks.
  assert.equal(lineWords(file, 0, del2), null);
  assert.equal(lineWords(file, 0, ctx), null);
});

test('markWords composes with highlighted markup and entities', () => {
  // Text: `a <b> c` with "<b>" changed (3 chars), across a token span.
  const html = 'a <span class="hljs-tag">&lt;b&gt;</span> c';
  const out = markWords(html, [[2, 5]]);
  assert.equal(out, 'a <span class="hljs-tag"><span class="dw">&lt;b&gt;</span></span> c');
  // A mark that crosses a tag is closed before it and reopened after it.
  assert.equal(markWords('x<i>yz</i>w', [[0, 3]]), '<span class="dw">x</span><i><span class="dw">yz</span></i>w');
  // No ranges: untouched.
  assert.equal(markWords(html, null), html);
  // A range that ends mid-text closes there.
  assert.equal(markWords('abcdef', [[1, 3]]), 'a<span class="dw">bc</span>def');
});
