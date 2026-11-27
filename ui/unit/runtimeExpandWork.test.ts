// Unfolded workflow conversation mode (`expandWork` / `startOpen`): verifies
// that individual commands, long thoughts, and >5 changed-file cards start
// expanded on arrival when `startOpen` is true, while normal Threads defaults
// (`startOpen = false`) remain folded.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { runInNewContext } from 'node:vm';
import { compile } from 'svelte/compiler';
import type { Component } from 'svelte';
import { render } from 'svelte/server';
import ts from 'typescript';
import { loadSource } from './sourceHarness.ts';

const workLog = loadSource(new URL('../src/modules/threads/runtime/workLog.ts', import.meta.url), {}, {}, { relative: true });
const turnChanges = loadSource(
  new URL('../src/modules/threads/runtime/turnChanges.ts', import.meta.url),
  { './workLog': workLog },
  {},
  { relative: true },
);

const nodeRequire = createRequire(import.meta.url);

function compileSsrComponent(
  fileUrl: URL,
  stubs: Record<string, unknown>,
): Component<Record<string, unknown>> {
  const source = readFileSync(fileUrl, 'utf8');
  const { js } = compile(source, {
    filename: fileUrl.pathname,
    generate: 'server',
    css: 'external',
  });
  const cjs = ts.transpileModule(js.code, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const mod = { exports: {} as Record<string, unknown> };
  const noopComponent = () => {};
  runInNewContext(
    cjs,
    {
      module: mod,
      exports: mod.exports,
      require: (spec: string) => {
        if (spec in stubs) return stubs[spec];
        if (spec.endsWith('.svelte')) return { __esModule: true, default: noopComponent };
        return nodeRequire(spec);
      },
      console,
    },
    { timeout: 5000, filename: fileUrl.pathname },
  );
  return mod.exports.default as Component<Record<string, unknown>>;
}

const textComponent = (prop: string) => (renderer: { push: (s: string) => void }, props: Record<string, unknown>) => {
  renderer.push(String(props[prop] ?? ''));
};

test('RuntimeTurnChanges starts folded above 5 files by default and open when startOpen is true', () => {
  const RuntimeTurnChanges = compileSsrComponent(
    new URL('../src/modules/threads/runtime/RuntimeTurnChanges.svelte', import.meta.url),
    { './turnChanges': turnChanges },
  );
  const files = Array.from({ length: 6 }, (_, i) => ({
    path: `src/file-${i + 1}.ts`,
    added: i + 1,
    removed: 0,
  }));

  const folded = render(RuntimeTurnChanges, { props: { files } }).body;
  assert.match(folded, /aria-expanded="false"/, 'normal Threads folds >5 changed files');
  assert.doesNotMatch(folded, /file-6\.ts/);

  const expanded = render(RuntimeTurnChanges, { props: { files, startOpen: true } }).body;
  assert.match(expanded, /aria-expanded="true"/, 'workflow expandWork opens >5 changed files on arrival');
  assert.match(expanded, /file-6\.ts/);
});

test('RuntimeWorkRow starts command output and long thoughts folded by default and open when startOpen is true', () => {
  const RuntimeWorkRow = compileSsrComponent(
    new URL('../src/modules/threads/runtime/RuntimeWorkRow.svelte', import.meta.url),
    {
      'svelte/reactivity': { SvelteMap: Map },
      '../../agents/conversation/Markdown.svelte': { __esModule: true, default: textComponent('md') },
      '../../agents/conversation/format': { lastLine: (s: string) => s.split('\n').at(-1) ?? '', patchToDiff: () => '' },
      './childThreads': { threadNoticeOf: () => null },
      '../../../lib/lazy-component.svelte': { lazyComponent: () => ({}) },
      './timeline': { errorText: (s: string) => s, openedTarget: () => null, runNoticeOf: () => null, toolError: () => null },
      './workLog': workLog,
      '../../../lib/stores/now.svelte': { now: () => 0 },
      '../../../lib/stores/workspace.svelte': { ws: {} },
      '../threadsStore.svelte': { threads: {} },
      './runtimeView.svelte': { runtimeViews: { loadingItem: {}, itemError: {}, ensureItemDetail: () => {} } },
    },
  );

  const cmdItem = {
    id: 'cmd-1',
    turn_id: 't1',
    ordinal: 1,
    type: 'command_execution',
    status: 'completed',
    title: null,
    input: 'npm test',
    output: 'ALL_TESTS_GREEN_MARKER',
    exit_code: 0,
    started_at: '2026-11-26T10:00:00Z',
    completed_at: '2026-11-26T10:00:02Z',
    updated_at: '2026-11-26T10:00:02Z',
  };

  const foldedCmd = render(RuntimeWorkRow, { props: { item: cmdItem } }).body;
  assert.match(foldedCmd, /aria-expanded="false"/, 'normal Threads folds command output');
  assert.doesNotMatch(foldedCmd, /ALL_TESTS_GREEN_MARKER/);

  const openCmd = render(RuntimeWorkRow, { props: { item: cmdItem, startOpen: true } }).body;
  assert.match(openCmd, /aria-expanded="true"/, 'workflow expandWork opens command output on arrival');
  assert.match(openCmd, /ALL_TESTS_GREEN_MARKER/);

  const longThoughtText = Array.from({ length: 8 }, (_, i) => `Reasoning line ${i + 1}: ${'detail '.repeat(12)}`).join('\n') + '\nFINAL_THOUGHT_MARKER';
  const thoughtItem = {
    id: 'think-1',
    turn_id: 't1',
    ordinal: 2,
    type: 'reasoning',
    status: 'completed',
    title: null,
    text: longThoughtText,
    started_at: '2026-11-26T10:00:00Z',
    completed_at: '2026-11-26T10:00:01Z',
    updated_at: '2026-11-26T10:00:01Z',
  };

  const foldedThought = render(RuntimeWorkRow, { props: { item: thoughtItem } }).body;
  assert.match(foldedThought, /aria-expanded="false"/, 'normal Threads clamps long thoughts behind More');
  assert.doesNotMatch(foldedThought, /FINAL_THOUGHT_MARKER/);

  const openThought = render(RuntimeWorkRow, { props: { item: thoughtItem, startOpen: true } }).body;
  assert.match(openThought, /aria-expanded="true"/, 'workflow expandWork shows full long thoughts on arrival');
  assert.match(openThought, /FINAL_THOUGHT_MARKER/);
  assert.doesNotMatch(openThought, /rw-detail/, 'open inline reasoning does not render an empty detail box');
});

test('RuntimeTimelineRows forwards expandWork (default false) to work rows, changed files, subagents, and attributed messages', () => {
  const seen: { workRow: unknown[]; turnChanges: unknown[]; subagent: unknown[] } = {
    workRow: [],
    turnChanges: [],
    subagent: [],
  };
  const spy = (bucket: unknown[], key: string) => (_r: unknown, props: Record<string, unknown>) => {
    bucket.push(props[key]);
  };
  const passthrough = (r: unknown, props: Record<string, unknown>) => {
    const children = props.children as ((renderer: unknown) => void) | undefined;
    children?.(r);
  };
  const timeline = loadSource(new URL('../src/modules/threads/runtime/timeline.ts', import.meta.url), {}, {}, { relative: true });
  const RuntimeTimelineRows = compileSsrComponent(
    new URL('../src/modules/threads/runtime/RuntimeTimelineRows.svelte', import.meta.url),
    {
      'svelte/reactivity': { SvelteMap: Map, SvelteSet: Set },
      '../../../lib/lazy-component.svelte': { lazyComponent: () => ({}) },
      './reviewNotes': { REVIEW_HEADER: 'REVIEW', sentReview: () => null },
      '../../../lib/clipboard': { copyWithToast: () => {} },
      '../../../lib/pathLinks/open': { activatePathLink: () => {} },
      '../../agents/conversation/Markdown.svelte': { __esModule: true, default: textComponent('md') },
      './agentRows': { threadCall: () => null },
      './mentionMatcher': { parseUserMessageItemWithBadges: (i: { text: string }) => ({ badges: [], prose: i.text }) },
      './childThreads': { memoTurnSubagents: () => ({ get: () => ({ groups: new Map(), skip: new Set() }) }) },
      './stableMap': { keepIfSame: (_p: unknown, next: unknown) => next },
      './timeline': timeline,
      './turnChanges': turnChanges,
      './workLog': workLog,
      './threadPrefs.svelte': { threadPrefs: { tools: 'all' } },
      './turnUsage': { turnUsageText: () => '' },
      './providerSwitch': { handoffSummary: () => '', handoffTitle: () => '' },
      './tapMeta': { tapMeta: () => {} },
      './RuntimeWorkFold.svelte': { __esModule: true, default: passthrough },
      './RuntimeWorkGroup.svelte': { __esModule: true, default: passthrough },
      './RuntimeWorkRow.svelte': { __esModule: true, default: spy(seen.workRow, 'startOpen') },
      './RuntimeTurnChanges.svelte': { __esModule: true, default: spy(seen.turnChanges, 'startOpen') },
      './RuntimeSubagentCard.svelte': { __esModule: true, default: spy(seen.subagent, 'startOpen') },
    },
  );

  const longPeer = `[Reply from subagent "Scout" (id: s2)]\n\n${'A long attributed subagent reply. '.repeat(14)}`;
  const projection = {
    lastSequence: 10,
    session: null,
    thread: null,
    turns: [{ id: 't1', ordinal: 1, status: 'completed', started_at: '2026-11-26T10:00:00Z', completed_at: '2026-11-26T10:00:05Z' }],
    items: [
      { id: 'u1', turn_id: 't1', ordinal: 0, type: 'user_message', status: 'completed', title: null, text: longPeer, started_at: '2026-11-26T10:00:00Z', completed_at: '2026-11-26T10:00:00Z', updated_at: '2026-11-26T10:00:00Z' },
      { id: 'cmd-1', turn_id: 't1', ordinal: 1, type: 'command_execution', status: 'completed', title: null, input: 'echo ok', output: 'ok', exit_code: 0, started_at: '2026-11-26T10:00:01Z', completed_at: '2026-11-26T10:00:02Z', updated_at: '2026-11-26T10:00:02Z' },
      { id: 'fc-1', turn_id: 't1', ordinal: 2, type: 'file_change', status: 'completed', title: null, file_name: 'a.ts', diff_str: '+x\n', started_at: '2026-11-26T10:00:02Z', completed_at: '2026-11-26T10:00:03Z', updated_at: '2026-11-26T10:00:03Z' },
      { id: 'sa-1', turn_id: 't1', ordinal: 3, type: 'subagent', status: 'completed', title: 'Sub', tool_uses: 1, started_at: '2026-11-26T10:00:03Z', completed_at: '2026-11-26T10:00:04Z', updated_at: '2026-11-26T10:00:04Z' },
    ],
    requests: [],
    plans: [],
    queued: [],
    output: {},
  };

  const baseProps = { sessionId: 's1', projection, agentName: 'Claude', onrespond: async () => {} };
  const foldedHtml = render(RuntimeTimelineRows, { props: baseProps }).body;
  assert.match(foldedHtml, /class="rt-from-body[^"]*folded"/, 'default Threads clamps long attributed messages');
  assert.deepEqual(seen.turnChanges, [false], 'default Threads keeps RuntimeTurnChanges startOpen=false');

  seen.workRow.length = 0;
  seen.turnChanges.length = 0;
  seen.subagent.length = 0;
  const openHtml = render(RuntimeTimelineRows, { props: { ...baseProps, expandWork: true } }).body;
  assert.doesNotMatch(openHtml, /class="rt-from-body[^"]*folded"/, 'expandWork removes the attributed-message clamp');
  assert.ok(seen.workRow.length > 0 && seen.workRow.every((v) => v === true), 'expandWork passes startOpen=true to RuntimeWorkRow');
  assert.deepEqual(seen.turnChanges, [true], 'expandWork passes startOpen=true to RuntimeTurnChanges');
  assert.deepEqual(seen.subagent, [true], 'expandWork passes startOpen=true to RuntimeSubagentCard');
});
