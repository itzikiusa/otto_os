// Chat redesign (docs/design/conversation-view.md §5.2, rev 5): the pure
// pieces behind the new presentation — one-line tool rows, step-group
// summaries, the live "doing now" call, the jump pill's unread count, agent
// questions, the plan's placement, and the code-block chrome added to the
// cached markdown HTML.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  askQuestions,
  countUnread,
  lastLine,
  pendingTool,
  providerName,
  segment,
  stepSummary,
  toolLine,
  toolStatus,
  type RenderItem,
  type StepBlock,
  type ToolCallBlock,
} from '../src/modules/agents/conversation/format.ts';
import { CODE_CAP_LINES, decorateCodeBlocks } from '../src/modules/agents/conversation/codeBlocks.ts';
import type { Block, ToolKind, ToolResult } from '../src/lib/api/types.ts';

const ok = (text = 'ok', extra: Partial<ToolResult> = {}): ToolResult => ({
  ok: true, text, truncated: false, bytes: text.length, image_ids: [], patch: null, file_path: null, ...extra,
});
const call = (id: string, tool: ToolKind, input: unknown, result: ToolResult | null = ok(), name = 'Tool', title = ''): ToolCallBlock => ({
  kind: 'tool_call', id, name, tool, title, input, result,
});

test('toolLine: commands, files and searches read as verb + target (+ dim detail)', () => {
  const bash = toolLine(call('1', 'shell', { command: 'cargo test -p otto-net\n--nocapture', description: 'Run the tests' }));
  assert.deepEqual([bash.verb, bash.present, bash.base, bash.target, bash.mono, bash.hint], ['Ran', 'Running', 'run', 'cargo test -p otto-net …', true, 'Run the tests']);
  const edit = toolLine(call('2', 'edit', { file_path: '/repo/otto/crates/net/src/retry.rs' }));
  assert.deepEqual([edit.verb, edit.present, edit.target, edit.detail], ['Edited', 'Editing', 'retry.rs', '/repo/otto/crates/net/src']);
  const grep = toolLine(call('3', 'search', { pattern: 'fn jitter', path: 'crates/net' }));
  assert.deepEqual([grep.verb, grep.target, grep.detail], ['Searched', 'fn jitter', 'crates/net']);
  // Codex passes the command as argv; an unknown input falls back to the title.
  assert.equal(toolLine(call('4', 'shell', { command: ['git', 'status'] })).target, 'git status');
  assert.equal(toolLine(call('5', 'mcp', {}, ok(), 'mcp__github__get_pr', 'github · get_pr')).target, 'github · get_pr');
  assert.equal(toolLine(call('6', 'other', null, ok(), 'Weird', '')).target, 'Weird');
});

test('toolStatus: no result is running while the session is active, else "none"', () => {
  assert.equal(toolStatus(call('1', 'shell', {}, null), true), 'running');
  assert.equal(toolStatus(call('1', 'shell', {}, null), false), 'none');
  assert.equal(toolStatus(call('1', 'shell', {}, ok()), false), 'ok');
  assert.equal(toolStatus(call('1', 'shell', {}, { ...ok(), ok: false }), true), 'err');
});

test('stepSummary says what a group did, in order, naming a single file', () => {
  const steps: StepBlock[] = [
    call('1', 'shell', { command: 'a' }),
    call('2', 'read', { file_path: '/r/a.rs' }),
    call('3', 'read', { file_path: '/r/b.rs' }),
    call('4', 'edit', { file_path: '/r/a.rs' }),
    call('5', 'edit', { file_path: '/r/a.rs' }),
    call('6', 'shell', { command: 'b' }),
    { kind: 'thinking', count: 2 },
  ];
  assert.equal(stepSummary(steps), 'Ran 2 commands, read 2 files, edited a.rs');
  assert.equal(stepSummary([call('1', 'search', { pattern: 'x' })]), 'Searched the code');
  assert.equal(stepSummary([call('1', 'search', { pattern: 'x' }), call('2', 'search', { pattern: 'y' })]), 'Ran 2 searches');
  assert.equal(stepSummary([{ kind: 'thinking', count: 1 }]), 'Thought');
  const many: StepBlock[] = (['shell', 'read', 'edit', 'search', 'web', 'mcp'] as ToolKind[]).map((k, i) => call(String(i), k, { file_path: `/f${i}` }));
  assert.equal(stepSummary(many), 'Ran a command, read f1, edited f2, and 3 more');
  assert.equal(stepSummary([{ kind: 'subagent', agent_id: 'a', description: 'Explore', agent_type: 'Explore', status: 'done' }]), 'Delegated to an agent');
});

