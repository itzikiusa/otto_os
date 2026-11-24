import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CompletionContext } from '@codemirror/autocomplete';
import { markdown } from '@codemirror/lang-markdown';
import { EditorState, Transaction, type Extension, type TransactionSpec } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import {
  BLOCKS,
  formatTable,
  formatTableAtCursor,
  insertBlock,
  insertLink,
  listIndent,
  markdownEnter,
  setHeading,
  slashCompletions,
  tableTab,
  toggleLinePrefix,
  toggleWrap,
} from '../src/lib/components/cm-md-assist.ts';

// Markdown writing help in the IDE editor (owner, 2026-11-24: "for .md files maybe have
// some helper to create them more easily"). The commands run on a real EditorView
// (node has no DOM: a stand-in view applies each command's transaction to an EditorState).

/** A stand-in EditorView: enough for commands (state, dispatch, focus, dom.closest). */
function fakeView(doc: string, anchor: number, head = anchor, ext: Extension = []) {
  let state = EditorState.create({ doc, selection: { anchor, head }, extensions: [markdown(), ext] });
  const view = {
    get state() { return state; },
    dispatch(spec: Transaction | TransactionSpec) {
      state = (spec instanceof Transaction ? spec : state.update(spec)).state;
    },
    focus() {},
    dom: { closest: () => ({}) },
  };
  return view as unknown as EditorView & { state: EditorState };
}

const sel = (v: EditorView) => v.state.sliceDoc(v.state.selection.main.from, v.state.selection.main.to);

test('Bold / Italic / Code wrap a selection, unwrap it again, and pair around an empty cursor', () => {
  const v = fakeView('make it bold', 8, 12);
  toggleWrap('**')(v);
  assert.equal(v.state.doc.toString(), 'make it **bold**');
  assert.equal(sel(v), 'bold');
  toggleWrap('**')(v);
  assert.equal(v.state.doc.toString(), 'make it bold');
  const e = fakeView('x ', 2);
  toggleWrap('`')(e);
  assert.equal(e.state.doc.toString(), 'x ``');
  assert.equal(e.state.selection.main.head, 3);
});

test('Link: the selection becomes the text and "url" is selected', () => {
  const v = fakeView('see docs here', 4, 8);
  insertLink(v);
  assert.equal(v.state.doc.toString(), 'see [docs](url) here');
  assert.equal(sel(v), 'url');
});

test('Headings set, change and toggle off', () => {
  const v = fakeView('Title', 0);
  setHeading(2)(v);
  assert.equal(v.state.doc.toString(), '## Title');
  setHeading(1)(v);
  assert.equal(v.state.doc.toString(), '# Title');
  setHeading(1)(v);
  assert.equal(v.state.doc.toString(), 'Title');
});

test('Lists, checklists and quotes over several lines; switching kinds; toggling off', () => {
  const v = fakeView('one\ntwo\nthree', 0, 13);
  toggleLinePrefix('number')(v);
  assert.equal(v.state.doc.toString(), '1. one\n2. two\n3. three');
  toggleLinePrefix('check')(v);
  assert.equal(v.state.doc.toString(), '- [ ] one\n- [ ] two\n- [ ] three');
  toggleLinePrefix('check')(v);
  assert.equal(v.state.doc.toString(), 'one\ntwo\nthree');
  const empty = fakeView('', 0);
  toggleLinePrefix('check')(empty);
  assert.equal(empty.state.selection.main.head, 6, 'the cursor lands after the new marker, where you type');
  const q = fakeView('  - item', 4);
  toggleLinePrefix('quote')(q);
  assert.equal(q.state.doc.toString(), '  > item', 'indentation kept');
});

test('Enter continues a checklist, a bulleted list and a quote, and ends a list on an empty item', () => {
  const cont = (doc: string): string => {
    const v = fakeView(doc, doc.length);
    assert.ok(markdownEnter(v));
    return v.state.doc.toString();
  };
  assert.equal(cont('- [ ] buy milk'), '- [ ] buy milk\n- [ ] ');
  assert.equal(cont('- one'), '- one\n- ');
  assert.equal(cont('> quoted'), '> quoted\n> ');
  assert.equal(cont('- one\n- '), '- one\n', 'an empty item ends the list (its marker goes)');
});

