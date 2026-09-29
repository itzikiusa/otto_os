// Chat v2 (docs/design/conversation-view.md §5.2, rev 6): the pure pieces
// behind the redesigned conversation — the settled-response fold, the
// changed-files card, token usage, day dividers, paths, the chat's markdown
// extensions (file references, PR / issue chips, fence titles), linked command
// output, whole-block highlighting split into lines, and the side panel's
// preview parsers.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { Marked } from 'marked';
import {
  addUsage,
  changedDiff,
  changedFiles,
  dayKey,
  fmtDay,
  foldResponse,
  foldStats,
  groupTurns,
  patchStats,
  relPath,
  resolvePath,
  segment,
  usageParts,
  type ToolCallBlock,
} from '../src/modules/agents/conversation/format.ts';
import { htmlDoc, parseDelimited, prettyJson, previewKind } from '../src/modules/agents/conversation/preview.ts';
import { ensureHljs, highlightLines, splitHighlighted } from '../src/lib/hl.ts';
import type { Block, ToolKind, ToolResult, Turn, TurnUsage } from '../src/lib/api/types.ts';

// chatMarkdown / mdRender import siblings without an extension (Vite
// resolution); node needs `.ts` appended.
const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));
const chat = await import('../src/modules/agents/conversation/chatMarkdown.ts');
const md = await import('../src/modules/vault/mdRender.ts');

const ok = (extra: Partial<ToolResult> = {}): ToolResult => ({
  ok: true, text: 'ok', truncated: false, bytes: 2, image_ids: [], patch: null, file_path: null, ...extra,
});
const call = (id: string, tool: ToolKind, input: unknown, result: ToolResult | null = ok()): ToolCallBlock => ({
  kind: 'tool_call', id, name: 'Tool', tool, title: '', input, result,
});
const text = (s: string): Block => ({ kind: 'text', md: s });
const think: Block = { kind: 'thinking', count: 1 };

const PATCH = ['--- a/src/a.rs', '+++ b/src/a.rs', '@@ -1,3 +1,3 @@', ' keep', '-old', '+new', '+more', ' keep'].join('\n');

test('foldResponse keeps the first and final message and folds the work between', () => {
  const segs = segment([think, text('Starting.'), call('a', 'shell', { command: 'ls' }), text('Midway.'), call('b', 'read', { file_path: '/r/x.rs' }), text('Done.'), { kind: 'artifact', artifact: { id: 'x', kind: 'url', label: 'PR', path: null, url: 'https://x', mime: null, produced_at: null, turn_id: 't' } }]);
  const f = foldResponse(segs)!;
  assert.ok(f);
  assert.deepEqual(f.head.map((s) => (s.kind === 'block' && s.block.kind === 'text' ? s.block.md : s.kind)), ['Starting.']);
  assert.deepEqual(f.tail.map((s) => (s.kind === 'block' ? s.block.kind : s.kind)), ['text', 'artifact'], 'the final message and what follows it');
  assert.deepEqual(f.body.map((s) => (s.kind === 'block' && s.block.kind === 'text' ? s.block.md : s.kind)), ['steps', 'steps', 'Midway.', 'steps']);
  assert.deepEqual(foldStats(f.body), { steps: 2, failed: 0, thinking: 1 });
  // Nothing to fold: prose only, thinking only, or no message at all.
  assert.equal(foldResponse(segment([text('Just an answer.')])), null);
  assert.equal(foldResponse(segment([think, text('Thought, then answered.')])), null);
  assert.equal(foldResponse(segment([call('c', 'shell', { command: 'ls' })])), null);
  // Ended on a tool call: no final message yet — everything after the first folds.
  const open = foldResponse(segment([text('On it.'), call('d', 'shell', { command: 'x' }), text('Now testing.'), call('e', 'shell', { command: 'y' }, ok({ ok: false }))]))!;
  assert.equal(open.tail.length, 0);
  assert.equal(foldStats(open.body).failed, 1);
});

test('patchStats counts hunk lines only; changedFiles merges per path and treats a Write as new', () => {
  assert.deepEqual(patchStats(PATCH), { add: 2, del: 1 });
  assert.deepEqual(patchStats(''), { add: 0, del: 0 });
  const blocks: Block[] = [
    call('e1', 'edit', { file_path: '/r/src/a.rs' }, ok({ patch: PATCH, file_path: '/r/src/a.rs' })),
    call('e2', 'edit', { file_path: '/r/src/a.rs', old_string: 'x', new_string: 'y\nz' }),
    call('w1', 'write', { file_path: '/r/docs/n.md', content: '# N\n\nbody\n' }),
    call('bad', 'edit', { file_path: '/r/src/b.rs', old_string: 'a', new_string: 'b' }, ok({ ok: false })),
    call('r1', 'read', { file_path: '/r/src/c.rs' }),
  ];
  const files = changedFiles(blocks);
  assert.deepEqual(files.map((f) => [f.path, f.add, f.del, f.created]), [
    ['/r/src/a.rs', 4, 2, false],
    ['/r/docs/n.md', 3, 0, true],
  ]);
  assert.equal(files[1].content, '# N\n\nbody\n');
  const diff = changedDiff(files);
  assert.equal(diff.files.length, 2);
  assert.equal(diff.files[0].hunks.length, 2, 'both edits of one file fold into one entry');
  assert.equal(diff.files[1].hunks[0].lines.every((l) => l.origin === 'add'), true);
});