test('pendingTool: the newest call without a result in the last response', () => {
  const item = (blocks: Block[], role: 'user' | 'assistant' = 'assistant'): RenderItem => ({
    id: 'i', role, turns: [], blocks, system: [], duration_ms: null, ts: null, model: null, reasoning_steps: 0,
  });
  const done = call('1', 'shell', {});
  const open = call('2', 'shell', { command: 'sleep 1' }, null);
  assert.equal(pendingTool(item([done, open, { kind: 'text', md: 'x' }]))?.id, '2');
  assert.equal(pendingTool(item([done])), null);
  assert.equal(pendingTool(item([open], 'user')), null);
  assert.equal(pendingTool(undefined), null);
});

test('lastLine: the last non-blank output line, clipped', () => {
  assert.equal(lastLine('a\nb\n\n  \n'), 'b');
  assert.equal(lastLine('only'), 'only');
  assert.equal(lastLine(''), '');
  assert.equal(lastLine(null), '');
  assert.equal(lastLine(`x\n${'y'.repeat(300)}`, 10), `${'y'.repeat(9)}…`);
});

test('countUnread counts items after the last one seen at the bottom', () => {
  const ids = ['a', 'b', 'c', 'd'];
  assert.equal(countUnread(ids, 'b'), 2);
  assert.equal(countUnread(ids, 'd'), 0);
  assert.equal(countUnread(ids, null), 0);
  assert.equal(countUnread(ids, 'gone'), 0, 'an anchor paged out of the window counts nothing');
});

test('askQuestions reads AskUserQuestion input defensively', () => {
  assert.deepEqual(
    askQuestions({ questions: [{ question: 'Which DB?', options: [{ label: 'Postgres' }, { label: 'SQLite' }, { nope: 1 }] }, { question: '' }, 7] }),
    [{ question: 'Which DB?', options: ['Postgres', 'SQLite'] }],
  );
  assert.deepEqual(askQuestions({ questions: [{ question: 'Go?', options: ['yes', 'no'] }] }), [{ question: 'Go?', options: ['yes', 'no'] }]);
  assert.deepEqual(askQuestions(null), []);
  assert.deepEqual(askQuestions({ questions: 'x' }), []);
});

test('providerName', () => {
  assert.equal(providerName('claude'), 'Claude');
  assert.equal(providerName('codex'), 'Codex');
  assert.equal(providerName('grok'), 'Grok');
  assert.equal(providerName(null), 'Agent');
});

test('segment: the plan is its own block; the call that wrote it gets no row', () => {
  const todo = call('t', 'task', { todos: [] }, ok(), 'TodoWrite', 'Update todos');
  const tasks: Block = { kind: 'tasks', tasks: [{ ext_id: null, title: 'x', status: 'pending', active_form: null }] };
  const bash = call('b', 'shell', { command: 'ls' });
  const segs = segment([todo, tasks, bash]);
  assert.deepEqual(segs.map((s) => (s.kind === 'steps' ? `steps:${s.steps.length}` : s.block.kind)), ['tasks', 'steps:1']);
  // A failed plan write keeps its row (the error is worth seeing).
  const failed = call('t2', 'task', {}, { ...ok(), ok: false });
  assert.deepEqual(segment([failed, tasks]).map((s) => s.kind), ['steps', 'block']);
});

test('decorateCodeBlocks adds label + Wrap/Copy, caps long blocks, leaves prose alone', () => {
  assert.equal(decorateCodeBlocks('<p>hi</p>'), '<p>hi</p>');
  const short = decorateCodeBlocks('<p>a</p><pre><code class="hljs language-rust">let x = 1;\nlet y = 2;</code></pre>');
  assert.match(short, /<div class="code-block" data-lines="2">/);
  assert.match(short, /<span class="code-lang">rust<\/span>/);
  assert.match(short, /data-code-act="copy"/);
  assert.match(short, /data-code-act="wrap" aria-pressed="false"/);
  assert.match(short, /<pre><code class="hljs language-rust">let x = 1;\nlet y = 2;<\/code><\/pre>/);
  assert.doesNotMatch(short, /data-code-act="expand"/);
  const body = Array.from({ length: CODE_CAP_LINES + 5 }, (_, i) => `l${i}`).join('\n');
  const long = decorateCodeBlocks(`<pre><code class="hljs">${body}</code></pre><pre><code class="hljs">x</code></pre>`);
  assert.match(long, /class="code-block capped" data-lines="23"/);
  assert.match(long, /Show all 23 lines/);
  assert.match(long, /<span class="code-lang"><\/span>/, 'no language → empty label');
  assert.equal(long.match(/class="code-block/g)?.length, 2, 'every block is wrapped');
  // Idempotent on its own output (a cache re-decorate can't nest toolbars).
  assert.equal(decorateCodeBlocks(long).match(/class="code-block/g)?.length, 2);
});
