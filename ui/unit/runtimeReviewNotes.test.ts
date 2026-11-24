// Review notes on a diff (runtime/reviewNotes.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { endsAt, isOutdated, noteAnchor, noteWhere, reviewMessage, sentReview, type ReviewNote } from '../src/modules/threads/runtime/reviewNotes.ts';
import { assembleComposerMessage } from '../src/modules/threads/runtime/composerContext.ts';

const L = (origin: string, content: string, old_line: number | null, new_line: number | null) => ({ origin, content, old_line, new_line }) as never;
const lines = [L('context', 'a', 10, 12), L('del', 'b', 11, null), L('add', 'B', null, 13), L('add', 'C', null, 14)];

test('a note records its lines, both line spans and the quote', () => {
  const a = noteAnchor(lines.slice(1));
  assert.deepEqual(a, { oldLines: '11', newLines: '13–14', quote: '-b\n+B\n+C' });
  assert.equal(noteWhere(a), 'new 13–14 · old 11');
  assert.equal(noteWhere(noteAnchor([lines[1]])), 'old 11');
});

test('one message: file, lines, quoted lines and the comment for each note', () => {
  const n = (path: string, sel: never[], comment: string): ReviewNote => ({ id: path, repoId: 'r', target: 'working', path, comment, state: 'draft', ...noteAnchor(sel) });
  assert.equal(
    reviewMessage([n('src/a.ts', lines.slice(1, 3), 'Keep the old name.'), n('README.md', [lines[0]], ' Typo? ')]),
    '1. src/a.ts (new 13 · old 11)\n   > -b\n   > +B\n   Keep the old name.\n\n2. README.md (new 12 · old 10)\n   >  a\n   Typo?',
  );
});

test('a note is outdated once its lines are gone from the diff; current while they stay', () => {
  const note = noteAnchor(lines.slice(1, 3));
  assert.equal(isOutdated(note, { hunks: [{ header: '', lines }] }), false);
  assert.equal(isOutdated(note, { hunks: [{ header: '', lines: [L('context', 'a', 10, 12), L('add', 'B2', null, 13)] }] }), true);
  assert.equal(isOutdated(note, { hunks: [] }), true);
});

test('a note shows under its last line (new side first, an old-only note under its old line)', () => {
  const both = noteAnchor(lines.slice(1));
  assert.equal(endsAt(both, lines[3]), true);
  assert.equal(endsAt(both, lines[2]), false);
  const old = noteAnchor([lines[1]]);
  assert.equal(endsAt(old, lines[1]), true);
  assert.equal(endsAt(old, lines[0]), false);
});

test('a sent review reads back from the message as its notes (the timeline block), for any agent', () => {
  const notes = [
    { id: 'a', repoId: 'r', target: 'working', path: 'src/a.ts', oldLines: '9', newLines: '9', quote: '-save(note);\n+queue.push(note);', comment: 'Keep the old name.', state: 'draft' },
    { id: 'b', repoId: 'r', target: 'working', path: 'README.md', oldLines: '', newLines: '1', quote: '+# harbor', comment: 'Typo?\nAnd a second line.', state: 'draft' },
  ] as ReviewNote[];
  const text = assembleComposerMessage('Please fix these.', [{ id: 'notes:x', kind: 'review-comment', label: 'Review · 2 notes', text: reviewMessage(notes) }]);
  const r = sentReview(text)!;
  assert.equal(r.prose, 'Please fix these.');
  assert.equal(r.label, 'Review · 2 notes');
  assert.deepEqual(r.notes.map((n) => [n.path, n.quote.length, n.comment]), [
    ['src/a.ts', 2, 'Keep the old name.'],
    ['README.md', 1, 'Typo?\nAnd a second line.'],
  ]);
  assert.equal(sentReview('just words'), null);
});
