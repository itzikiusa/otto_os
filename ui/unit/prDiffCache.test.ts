import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// The shared PR diff cache (src/modules/git/diff-load.ts) evicts on write:
// expired entries, then LRU past 8 PRs or an estimated 48 MB (r3-05-02).

const flush = () => new Promise((r) => setImmediate(r));

function setup(lineLen = 10, linesPerFile = 10) {
  let now = 1_000_000;
  const file = (path: string) => ({
    path,
    old_path: null,
    is_binary: false,
    hunks: [{ header: '@@', lines: Array.from({ length: linesPerFile }, () => ({ origin: 'add', content: 'x'.repeat(lineLen), old_line: null, new_line: 1 })) }],
  });
  const api = {
    get: async (url: string) => ({ files: [file(url.includes('path=') ? decodeURIComponent(/path=([^&]+)/.exec(url)![1]) : 'a.ts')] }),
  };
  const mod = loadSource(new URL('../src/modules/git/diff-load.ts', import.meta.url), { '../../lib/api/client': { api } }, {
    Date: class {
      static now() {
        return now;
      }
    },
  });
  return { mod, advance: (ms: number) => (now += ms), file };
}

test('at most 8 PRs stay cached, least recently used first out', async () => {
  const { mod } = setup();
  for (let n = 1; n <= 10; n++) await mod.prDiffSummary('r', n);
  await flush();
  const keys = [...mod.prCacheSnapshot()].map(([k]: [string]) => k);
  assert.deepEqual(keys, ['r#3', 'r#4', 'r#5', 'r#6', 'r#7', 'r#8', 'r#9', 'r#10']);
  await mod.prDiffSummary('r', 3); // touch → most recent
  await mod.prDiffSummary('r', 11);
  await flush();
  const after = [...mod.prCacheSnapshot()].map(([k]: [string]) => k);
  assert.ok(!after.includes('r#4'), 'the LRU entry went, not the touched one');
  assert.equal(after[after.length - 2], 'r#3');
});

test('expired entries are swept when another PR is opened', async () => {
  const { mod, advance } = setup();
  await mod.prDiffSummary('r', 1);
  await mod.prDiffSummary('r', 2);
  advance(61_000);
  await mod.prDiffSummary('r', 3);
  await flush();
  assert.deepEqual([...mod.prCacheSnapshot()].map(([k]: [string]) => k), ['r#3']);
});

test('the byte budget evicts older PRs; loaded files are charged', async () => {
  // ~2 KB per line → ~20 MB per file of 10k lines.
  const { mod, file } = setup(1000, 10_000);
  await mod.prDiffSummary('r', 1);
  await mod.prDiffSummary('r', 2);
  await flush();
  const perFile = mod.fileDiffBytes(file('a.ts'));
  assert.ok(perFile > 19_000_000);
  await mod.prDiffFile('r', 2, file('b.ts'), false);
  await flush();
  // 3 × ~20 MB > 48 MB: PR 1 is evicted, PR 2 (just charged) stays.
  const snap = [...mod.prCacheSnapshot()];
  assert.deepEqual(snap.map(([k]: [string]) => k), ['r#2']);
  assert.ok(snap[0][1] >= 2 * perFile);
});
