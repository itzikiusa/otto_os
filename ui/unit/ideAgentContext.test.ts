import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { test } from 'node:test';
import {
  buildPureSymbolDraftChip,
  upsertIdeContextChipList,
} from '../src/modules/ide/agent/ideSymbolChip.ts';
import {
  buildIdeAgentPrompt,
  computeLineDiff,
  detectProposalLanguage,
  extractCodeProposals,
  IDE_PROPOSAL_INSTRUCTION,
  IDE_RUNTIME_PROVIDERS,
  lineRangeFromOffsets,
  matchesIdeProjectThread,
  MAX_CHIP_SNIPPET_CHARS,
  MAX_CONTEXT_CHIPS,
  offsetsForLineRange,
  parseFenceInfo,
  parseIdeContextChipsFromPrompt,
  resolveIdeHarnesses,
  resolveIdeInlineProviders,
  spliceCodeRange,
  stripIdeContextBlocks,
  truncateSnippet,
  type IdeContextChip,
} from '../src/modules/ide/store/ideDiffApplier.ts';
import { loadSource } from './sourceHarness.ts';

const SAMPLE_GO = [
  'package calc',
  '',
  'func Add(a, b int) int {',
  '\treturn a + b',
  '}',
  '',
  'func Sub(a, b int) int {',
  '\treturn a - b',
  '}',
  '',
].join('\n');

function makeChip(overrides: Partial<IdeContextChip> = {}): IdeContextChip {
  return {
    id: 'ctx-1',
    key: 'selection:calc/calc.go:3-5',
    kind: 'selection',
    label: '@calc/calc.go:L3-L5',
    sublabel: '3 lines',
    path: 'calc/calc.go',
    startLine: 3,
    endLine: 5,
    language: 'go',
    snippet: 'func Add(a, b int) int {\n\treturn a + b\n}',
    ...overrides,
  };
}

test('buildIdeAgentPrompt wraps context, enforces caps, and round-trips metadata', () => {
  const huge = 'x'.repeat(MAX_CHIP_SNIPPET_CHARS + 500);
  const chipA = makeChip({ label: '@calc/calc.go:L3-L5 "main" & <tag>' });
  const chipB = makeChip({
    id: 'ctx-2',
    key: 'file:notes.md',
    kind: 'file',
    label: '@notes.md',
    path: 'notes.md',
    startLine: 1,
    endLine: 20,
    language: 'markdown',
    snippet: `Literal closing tag </ide_context> inside snippet\n${huge}`,
  });

  const prompt = buildIdeAgentPrompt('Refactor Add for clarity', [chipA, chipB], {
    workspaceRoot: '/ws/demo',
    activePath: 'calc/calc.go',
    planMode: true,
  });

  assert.ok(prompt.startsWith('<ide_context>\n'));
  assert.ok(prompt.includes('<workspace root="/ws/demo" active_file="calc/calc.go" />'));
  assert.ok(prompt.includes(IDE_PROPOSAL_INSTRUCTION));
  assert.ok(prompt.includes('Plan mode:'));
  assert.ok(prompt.includes('… [truncated]'));
  assert.ok(prompt.endsWith('\n</ide_context>\n\nRefactor Add for clarity'));

  const parsed = parseIdeContextChipsFromPrompt(prompt);
  assert.equal(parsed.length, 2);
  assert.equal(parsed[0].kind, 'selection');
  assert.equal(parsed[0].label, '@calc/calc.go:L3-L5 "main" & <tag>');
  assert.equal(parsed[0].path, 'calc/calc.go');
  assert.equal(parsed[0].startLine, 3);
  assert.equal(parsed[0].endLine, 5);
  assert.equal(parsed[1].kind, 'file');
  assert.equal(parsed[1].path, 'notes.md');

  // Escaped `</ide_context>` inside snippet must never terminate stripping early.
  assert.equal(stripIdeContextBlocks(prompt), 'Refactor Add for clarity');

  // Enforces MAX_CONTEXT_CHIPS (12) and tight maxContextChars budget.
  const manyChips = Array.from({ length: MAX_CONTEXT_CHIPS + 5 }, (_, i) =>
    makeChip({ id: `c-${i}`, key: `k-${i}`, label: `@f${i}.go`, path: `f${i}.go`, snippet: 'line '.repeat(60) }),
  );
  const cappedByCount = buildIdeAgentPrompt('Go', manyChips);
  assert.equal(parseIdeContextChipsFromPrompt(cappedByCount).length, MAX_CONTEXT_CHIPS);

  const tight = buildIdeAgentPrompt('Go', manyChips, { maxContextChars: 900 });
  const xmlOnly = tight.slice(0, tight.indexOf('</ide_context>') + '</ide_context>'.length);
  assert.ok(xmlOnly.length <= 900, `expected <= 900 chars, got ${xmlOnly.length}`);
  assert.equal(truncateSnippet('abcdef', 4), 'abcd');
});