test('usage sums over a response; the token line names every kind with a tooltip', () => {
  const u = (i: number, o: number, th = 0): TurnUsage => ({ input_tokens: i, output_tokens: o, thinking_tokens: th, cache_read_tokens: 100, cache_creation_tokens: 0 });
  const turn = (id: string, role: 'user' | 'assistant', usage?: TurnUsage | null, blocks: Block[] = [text(id)]): Turn => ({
    id, role, ts: null, blocks, duration_ms: null, model: null, system: [], reasoning_steps: 0, usage,
  });
  const items = groupTurns([turn('q', 'user'), turn('a1', 'assistant', u(10, 5, 2)), turn('r', 'user', null, [call('x', 'shell', {})]), turn('a2', 'assistant', null), turn('a3', 'assistant', u(1, 1))]);
  assert.equal(items.length, 2);
  assert.deepEqual(items[1].usage, { input_tokens: 11, output_tokens: 6, thinking_tokens: 2, cache_read_tokens: 200, cache_creation_tokens: 0 });
  assert.equal(items[0].usage, null, 'your turn carries no usage');
  assert.equal(addUsage(null, null), null);
  const parts = usageParts(items[1].usage);
  assert.deepEqual(parts.map((p) => [p.key, p.value]), [['input', 11], ['thinking', 2], ['output', 4], ['cache_read', 200]], 'output = answer only; empty cache write dropped');
  assert.ok(parts.every((p) => p.title.length > 20));
  assert.deepEqual(usageParts(null), []);
});

test('day dividers and paths', () => {
  const now = new Date('2026-09-29T12:00:00');
  assert.equal(fmtDay('2026-09-29T08:00:00', now), 'Today');
  assert.equal(fmtDay('2026-09-28T23:30:00', now), 'Yesterday');
  assert.match(fmtDay('2026-09-20T10:00:00', now), /Sep/);
  assert.match(fmtDay('2025-01-02T10:00:00', now), /2025/);
  assert.equal(fmtDay(null, now), '');
  assert.equal(dayKey('2026-09-29T08:00:00'), '2026-09-29');
  assert.equal(dayKey('nope'), '');
  assert.equal(resolvePath('src/a.rs', '/repo'), '/repo/src/a.rs');
  assert.equal(resolvePath('./src/a.rs', '/repo/'), '/repo/src/a.rs');
  assert.equal(resolvePath('/abs/a.rs', '/repo'), '/abs/a.rs');
  assert.equal(resolvePath('src/a.rs', null), 'src/a.rs');
  assert.equal(relPath('/repo/src/a.rs', '/repo'), 'src/a.rs');
  assert.equal(relPath('/elsewhere/a.rs', '/repo'), '/elsewhere/a.rs');
});

test('file references: code spans, prose paths and locations', () => {
  assert.deepEqual(chat.splitLocation('src/a.rs:88:9'), { path: 'src/a.rs', line: 88, col: 9 });
  assert.deepEqual(chat.splitLocation('src/a.rs#L12'), { path: 'src/a.rs', line: 12, col: null });
  assert.deepEqual(chat.splitLocation('src/a.rs'), { path: 'src/a.rs', line: null, col: null });
  assert.deepEqual(chat.codeSpanFile('crates/otto-net/src/retry.rs:7'), { path: 'crates/otto-net/src/retry.rs', line: 7, col: null });
  assert.deepEqual(chat.codeSpanFile('retry.rs'), { path: 'retry.rs', line: null, col: null });
  assert.deepEqual(chat.codeSpanFile('Cargo.toml'), { path: 'Cargo.toml', line: null, col: null });
  for (const notAFile of ['v0.4.1', 'self.x', 'otto-net', 'a b.rs', 'std::env', 'x.min(max)', 'attempt = 12']) {
    assert.equal(chat.codeSpanFile(notAFile), null, notAFile);
  }
  assert.deepEqual(chat.githubRef('https://github.com/itzikiusa/otto/pull/7723'), { owner: 'itzikiusa', repo: 'otto', kind: 'pr', n: 7723 });
  assert.equal(chat.githubRef('https://github.com/itzikiusa/otto/issues/5#c')?.kind, 'issue');
  assert.equal(chat.githubRef('https://example.com/pull/1'), null);
});

