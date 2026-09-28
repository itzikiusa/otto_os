import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// Find-in-page providers (src/lib/findProviders.ts): windowed views expose
// their whole row model to ⌘F; nested providers never double-count.

const FOLLOWING = 4;

/** A fake element: `path` is its position in document order (e.g. [0, 2]). */
function el(path: number[], opts: { hidden?: boolean; connected?: boolean } = {}) {
  const self = {
    path,
    isConnected: opts.connected ?? true,
    checkVisibility: () => !opts.hidden,
    contains(other: { path: number[] }) {
      return other.path.length >= path.length && path.every((p, i) => other.path[i] === p);
    },
    compareDocumentPosition(other: { path: number[] }) {
      for (let i = 0; i < Math.min(path.length, other.path.length); i++) {
        if (other.path[i] !== path[i]) return other.path[i] > path[i] ? FOLLOWING : 2;
      }
      return other.path.length > path.length ? FOLLOWING | 16 : 2 | 8;
    },
  };
  return self;
}

function load() {
  return loadSource(new URL('../src/lib/findProviders.ts', import.meta.url), {}, {
    Node: { DOCUMENT_POSITION_FOLLOWING: FOLLOWING, ELEMENT_NODE: 1 },
  });
}

function provider(root: unknown, rows: string[], active = true) {
  return {
    root: () => root,
    count: () => rows.length,
    text: (i: number) => rows[i],
    reveal: () => {},
    rowElement: () => null,
    active: () => active,
  };
}

test('countOccurrences: case-insensitive, non-overlapping, capped', () => {
  const { countOccurrences } = load();
  assert.equal(countOccurrences('Foo foo FOO', 'foo'), 3);
  assert.equal(countOccurrences('aaaa', 'aa'), 2, 'non-overlapping');
  assert.equal(countOccurrences('foo foo foo', 'foo', 2), 2, 'cap');
  assert.equal(countOccurrences('fo', 'foo'), 0);
  assert.equal(countOccurrences('anything', ''), 0);
});

test('active providers: document order, hidden/detached/inactive skipped, nested dropped', () => {
  const fp = load();
  const diff = el([0, 1]);
  const grid = el([0, 0]);
  const chat = el([1]);
  const toolOutput = el([1, 3, 0]); // a windowed tool output INSIDE the chat
  const hidden = el([2], { hidden: true });
  const detached = el([3], { connected: false });
  const offs = [
    fp.registerFindProvider(provider(diff, ['a'])),
    fp.registerFindProvider(provider(grid, ['b'])),
    fp.registerFindProvider(provider(chat, ['c'])),
    fp.registerFindProvider(provider(toolOutput, ['d'])),
    fp.registerFindProvider(provider(hidden, ['e'])),
    fp.registerFindProvider(provider(detached, ['f'])),
    fp.registerFindProvider(provider(el([4]), ['g'], false)),
  ];
  const roots = [...fp.activeFindProviders()].map((a: { root: unknown }) => a.root);
  assert.deepEqual(roots, [grid, diff, chat], 'nested tool output is covered by the chat');
  for (const off of offs) off();
  assert.equal(fp.activeFindProviders().length, 0, 'unregister removes them');
});

test('an inactive outer provider leaves the nested one active', () => {
  const fp = load();
  const chat = el([1]);
  const toolOutput = el([1, 3, 0]);
  fp.registerFindProvider(provider(chat, ['c'], false));
  fp.registerFindProvider(provider(toolOutput, ['d']));
  assert.deepEqual([...fp.activeFindProviders()].map((a: { root: unknown }) => a.root), [toolOutput]);
});
