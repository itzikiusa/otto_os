// Persisted PR "Viewed" marks (modules/git/diff-viewed-store.ts): a mark
// survives a reload while the file is unchanged and drops once it changes.
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';

class MemStorage {
  #m = new Map<string, string>();
  get length(): number { return this.#m.size; }
  key(i: number): string | null { return [...this.#m.keys()][i] ?? null; }
  getItem(k: string): string | null { return this.#m.get(k) ?? null; }
  setItem(k: string, v: string): void { this.#m.set(k, String(v)); }
  removeItem(k: string): void { this.#m.delete(k); }
  clear(): void { this.#m.clear(); }
}
(globalThis as { localStorage?: unknown }).localStorage = new MemStorage();

const { loadViewed, saveViewed, staleViewed, viewSig } = await import('../src/modules/git/diff-viewed-store.ts');

type F = { path: string; old_path: null; is_binary: boolean; hunks: { header: string; lines: { origin: 'add' | 'del' | 'context'; content: string; old_line: number | null; new_line: number | null }[] }[]; added: number; deleted: number; status: 'modified' | 'added' };
const f = (path: string, added: number, deleted: number, status: 'modified' | 'added' = 'modified'): F => ({
  path, old_path: null, is_binary: false, hunks: [], added, deleted, status,
});
/** `file` with its hunks loaded: one added line per text. */
const withHunks = (file: F, ...texts: string[]): F => ({
  ...file,
  hunks: [{ header: '@@ -1 +1 @@', lines: texts.map((content, i) => ({ origin: 'add' as const, content, old_line: null, new_line: i + 1 })) }],
});
const store = () => (globalThis as unknown as { localStorage: MemStorage }).localStorage;

beforeEach(() => (globalThis as unknown as { localStorage: MemStorage }).localStorage.clear());

test('marks persist for unchanged files and drop for changed ones', () => {
  const files = [f('a.ts', 1, 2), f('b.ts', 3, 0, 'added')];
  saveViewed('r#1', files, new Set(['a.ts', 'b.ts']));
  assert.deepEqual([...loadViewed('r#1', files)].sort(), ['a.ts', 'b.ts']);
  // A push changed b.ts: its mark no longer applies.
  const after = [f('a.ts', 1, 2), f('b.ts', 5, 0, 'added')];
  assert.deepEqual([...loadViewed('r#1', after)], ['a.ts']);
  // Other PRs are untouched.
  assert.equal(loadViewed('r#2', files).size, 0);
});

test('clearing every mark removes the key; signature format', () => {
  const files = [f('a.ts', 1, 2)];
  saveViewed('r#1', files, new Set(['a.ts']));
  saveViewed('r#1', files, new Set());
  assert.equal(loadViewed('r#1', files).size, 0);
  assert.equal(viewSig(files[0]), 'modified:1:2');
});

test('saving one view merges: marks of files that view did not show are kept', () => {
  // The all-commits view ticks a.ts and b.ts…
  saveViewed('r#1', [f('a.ts', 1, 2), f('b.ts', 3, 0)], new Set(['a.ts', 'b.ts']));
  // …a view that shows only b.ts unticks it; a.ts must survive.
  saveViewed('r#1', [f('b.ts', 3, 0)], new Set());
  assert.deepEqual([...loadViewed('r#1', [f('a.ts', 1, 2), f('b.ts', 3, 0)])], ['a.ts']);
});

test('a hashed mark drops when a push rewrote lines but kept the counts', () => {
  const a = f('a.ts', 1, 0);
  saveViewed('r#1', [a], new Set(['a.ts']), new Map([['a.ts', withHunks(a, 'const x = 1;')]]));
  // Summary view after a push with the same counts: hunks not loaded yet, so
  // the mark holds provisionally on its stat…
  assert.deepEqual([...loadViewed('r#1', [a])], ['a.ts']);
  // …and the hunks arriving contradict it.
  const pushed = new Map([['a.ts', withHunks(a, 'const x = 2;')]]);
  assert.deepEqual(staleViewed('r#1', [a], new Set(['a.ts']), pushed), ['a.ts']);
  assert.equal(loadViewed('r#1', [a], pushed).size, 0);
  // The same patch keeps it.
  const same = new Map([['a.ts', withHunks(a, 'const x = 1;')]]);
  assert.deepEqual(staleViewed('r#1', [a], new Set(['a.ts']), same), []);
  assert.deepEqual([...loadViewed('r#1', [a], same)], ['a.ts']);
});

test('an old stat-only mark survives the upgrade, with or without hunks', () => {
  store().setItem('otto_viewed:r#1', JSON.stringify({ 'a.ts': 'modified:1:0' }));
  const a = f('a.ts', 1, 0);
  const loaded = new Map([['a.ts', withHunks(a, 'anything')]]);
  assert.deepEqual([...loadViewed('r#1', [a])], ['a.ts']);
  assert.deepEqual([...loadViewed('r#1', [a], loaded)], ['a.ts']);
  assert.deepEqual(staleViewed('r#1', [a], new Set(['a.ts']), loaded), []);
  // …but not once the stat itself changed.
  assert.equal(loadViewed('r#1', [f('a.ts', 2, 0)]).size, 0);
});

test('re-saving from a summary view keeps a hashed mark hashed', () => {
  const a = f('a.ts', 1, 0);
  saveViewed('r#1', [a], new Set(['a.ts']), new Map([['a.ts', withHunks(a, 'v1')]]));
  const before = store().getItem('otto_viewed:r#1');
  assert.match(before ?? '', /modified:1:0#[0-9a-f]{8}/);
  saveViewed('r#1', [a], new Set(['a.ts']));
  assert.equal(store().getItem('otto_viewed:r#1'), before);
});