test('the chat markdown extension: file refs, chips, and URLs left alone', () => {
  const m = new Marked({ gfm: true, breaks: true }).use(chat.chatMarkedExtension);
  const html = m.parse(
    'Fixed in crates/otto-net/src/retry.rs:7 and `src/lib.rs`. See https://github.com/a/b/pull/12 (#64), [docs](https://docs.rs/x/y.html), and/or v0.4.1.',
  ) as string;
  assert.match(html, /<a class="file-ref" data-path="crates\/otto-net\/src\/retry.rs" data-anchor="7"[^>]*>crates\/otto-net\/src\/retry.rs:7<\/a>/);
  assert.match(html, /<a class="file-ref code" data-path="src\/lib.rs"[^>]*><code>src\/lib.rs<\/code><\/a>/);
  assert.match(html, /<a class="ref-chip pr" href="https:\/\/github.com\/a\/b\/pull\/12"[^>]*>a\/b#12<\/a>/);
  assert.match(html, /<a class="ref-chip issue" data-raw="64"[^>]*>#64<\/a>/);
  assert.match(html, /<a href="https:\/\/docs.rs\/x\/y.html">docs<\/a>/, 'a labelled link stays a link; its path is not a file ref');
  assert.doesNotMatch(html, /data-path="docs.rs/);
  assert.match(html, /and\/or v0.4.1/, 'no false paths');
});

test('linkifyOutput links paths and URLs in escaped command output', () => {
  const out = chat.linkifyOutput("panicked at crates/otto-net/src/retry.rs:88:9:\nsee &lt;https://x.dev/a?b=1&amp;c=2&gt;.");
  assert.match(out, /<a class="file-ref" data-path="crates\/otto-net\/src\/retry.rs" data-anchor="88:9"/);
  assert.match(out, /<a class="out-link" href="https:\/\/x.dev\/a\?b=1&amp;c=2"[^>]*>https:\/\/x.dev\/a\?b=1&amp;c=2<\/a>&gt;\./);
  assert.equal(chat.linkifyOutput('<span>x</span>'), '<span>x</span>', 'markup is never re-linked');
});

test('fence info: language + file name', () => {
  assert.deepEqual(md.parseFenceInfo('rust src/retry.rs'), { tag: 'rust', file: 'src/retry.rs' });
  assert.deepEqual(md.parseFenceInfo('ts title="a/b.ts"'), { tag: 'ts', file: 'a/b.ts' });
  assert.deepEqual(md.parseFenceInfo('py:tool.py'), { tag: 'py', file: 'tool.py' });
  assert.deepEqual(md.parseFenceInfo('retry.rs'), { tag: 'rs', file: 'retry.rs' });
  assert.deepEqual(md.parseFenceInfo('bash'), { tag: 'bash', file: null });
  assert.deepEqual(md.parseFenceInfo(''), { tag: null, file: null });
});

test('highlightLines tokenizes the whole block and keeps every line self-contained', async () => {
  assert.deepEqual(splitHighlighted('<span class="c">/* a\nb */</span>\nx'), ['<span class="c">/* a</span>', '<span class="c">b */</span>', 'x']);
  assert.deepEqual(splitHighlighted('<span class="s">`<span class="t">${</span>\n}`</span>'), [
    '<span class="s">`<span class="t">${</span></span>',
    '<span class="s">}`</span>',
  ]);
  // Before hljs lands: escaped lines.
  assert.deepEqual(highlightLines('a < b\nc', 'typescript').length, 2);
  await ensureHljs();
  const lines = highlightLines('/* one\ntwo */\nconst x = 1;', 'typescript');
  assert.equal(lines.length, 3);
  assert.match(lines[1], /^<span class="hljs-comment">two \*\/<\/span>$/, 'the comment continues onto line 2');
  assert.match(lines[2], /hljs-keyword">const/);
  assert.equal(highlightLines('x', 'no-such-lang')[0], 'x');
});

test('preview parsers: kinds, CSV/TSV, JSON, HTML documents', () => {
  assert.equal(previewKind('/a/README.md'), 'markdown');
  assert.equal(previewKind('r.HTML'), 'html');
  assert.equal(previewKind('x.svg'), 'svg');
  assert.equal(previewKind('x.png'), 'image');
  assert.equal(previewKind('x.json'), 'json');
  assert.equal(previewKind('x.csv'), 'csv');
  assert.equal(previewKind('x.tsv'), 'tsv');
  assert.equal(previewKind('x.rs'), 'code');
  const t = parseDelimited('a,b,c\n1,"two, 2","say ""hi"""\r\n3,,"multi\nline"\n', ',');
  assert.deepEqual(t.rows, [['a', 'b', 'c'], ['1', 'two, 2', 'say "hi"'], ['3', '', 'multi\nline']]);
  assert.equal(t.cut, false);
  assert.deepEqual(parseDelimited('x\ty\n1\t2', '\t').rows, [['x', 'y'], ['1', '2']]);
  const many = parseDelimited(Array.from({ length: 50 }, (_, i) => String(i)).join('\n'), ',', 10);
  assert.equal(many.rows.length, 10);
  assert.equal(many.cut, true);
  assert.equal(prettyJson('{"a":[1,2]}'), '{\n  "a": [\n    1,\n    2\n  ]\n}');
  assert.equal(prettyJson('{nope'), null);
  assert.match(htmlDoc('<p>x</p>'), /^<!doctype html>.*<body><p>x<\/p><\/body><\/html>$/s);
  assert.equal(htmlDoc('<html><body>y</body></html>'), '<html><body>y</body></html>');
});
