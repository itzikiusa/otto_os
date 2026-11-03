import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { Turn } from '../src/lib/api/types';
import { buildSideChatPrompt, summarizeTurn } from '../src/modules/threads/sideChatPrompt.ts';

const turn = (role: 'user' | 'assistant', blocks: Turn['blocks']): Turn =>
  ({ id: `${role}-${Math.random()}`, role, ts: null, blocks, duration_ms: null, model: null, system: [], reasoning_steps: 0 }) as Turn;

describe('side chat prompt', () => {
  it('summarizes prose and tool titles, skipping empty turns', () => {
    const t = turn('assistant', [
      { kind: 'text', md: 'Fixed the   parser.' },
      { kind: 'tool_call', id: '1', name: 'Edit', tool: 'edit' as never, title: 'Edit src/a.ts', input: {}, result: null },
    ]);
    assert.equal(summarizeTurn(t), 'Assistant: Fixed the parser. [tools: Edit src/a.ts]');
    assert.equal(summarizeTurn(turn('user', [{ kind: 'thinking', count: 1 }])), null);
  });

  it('carries the main thread snapshot and the question, newest turns last', () => {
    const turns = Array.from({ length: 12 }, (_, i) => turn(i % 2 ? 'assistant' : 'user', [{ kind: 'text', md: `turn ${i}` }]));
    const p = buildSideChatPrompt('  why does CI fail? ', {
      title: 'Fix auth',
      harness: 'Claude Code',
      cwd: '/repo',
      branch: 'feat/auth',
      changedFiles: ['a.ts', 'b.ts'],
      turns,
    });
    assert.match(p, /Main thread: "Fix auth" \(Claude Code\) in \/repo on branch feat\/auth\./);
    assert.match(p, /Uncommitted changes: a\.ts, b\.ts\./);
    assert.match(p, /Do not edit files/);
    assert.equal(/: turn 3$/m.test(p), false);
    assert.equal(p.includes('User: turn 4'), true);
    assert.ok(p.indexOf('turn 11') > p.indexOf('turn 4'));
    assert.ok(p.endsWith('Question: why does CI fail?'));
  });
});
