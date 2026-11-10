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

const { loadViewed, saveViewed, viewSig } = await import('../src/modules/git/diff-viewed-store.ts');

const f = (path: string, added: number, deleted: number, status: 'modified' | 'added' = 'modified') => ({
  path, old_path: null, is_binary: false, hunks: [], added, deleted, status,
});

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