test('stripIdeContextBlocks + extractCodeProposals handle otto-fake-claude echo safely', () => {
  // Context snippet intentionally embeds a fenced Go block to prove echoed context never leaks!
  const contextWithFence = makeChip({
    kind: 'file',
    label: '@README.md',
    path: 'README.md',
    language: 'markdown',
    snippet: 'Example:\n```go path=calc/calc.go\npackage wrong_from_context\n```',
  });

  // 1. Echoed prompt WITHOUT a proposal fence -> 0 proposals extracted.
  const askOnlyPrompt = buildIdeAgentPrompt('Explain what Add does.', [contextWithFence, makeChip()], {
    workspaceRoot: '/ws',
    activePath: 'calc/calc.go',
  });
  const echoedAsk = `echo: ${askOnlyPrompt}`;
  assert.equal(extractCodeProposals(echoedAsk, 'calc/calc.go').length, 0);

  // 2. Echoed prompt WITH a real proposal fence -> extracts ONLY the real proposal outside <ide_context>.
  const updatedGo = [
    'package calc',
    '',
    'func Add(a, b int) int {',
    '\treturn a + b + 0',
    '}',
  ].join('\n');
  const userWithFence = `Apply this update:\n\`\`\`go path=calc/calc.go\n${updatedGo}\n\`\`\``;
  const fullPrompt = buildIdeAgentPrompt(userWithFence, [contextWithFence, makeChip()], {
    workspaceRoot: '/ws',
    activePath: 'calc/calc.go',
  });
  const echoedTurn = `echo: ${fullPrompt}`;

  const proposals = extractCodeProposals(echoedTurn, 'calc/calc.go', { startLine: 3, endLine: 5 }, {
    messageId: 'm-echo',
    resolvePath: (p) => `/ws/${p}`,
    relativePath: (p) => p.replace(/^\/ws\//, ''),
    getFileContent: () => SAMPLE_GO,
  });

  assert.equal(proposals.length, 1);
  assert.equal(proposals[0].id, 'm-echo:p0');
  assert.equal(proposals[0].path, '/ws/calc/calc.go');
  assert.equal(proposals[0].relPath, 'calc/calc.go');
  assert.equal(proposals[0].language, 'go');
  assert.equal(proposals[0].code, updatedGo);
  assert.equal(proposals[0].range, undefined);

  // 3. Single-line `echo: ```go path=...` without <ide_context> also extracts cleanly.
  const directEcho = `echo: \`\`\`go path=calc/calc.go\n${updatedGo}\n\`\`\``;
  const direct = extractCodeProposals(directEcho);
  assert.equal(direct.length, 1);
  assert.equal(direct[0].relPath, 'calc/calc.go');
});

test('parseFenceInfo & extractCodeProposals support all fence path/range/fallback styles', () => {
  assert.deepEqual(parseFenceInfo('go path=calc/calc.go'), {
    language: 'go',
    path: 'calc/calc.go',
    lineRange: null,
    isPlaceholder: false,
  });
  assert.deepEqual(parseFenceInfo('go calc/calc.go'), {
    language: 'go',
    path: 'calc/calc.go',
    lineRange: null,
    isPlaceholder: false,
  });
  assert.deepEqual(parseFenceInfo('title="web/calc.mjs"'), {
    language: 'javascript',
    path: 'web/calc.mjs',
    lineRange: null,
    isPlaceholder: false,
  });
  assert.deepEqual(parseFenceInfo('ts title="web/calc.mjs"'), {
    language: 'ts',
    path: 'web/calc.mjs',
    lineRange: null,
    isPlaceholder: false,
  });
  assert.deepEqual(parseFenceInfo('go path=calc/calc.go:L3-L5'), {
    language: 'go',
    path: 'calc/calc.go',
    lineRange: { startLine: 3, endLine: 5 },
    isPlaceholder: false,
  });
  assert.equal(parseFenceInfo('<lang> path=<relative-path>').isPlaceholder, true);
  assert.equal(detectProposalLanguage('ui/App.svelte'), 'svelte');

  const multiReply = [
    'Run tests first:',
    '```bash',
    'go test ./...',
    '```',
    '',
    'Updated function:',
    '```go path=calc/calc.go:L3-L5',
    'func Add(a, b int) int {',
    '\t// sum two ints',
    '\treturn a + b',
    '}',
    '```',
    '',
    'And helper module:',
    '```ts title="web/calc.mjs"',
    'export const add = (a, b) => a + b;',
    '```',
  ].join('\n');

  const props = extractCodeProposals(multiReply, 'calc/calc.go', null, {
    messageId: 'msg-42',
    appliedIds: { 'msg-42:p0': 'snap' },
    getFileContent: (p) => (p === 'calc/calc.go' ? SAMPLE_GO : ''),
  });

  // Plain ```bash fence must be ignored when fallbackPath is Go; the 2 file fences are extracted.
  assert.equal(props.length, 2);
  assert.equal(props[0].relPath, 'calc/calc.go');
  assert.equal(props[0].applied, true);
  assert.deepEqual(props[0].range, { from: 14, to: 54, startLine: 3, endLine: 5 });
  assert.equal(props[0].addedCount, 1);
  assert.equal(props[0].removedCount, 0);
  assert.equal(props[1].relPath, 'web/calc.mjs');
  assert.equal(props[1].applied, false);

  // Plain ```go fence falls back to fallbackPath + fallbackRange for partial snippet,
  // but promotes to full-file replacement when snippet starts with line 1 (`package calc`).
  const partialPlain = '```go\nfunc Add(a, b int) int {\n\treturn a + b + 1\n}\n```';
  const [partialProp] = extractCodeProposals(
    partialPlain,
    'calc/calc.go',
    { startLine: 3, endLine: 5 },
    { getFileContent: () => SAMPLE_GO },
  );
  assert.deepEqual(partialProp.range, { from: 14, to: 54, startLine: 3, endLine: 5 });

  const fullPlain = `\`\`\`go\n${SAMPLE_GO}\`\`\``;
  const [fullProp] = extractCodeProposals(
    fullPlain,
    'calc/calc.go',
    { startLine: 3, endLine: 5 },
    { getFileContent: () => SAMPLE_GO },
  );
  assert.equal(fullProp.range, undefined);
});

test('computeLineDiff produces exact line numbers and stays fast past 600 lines', () => {
  assert.deepEqual(computeLineDiff('', ''), []);

  const small = computeLineDiff('a\nb\nc\n', 'a\nb2\nc\nd\n');
  assert.deepEqual(small, [
    { kind: 'same', text: 'a', oldLine: 1, newLine: 1 },
    { kind: 'del', text: 'b', oldLine: 2, newLine: null },
    { kind: 'add', text: 'b2', oldLine: null, newLine: 2 },
    { kind: 'same', text: 'c', oldLine: 3, newLine: 3 },
    { kind: 'add', text: 'd', oldLine: null, newLine: 4 },
  ]);

  // > 600 lines exercises uniqueAnchors LIS + bounded DP fast path.
  const beforeLines = Array.from({ length: 900 }, (_, i) => `line_${i} = ${i}`);
  const afterLines = beforeLines.slice();
  afterLines[120] = 'line_120 = updated_120';
  afterLines.splice(450, 2, 'inserted_a', 'inserted_b', 'inserted_c');
  afterLines[780] = 'line_780 = updated_780';

  // Best of five: the bound is about the algorithm (the bounded fast path),
  // not a cold JIT or a busy shared runner — one slow sample failed CI at 57 ms.
  const beforeText = beforeLines.join('\n');
  const afterText = afterLines.join('\n');
  let largeDiff = computeLineDiff(beforeText, afterText);
  let elapsedMs = Infinity;
  for (let i = 0; i < 5; i++) {
    const t0 = performance.now();
    largeDiff = computeLineDiff(beforeText, afterText);
    elapsedMs = Math.min(elapsedMs, performance.now() - t0);
  }

  const reconBefore = largeDiff.filter((d) => d.kind !== 'add').map((d) => d.text);
  const reconAfter = largeDiff.filter((d) => d.kind !== 'del').map((d) => d.text);
  assert.deepEqual(reconBefore, beforeLines);
  assert.deepEqual(reconAfter, afterLines);
  assert.equal(largeDiff.filter((d) => d.kind === 'del').length, 4);
  assert.equal(largeDiff.filter((d) => d.kind === 'add').length, 5);
  assert.ok(elapsedMs < 50, `expected sub-50ms large diff, took ${elapsedMs.toFixed(2)}ms`);
});

test('offsetsForLineRange, lineRangeFromOffsets, and spliceCodeRange preserve surrounding lines', () => {
  // Collapsed cursor on line 4 (`\treturn a + b`) selects the whole line without trailing newline.
  const line3Range = offsetsForLineRange(SAMPLE_GO, 3, 5);
  assert.equal(line3Range.text, 'func Add(a, b int) int {\n\treturn a + b\n}');

  const cursorPos = SAMPLE_GO.indexOf('return a + b') + 3;
  const collapsed = lineRangeFromOffsets(SAMPLE_GO, cursorPos, cursorPos);
  assert.equal(collapsed.startLine, 4);
  assert.equal(collapsed.endLine, 4);
  assert.equal(collapsed.text, '\treturn a + b');

  // Selection with trailing newline (`to` right after `\n` of line 5) still reports endLine = 5
  // and splices cleanly without losing or doubling the newline before line 6.
  const selWithNl = lineRangeFromOffsets(SAMPLE_GO, line3Range.from, line3Range.to + 1);
  assert.equal(selWithNl.startLine, 3);
  assert.equal(selWithNl.endLine, 5);

  const replacement = 'func Add(a, b int) int {\n\treturn a + b + 1\n}';
  const splicedByOffsets = spliceCodeRange(SAMPLE_GO, replacement, {
    from: selWithNl.from,
    to: selWithNl.to,
  });
  const splicedByLines = spliceCodeRange(SAMPLE_GO, `${replacement}\n`, {
    startLine: 3,
    endLine: 5,
  });
  const expected = SAMPLE_GO.replace('\treturn a + b', '\treturn a + b + 1');
  assert.equal(splicedByOffsets, expected);
  assert.equal(splicedByLines, expected);
});

test('resolveIdeHarnesses supports all enabled Threads providers + custom CLIs while respecting disabled_providers', () => {
  const harnesses = resolveIdeHarnesses(
    ['claude', 'codex', 'agy', 'gemini', 'grok', 'opencode', 'pi', 'muse', 'custom-qwen', 'shell'],
    ['custom-deepseek', 'shell', null, '  '],
  );

  for (const runtimeProv of IDE_RUNTIME_PROVIDERS) {
    assert.ok(harnesses.includes(runtimeProv), `expected ${runtimeProv} in IDE harnesses`);
  }
  assert.ok(harnesses.includes('custom-qwen'));
  assert.ok(harnesses.includes('custom-deepseek'));
  assert.ok(!harnesses.includes('shell'));
  assert.equal(new Set(harnesses).size, harnesses.length);

  // Disabled built-ins (omitted from `/meta.providers`) must NOT sneak back in.
  const subset = resolveIdeHarnesses(['codex', 'opencode', 'custom-kilo', 'shell']);
  assert.deepEqual(subset, ['codex', 'opencode', 'custom-kilo']);
  assert.deepEqual(resolveIdeInlineProviders(subset, ['codex', 'opencode', 'custom-kilo']), ['codex', 'opencode']);
  assert.deepEqual(resolveIdeInlineProviders(['custom-kilo'], ['custom-kilo']), []);
});

test('matchesIdeProjectThread matches primary root, attached multi-folder roots, macOS /private/var symlinks, and workspace fallback', () => {
  const roots = ['/var/folders/ab/proj-main', '/Users/dev/shared-lib'];

  assert.equal(
    matchesIdeProjectThread({
      sessionId: 's-exact',
      workspaceId: 'scratch',
      targetWorkspaceId: 'ws-1',
      cwd: '/private/var/folders/ab/proj-main/pkg/sub',
      projectRoots: roots,
    }),
    true,
  );

  assert.equal(
    matchesIdeProjectThread({
      sessionId: 's-attached',
      workspaceId: 'ws-other',
      targetWorkspaceId: 'ws-1',
      cwd: '/Users/dev/shared-lib',
      projectRoots: roots,
    }),
    true,
  );

  assert.equal(
    matchesIdeProjectThread({
      sessionId: 's-unrelated',
      workspaceId: 'ws-1',
      targetWorkspaceId: 'ws-1',
      cwd: '/Users/dev/unrelated-repo',
      projectRoots: roots,
    }),
    false,
  );

  // Stale `activeSessionId` from a different folder on disk must NOT leak across workspace switches.
  assert.equal(
    matchesIdeProjectThread({
      sessionId: 's-stale-project-a',
      activeSessionId: 's-stale-project-a',
      workspaceId: 'ws-other',
      targetWorkspaceId: 'ws-1',
      cwd: '/Users/dev/unrelated-repo',
      projectRoots: roots,
    }),
    false,
  );

  // When `projectRoots` is empty (e.g. SSH target), `activeSessionId` stays visible.
  assert.equal(
    matchesIdeProjectThread({
      sessionId: 's-ssh-active',
      activeSessionId: 's-ssh-active',
      workspaceId: 'ws-ssh',
      targetWorkspaceId: 'ws-1',
      cwd: '/home/ubuntu/app',
      projectRoots: [],
    }),
    true,
  );

  // `resolveIdeInlineProviders` restricts `⌘I` inline edit to structured runtime providers.
  const inline = resolveIdeInlineProviders(['claude', 'aider-custom', 'agy', 'grok', 'shell', 'opencode', 'codex']);
  assert.deepEqual(inline, ['claude', 'agy', 'grok', 'opencode', 'codex']);
  assert.deepEqual(resolveIdeInlineProviders(['aider-custom', 'shell']), [...IDE_RUNTIME_PROVIDERS]);

  assert.equal(
    matchesIdeProjectThread({
      sessionId: 's-ws-only',
      workspaceId: 'ws-1',
      targetWorkspaceId: 'ws-1',
      cwd: null,
      projectRoots: roots,
    }),
    true,
  );
});

test('buildIdeAgentPrompt omits empty wrapper when omitWhenEmpty is set and supports run_output & all 8 languages', () => {
  assert.equal(
    buildIdeAgentPrompt('Explain this module', [], {
      workspaceRoot: '/ws/demo',
      activePath: 'main.go',
      planMode: false,
      omitWhenEmpty: true,
    }),
    'Explain this module',
  );

  // Plan mode still includes `<ide_context>` instructions even with 0 context chips.
  const planOnly = buildIdeAgentPrompt('Design refactor plan', [], {
    workspaceRoot: '/ws/demo',
    planMode: true,
    omitWhenEmpty: true,
  });
  assert.ok(planOnly.startsWith('<ide_context>\n'));
  assert.ok(planOnly.includes('Plan mode:'));

  // `run_output` chip round-trips cleanly.
  const runChip = makeChip({
    id: 'ctx-run',
    key: 'run:r1',
    kind: 'run_output',
    label: '✗ go test ./...',
    path: undefined,
    startLine: undefined,
    endLine: undefined,
    language: undefined,
    snippet: 'Run: go test ./... (failed, exit 1)\n\nFAIL calc/calc_test.go:14',
  });
  const withRun = buildIdeAgentPrompt('Fix failing run', [runChip], { omitWhenEmpty: true });
  const parsedRun = parseIdeContextChipsFromPrompt(withRun);
  assert.equal(parsedRun.length, 1);
  assert.equal(parsedRun[0].kind, 'run_output');
  assert.equal(parsedRun[0].label, '✗ go test ./...');

  // All 8 required languages/formats detect accurately.
  assert.equal(detectProposalLanguage('src/index.mts'), 'typescript');
  assert.equal(detectProposalLanguage('src/legacy.cjs'), 'javascript');
  assert.equal(detectProposalLanguage('cmd/server/main.go'), 'go');
  assert.equal(detectProposalLanguage('jdt://contents/java.base/java.util/List.class?foo=1'), 'java');
  assert.equal(detectProposalLanguage('crates/core/src/lib.rs'), 'rust');
  assert.equal(detectProposalLanguage('typings/app.pyi'), 'python');
  assert.equal(detectProposalLanguage('scripts/deploy.zsh'), 'shell');
  assert.equal(detectProposalLanguage('/Users/dev/.zshrc'), 'shell');
  assert.equal(detectProposalLanguage('config/tsconfig.jsonc'), 'json');
  assert.equal(detectProposalLanguage('events/trace.jsonl'), 'jsonl');
  assert.equal(detectProposalLanguage('data/metrics.csv'), 'csv');
  assert.equal(detectProposalLanguage('data/metrics.tsv'), 'tsv');
});

test('attachSymbol (buildPureSymbolDraftChip + upsertIdeContextChipList) formats @relPath#SymbolName and slices exact symbol lines', () => {
  const draftAdd = buildPureSymbolDraftChip('/ws/demo/calc/calc.go', 'calc/calc.go', SAMPLE_GO, {
    name: 'Add',
    kind: 'function',
    line: 3,
    end_line: 5,
  });
  assert.equal(draftAdd.kind, 'symbol');
  assert.equal(draftAdd.label, '@calc/calc.go#Add');
  assert.equal(draftAdd.sublabel, 'L3-L5');
  assert.equal(draftAdd.path, '/ws/demo/calc/calc.go');
  assert.equal(draftAdd.startLine, 3);
  assert.equal(draftAdd.endLine, 5);
  assert.equal(draftAdd.language, 'go');
  assert.equal(draftAdd.snippet, 'func Add(a, b int) int {\n\treturn a + b\n}');

  let seq = 0;
  const first = upsertIdeContextChipList([], draftAdd, () => `ctx-${++seq}`);
  assert.equal(first.next.length, 1);
  assert.equal(first.chip.id, 'ctx-1');
  assert.equal(first.chip.label, '@calc/calc.go#Add');

  // Re-attaching the same symbol updates in place without duplicating the chip id.
  const updatedBuf = SAMPLE_GO.replace('return a + b', 'return a + b + 1');
  const draftAddEdited = buildPureSymbolDraftChip('/ws/demo/calc/calc.go', 'calc/calc.go', updatedBuf, {
    name: 'Add',
    kind: 'function',
    line: 3,
    end_line: 5,
  });
  const second = upsertIdeContextChipList(first.next, draftAddEdited, () => `ctx-${++seq}`);
  assert.equal(second.next.length, 1);
  assert.equal(second.chip.id, 'ctx-1');
  assert.ok(second.chip.snippet.includes('return a + b + 1'));

  // Empty buffer falls back cleanly to `<kind> <name> (L<start>-L<end>)` and round-trips through `<ide_context>`.
  const fallbackSub = buildPureSymbolDraftChip('/ws/demo/calc/calc.go', 'calc/calc.go', '', {
    name: 'Sub',
    kind: 'function',
    line: 7,
    end_line: 9,
  });
  assert.equal(fallbackSub.label, '@calc/calc.go#Sub');
  assert.equal(fallbackSub.snippet, 'function Sub (L7-L9)');

  const third = upsertIdeContextChipList(second.next, fallbackSub, () => `ctx-${++seq}`);
  assert.equal(third.next.length, 2);

  const prompt = buildIdeAgentPrompt('Optimize symbol implementations', third.next, {
    workspaceRoot: '/ws/demo',
    omitWhenEmpty: true,
  });
  const parsed = parseIdeContextChipsFromPrompt(prompt);
  assert.equal(parsed.length, 2);
  assert.equal(parsed[0].kind, 'symbol');
  assert.equal(parsed[0].label, '@calc/calc.go#Add');
  assert.equal(parsed[0].startLine, 3);
  assert.equal(parsed[0].endLine, 5);
  assert.equal(parsed[1].kind, 'symbol');
  assert.equal(parsed[1].label, '@calc/calc.go#Sub');
});

test('resolveIdeHarnesses and resolveIdeInlineProviders honor empty provider lists and canonical harness ordering', () => {
  assert.deepEqual(resolveIdeHarnesses([]), []);
  assert.deepEqual(resolveIdeHarnesses(['shell']), []);
  assert.deepEqual(
    resolveIdeHarnesses(['muse', 'grok', 'gemini', 'pi', 'opencode', 'agy', 'codex', 'claude', 'shell']),
    ['claude', 'codex', 'agy', 'opencode', 'pi', 'gemini', 'grok', 'muse'],
  );
  assert.deepEqual(IDE_RUNTIME_PROVIDERS, [
    'claude',
    'codex',
    'agy',
    'opencode',
    'pi',
    'gemini',
    'grok',
    'muse',
  ]);

  assert.deepEqual(resolveIdeInlineProviders(['claude', 'codex'], []), []);
  assert.deepEqual(resolveIdeInlineProviders(['claude', 'codex'], ['shell']), []);
  assert.deepEqual(resolveIdeInlineProviders(['claude', 'codex'], ['codex']), ['codex']);
});

test('ideContextBuilders exposes active draft builders without unused selection or inline-prompt helpers', () => {
  const builders = loadSource(
    new URL('../src/modules/ide/agent/ideContextBuilders.ts', import.meta.url),
    {
      svelte: { untrack: <T>(fn: () => T) => fn() },
      'lib/api/client': { api: { get: async () => ({ content: '', language: '' }) } },
      'lib/stores/auth.svelte': { auth: { meta: null } },
      'lib/stores/workspace.svelte': { SCRATCH_WORKSPACE_ID: 'scratch', ws: { workspaces: [], currentId: 'w1' } },
      'modules/ide/ideStore.svelte': {
        ide: {
          target: { kind: 'local', root: '/ws/demo', workspace_id: 'w1' },
          tabs: [],
          projectEnv: null,
          resolvePath: (p: string) => p,
          relativePath: (p: string) => p.replace(/^\/ws\/demo\//, ''),
          targetQuery: () => '',
        },
        ideBasename: (p: string) => p.split('/').pop() ?? p,
      },
    },
    {},
    { relative: true },
  ) as Record<string, unknown>;

  assert.equal(typeof builders.buildSymbolDraftChip, 'function');
  assert.equal(typeof builders.buildDiagnosticDraftChip, 'function');
  assert.equal(typeof builders.buildTestFailureDraftChip, 'function');
  assert.equal(typeof builders.buildFailedRunDraftChip, 'function');
  assert.equal(typeof builders.buildDebugFrameDraftChip, 'function');

  assert.equal('upsertIdeContextChip' in builders, false);
  assert.equal('buildSelectionDraftChip' in builders, false);
  assert.equal('buildIdeInlinePromptState' in builders, false);
  assert.equal('resolveIdeEditorSelection' in builders, false);
});

test('runtimeThreadPort segregates headless RuntimeSessionPort from lazy RuntimeComposerUiPort', async () => {
  const portMod = loadSource(
    new URL('../src/lib/runtimeThreadPort.ts', import.meta.url),
    { 'lib/stores/router.svelte': { router: { go: () => {} } } },
    {},
    { relative: true },
  ) as typeof import('../src/lib/runtimeThreadPort.ts');

  let uiLoadCalls = 0;
  const fakeUiPort = {
    council: {
      on: false,
      seats: [],
      ready: false,
      watchModifiers: () => () => {},
      pick: () => {},
      start: async () => [],
    },
    councilAgentLabel: () => 'Council',
    askTargets: { for: () => [], clear: () => {} },
    CouncilStagePicker: (() => null) as unknown as import('../src/lib/runtimeThreadPort.ts').RuntimeComposerUiPort['CouncilStagePicker'],
    RuntimeAccessChip: (() => null) as unknown as import('../src/lib/runtimeThreadPort.ts').RuntimeComposerUiPort['RuntimeAccessChip'],
    RuntimeComposerImages: (() => null) as unknown as import('../src/lib/runtimeThreadPort.ts').RuntimeComposerUiPort['RuntimeComposerImages'],
    RuntimeMentionMenu: (() => null) as unknown as import('../src/lib/runtimeThreadPort.ts').RuntimeComposerUiPort['RuntimeMentionMenu'],
    StageSlashPopup: (() => null) as unknown as import('../src/lib/runtimeThreadPort.ts').RuntimeComposerUiPort['StageSlashPopup'],
  };

  const fakeSessionPort = {
    runtimeThreads: {} as import('../src/lib/runtimeThreadPort.ts').RuntimeSessionPort['runtimeThreads'],
    runtimeViews: { projections: {}, open: () => () => {} },
    threads: { rememberProject: () => {}, selectThread: () => {}, startNewThread: () => {} },
    readAttachments: async () => [],
    loadComposerUi: async () => {
      uiLoadCalls++;
      return fakeUiPort;
    },
  };

  portMod.registerIdeRuntimeSessionPort(async () => fakeSessionPort);

  const headless = await portMod.loadIdeRuntimeSessionPort();
  assert.equal(headless, fakeSessionPort);
  assert.equal(portMod.getIdeRuntimeSessionPort(), fakeSessionPort);
  assert.equal(await portMod.loadIdeRuntimeBridge(), fakeSessionPort);
  assert.equal(uiLoadCalls, 0, 'headless session port must not eagerly load composer UI components');
  assert.equal(portMod.getIdeRuntimeComposerUi(), null);

  const ui = await portMod.loadIdeRuntimeComposerUi();
  assert.equal(ui, fakeUiPort);
  assert.equal(uiLoadCalls, 1);
  assert.equal(portMod.getIdeRuntimeComposerUi(), fakeUiPort);
});
