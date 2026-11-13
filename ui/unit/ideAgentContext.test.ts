import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { test } from 'node:test';
import {
  buildIdeAgentPrompt,
  computeLineDiff,
  detectProposalLanguage,
  extractCodeProposals,
  IDE_PROPOSAL_INSTRUCTION,
  lineRangeFromOffsets,
  MAX_CHIP_SNIPPET_CHARS,
  MAX_CONTEXT_CHIPS,
  offsetsForLineRange,
  parseFenceInfo,
  parseIdeContextChipsFromPrompt,
  spliceCodeRange,
  stripIdeContextBlocks,
  truncateSnippet,
  type IdeContextChip,
} from '../src/modules/ide/store/ideDiffApplier.ts';

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

  const t0 = performance.now();
  const largeDiff = computeLineDiff(beforeLines.join('\n'), afterLines.join('\n'));
  const elapsedMs = performance.now() - t0;

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