test('Enter after an unclosed fence adds the closing fence with the cursor inside', () => {
  const doc = 'text\n```go';
  const v = fakeView(doc, doc.length);
  assert.ok(markdownEnter(v));
  assert.equal(v.state.doc.toString(), 'text\n```go\n\n```');
  assert.equal(v.state.doc.lineAt(v.state.selection.main.head).number, 3);
});

test('Slash menu: only at the start of an empty line; Table inserts a 3×3 table with the first header selected', () => {
  const at = (doc: string, pos = doc.length) =>
    slashCompletions(new CompletionContext(EditorState.create({ doc, extensions: [markdown()] }), pos, false));
  assert.equal(at('text /'), null);
  assert.equal(at('/', 1)?.from, 1);
  assert.deepEqual(at('/ta')!.options.map((o) => o.label), BLOCKS.map((b) => b.label));
  for (const label of ['Heading 1', 'Heading 2', 'Heading 3', 'Table', 'Code block', 'Checklist', 'Quote', 'Divider', 'Callout']) {
    assert.ok(BLOCKS.some((b) => b.label === label), label);
  }
  const v = fakeView('intro\n/ta', 9);
  insertBlock(v, BLOCKS.find((b) => b.label === 'Table')!, 6, 9);
  assert.equal(v.state.doc.toString(), 'intro\n\n| Column 1 | Column 2 | Column 3 |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |', 'a blank line keeps the table out of the paragraph');
  assert.equal(sel(v), 'Column 1');
  const c = fakeView('/code', 5);
  insertBlock(c, BLOCKS.find((b) => b.label === 'Code block')!, 0, 5);
  assert.equal(c.state.doc.toString(), '```\n\n```');
  assert.equal(c.state.doc.lineAt(c.state.selection.main.head).number, 2, 'cursor inside the fence');
  const after = fakeView('- [ ] done\n/', 12);
  insertBlock(after, BLOCKS.find((b) => b.label === 'Checklist')!, 11, 12);
  assert.equal(after.state.doc.toString(), '- [ ] done\n- [ ] ', 'a checklist item continues the list');
  const div = fakeView('para\n/', 6);
  insertBlock(div, BLOCKS.find((b) => b.label === 'Divider')!, 5, 6);
  assert.equal(div.state.doc.toString(), 'para\n\n---\n', 'not a setext heading');
  const callout = fakeView('/', 1);
  insertBlock(callout, BLOCKS.find((b) => b.label === 'Callout')!, 0, 1);
  assert.equal(callout.state.doc.toString(), '> [!NOTE]\n> ');
});

test('Format table aligns columns and keeps alignment colons', () => {
  assert.deepEqual(formatTable(['| Name | Value |', '|:--|--:|', '| alpha | 1 |', '| b | 22 |']), [
    '| Name  | Value |',
    '| :---- | ----: |',
    '| alpha | 1     |',
    '| b     | 22    |',
  ]);
  const v = fakeView('x\n| a | bb |\n| - | - |\n| ccc | d |\ny', 3);
  assert.ok(formatTableAtCursor(v));
  assert.equal(v.state.doc.toString(), 'x\n| a   | bb  |\n| --- | --- |\n| ccc | d   |\ny');
  assert.equal(formatTableAtCursor(fakeView('plain', 1)), false);
});

test('Tab moves to the next cell and adds a row past the last one', () => {
  const doc = '| a | b |\n| --- | --- |\n| 1 | 2 |';
  const v = fakeView(doc, 2);
  assert.ok(tableTab(1)(v));
  assert.equal(sel(v), 'b');
  assert.ok(tableTab(1)(v));
  assert.equal(sel(v), '1', 'the separator row is skipped');
  assert.ok(tableTab(1)(v));
  assert.ok(tableTab(1)(v));
  assert.equal(v.state.doc.toString(), doc + '\n|  |  |', 'a new row');
  assert.equal(v.state.doc.lineAt(v.state.selection.main.head).number, 4);
});


test('Tab / Shift-Tab indent and outdent a list item (not plain text)', () => {
  const v = fakeView('- one\n- two', 9);
  assert.ok(listIndent(true)(v));
  assert.match(v.state.doc.line(2).text, /^\s+- two$/);
  assert.ok(listIndent(false)(v));
  assert.equal(v.state.doc.line(2).text, '- two');
  assert.equal(listIndent(true)(fakeView('plain text', 3)), false);
});
