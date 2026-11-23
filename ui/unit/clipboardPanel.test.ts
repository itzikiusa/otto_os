import test from 'node:test';
import assert from 'node:assert/strict';
import {
  RENDER_CAP, dayLabel, formatSize, groupByDay, moveSelection, pasteLabel, title, type ClipEntry,
} from '../src/modules/clipboard/clipboardList.ts';
import { loadSource } from './sourceHarness.ts';

const NOW = new Date(2026, 10, 20, 15, 0).getTime();
const DAY = 86_400_000;
let n = 0;
const entry = (over: Partial<ClipEntry>): ClipEntry => ({
  id: `e${n++}`, kind: 'text', text: 'hello', width: 0, height: 0, size: 5, source: null,
  first_ms: NOW, last_ms: NOW, count: 1, pinned: false, ...over,
});

test('entries group as Pinned, Today, Yesterday, then dates — in the shell’s order', () => {
  const list = [
    entry({ text: 'now' }),
    entry({ text: 'pinned old', pinned: true, last_ms: NOW - 9 * DAY }),
    entry({ text: 'yesterday', last_ms: NOW - DAY }),
    entry({ text: 'older', last_ms: NOW - 3 * DAY }),
  ];
  const groups = groupByDay(list, NOW);
  assert.deepEqual(groups.map((g) => g.label).slice(0, 3), ['Pinned', 'Today', 'Yesterday']);
  assert.equal(groups.length, 4);
  assert.deepEqual(groups.flatMap((g) => g.items.map((e) => e.text)), ['pinned old', 'now', 'yesterday', 'older']);
  assert.equal(dayLabel(NOW - 60_000, NOW), 'Today');
});

test('a long history renders at most RENDER_CAP rows', () => {
  const many = Array.from({ length: RENDER_CAP + 50 }, () => entry({}));
  assert.equal(groupByDay(many, NOW).flatMap((g) => g.items).length, RENDER_CAP);
});

test('row titles: image size, file name, first text line', () => {
  assert.equal(title(entry({ kind: 'image', width: 381, height: 60 })), 'Image (381×60)');
  assert.equal(title(entry({ kind: 'file', text: '/Users/me/Q3 report.pdf' })), 'Q3 report.pdf');
  assert.equal(title(entry({ kind: 'file', text: '/a/one.txt\n/a/two.txt\n/a/three' })), 'one.txt and 2 more');
  assert.equal(title(entry({ text: '\n\n  first line  \nsecond' })), 'first line');
});

test('sizes, selection and the default action read well', () => {
  assert.equal(formatSize(512), '512 B');
  assert.equal(formatSize(1536), '1.5 KB');
  assert.equal(formatSize(300 * 1024), '300 KB');
  assert.equal(formatSize(5 * 1024 * 1024), '5.0 MB');
  assert.equal(moveSelection(0, -1, 5), 0);
  assert.equal(moveSelection(4, 1, 5), 4);
  assert.equal(moveSelection(2, 1, 5), 3);
  assert.equal(moveSelection(3, 1, 0), 0);
  assert.equal(pasteLabel('Safari'), 'Paste to Safari');
  assert.equal(pasteLabel(null), 'Paste');
});

function bridge(isTauri: boolean) {
  const calls: [string, unknown][] = [];
  const mod = loadSource(new URL('../src/lib/clipboardHistory.ts', import.meta.url), {
    './desktop': { isTauri },
    '@tauri-apps/api/core': {
      invoke: async (cmd: string, args: unknown) => {
        calls.push([cmd, args]);
        return cmd === 'clipboard_paste' ? false : [];
      },
    },
  }) as typeof import('../src/lib/clipboardHistory.ts');
  return { clip: mod.clip, calls };
}

test('the panel’s native calls map to the shell’s commands', async () => {
  const { clip, calls } = bridge(true);
  await clip.list('deploy', 'image');
  await clip.set({ paused: true });
  // The paste fallback: the shell answers false (copied only, no Accessibility).
  assert.equal(await clip.paste('abc', true), false);
  await clip.icon('com.apple.Safari');
  // Args come from the sandboxed module's realm: compare as plain data.
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ['clipboard_list', { query: 'deploy', kind: 'image' }],
    ['clipboard_set', { enabled: null, paused: true }],
    ['clipboard_paste', { id: 'abc', paste: true }],
    ['clipboard_icon', { bundleId: 'com.apple.Safari' }],
  ]);
});

test('outside the desktop app every call refuses instead of pretending', async () => {
  const { clip, calls } = bridge(false);
  await assert.rejects(clip.status(), /desktop app/);
  assert.equal(calls.length, 0);
});
