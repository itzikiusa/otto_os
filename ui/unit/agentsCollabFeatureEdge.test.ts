import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { Turn } from '../src/lib/api/types.ts';
import {
  formatTranscriptMarkdown,
  stripAnsi,
} from '../src/modules/agents/conversation/exportMarkdownFormat.ts';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

function makeTurn(partial: Partial<Turn> & Pick<Turn, 'role' | 'blocks'>): Turn {
  return {
    id: partial.id ?? 't1',
    role: partial.role,
    ts: partial.ts ?? null,
    blocks: partial.blocks,
    duration_ms: null,
    model: null,
    system: [],
    reasoning_steps: 0,
  };
}

test('stripAnsi removes CSI, OSC, and stray CR sequences cleanly', () => {
  const dirty = '\x1b[32mPASS\x1b[0m \x1b]0;window title\x07line1\r\nline2\r';
  assert.equal(stripAnsi(dirty), 'PASS line1\nline2\n');
});

test('formatTranscriptMarkdown formats prompts, assistant replies, tool summaries, and skips empty turns', () => {
  const turns: Turn[] = [
    makeTurn({
      id: 'u1',
      role: 'user',
      blocks: [{ kind: 'text', md: 'Fix the failing \x1b[31mauth\x1b[0m test' }],
    }),
    makeTurn({
      id: 'empty',
      role: 'assistant',
      blocks: [{ kind: 'thinking', count: 1 }],
    }),
    makeTurn({
      id: 'a1',
      role: 'assistant',
      blocks: [
        {
          kind: 'tool_call',
          id: 'tc1',
          name: 'Bash',
          tool: 'shell',
          title: 'Run tests',
          input: { command: 'cargo test auth' },
          result: { ok: true, text: 'huge raw stdout should not be dumped', truncated: false, bytes: 36, image_ids: [], patch: null, file_path: null },
        },
        {
          kind: 'tool_call',
          id: 'tc2',
          name: 'Edit',
          tool: 'edit',
          title: 'Edit auth.rs',
          input: { file_path: 'src/auth.rs' },
          result: { ok: false, text: 'conflict', truncated: false, bytes: 8, image_ids: [], patch: null, file_path: 'src/auth.rs' },
        },
        { kind: 'subagent', agent_id: 'sub-1', agent_type: 'explore', description: 'Verify token expiry', status: 'done' },
        { kind: 'text', md: 'Fixed token TTL check in `src/auth.rs`.' },
      ],
    }),
  ];

  const md = formatTranscriptMarkdown({
    title: 'Auth bugfix',
    provider: 'claude',
    model: 'claude-sonnet-4',
    cwd: '/repo/app',
    turns,
  });

  assert.ok(md.startsWith('# Auth bugfix\n\n_Provider: Claude · Model: `claude-sonnet-4` · Folder: `/repo/app`_'));
  assert.ok(md.includes('## You\n\nFix the failing auth test'));
  assert.ok(md.includes('## Claude'));
  assert.ok(md.includes('`cargo test auth`'));
  assert.ok(md.includes('_(failed)_'));
  assert.ok(md.includes('- **Subagent:** Verify token expiry'));
  assert.ok(md.includes('Fixed token TTL check in `src/auth.rs`.'));
  assert.ok(!md.includes('huge raw stdout'), 'raw tool stdout must not bloat Markdown export');
  assert.ok(!md.includes('\x1b['), 'no raw ANSI escape sequences allowed');
});

test('formatTranscriptMarkdown returns empty string when there is no readable content', () => {
  assert.equal(formatTranscriptMarkdown({ turns: [] }), '');
  assert.equal(
    formatTranscriptMarkdown({
      turns: [makeTurn({ role: 'assistant', blocks: [{ kind: 'thinking', count: 2 }] })],
    }),
    '',
  );
});

test('PromoteDiffPreview.fetchCurrent handles found, 404 new skill, error, and stale race', async () => {
  class FakeApiError extends Error {
    status: number;
    constructor(status: number, msg: string) {
      super(msg);
      this.status = status;
    }
  }
  const first = deferred<{ content: string }>();
  const second = deferred<{ content: string }>();
  let calls = 0;

  const state = componentFunctions(
    new URL('../src/modules/skills-eval/PromoteDiffPreview.svelte', import.meta.url),
    ['fetchCurrent'],
    {
      seq: 0,
      disposed: false,
      lookup: { kind: 'idle' } as { kind: string; content?: string; message?: string },
      ApiError: FakeApiError,
      loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)),
      skillLabApi: {
        getFile: (name: string) => {
          calls++;
          if (name === 'skill-a') return first.promise;
          if (name === 'skill-b') return second.promise;
          if (name === 'missing') return Promise.reject(new FakeApiError(404, 'Not found'));
          return Promise.reject(new Error('network down'));
        },
      },
    },
  );

  // Stale race: skill-a resolves AFTER skill-b
  const pA = state.fetchCurrent('skill-a');
  const pB = state.fetchCurrent('skill-b');
  second.resolve({ content: '# Skill B installed' });
  await pB;
  first.resolve({ content: '# Skill A stale' });
  await pA;
  assert.equal(calls, 2);
  assert.equal(state.lookup.kind, 'found');
  assert.equal(state.lookup.content, '# Skill B installed');

  // 404 => new skill
  await state.fetchCurrent('missing');
  assert.equal(state.lookup.kind, 'new');

  // Non-404 error => error state
  await state.fetchCurrent('broken');
  assert.equal(state.lookup.kind, 'error');
  assert.equal(state.lookup.message, 'network down');
});
