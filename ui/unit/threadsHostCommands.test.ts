// The embeddable runtime conversation's contract (lib/runtimeThreadPort.ts):
// the slash popup offers only the view commands the host can carry out, a
// command without a handler falls through to the agent, and an embed never
// takes focus unless the host asks.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { availableSlash, runHostSlash, VIEW_SLASH } from '../src/modules/threads/runtime/hostCommands.ts';

const cmd = (name: string) => ({ name, description: name, source: 'interactive' as const });
const ALL = ['model', 'diff', 'files', 'terminal', 'new', 'clear', 'fork', 'review', 'btw'].map(cmd);
const names = (list: { name: string }[]) => list.map((c) => c.name);

test('no host commands: only the thread’s own commands are offered', () => {
  assert.deepEqual(names(availableSlash(ALL, undefined)), ['clear', 'review']);
  assert.deepEqual(names(availableSlash(ALL, {})), ['clear', 'review']);
});

test('each host handler adds exactly its commands, in the popup’s order', () => {
  const noop = () => {};
  assert.deepEqual(names(availableSlash(ALL, { openDiff: noop })), ['diff', 'clear', 'review']);
  assert.deepEqual(names(availableSlash(ALL, { newThread: noop })), ['new', 'clear', 'fork', 'review']);
  assert.deepEqual(
    names(availableSlash(ALL, { openDiff: noop, openFiles: noop, openTerminal: noop, newThread: noop, sideChat: noop })),
    ['diff', 'files', 'terminal', 'new', 'clear', 'fork', 'review', 'btw'],
  );
  // A command that is not a view command (the model picker) is never offered here.
  assert.equal(Object.prototype.hasOwnProperty.call(VIEW_SLASH, 'model'), false);
});

test('a handled intent goes to the host with the thread id; an unhandled one is not consumed', () => {
  const calls: unknown[][] = [];
  const host = {
    openDiff: (id: string) => calls.push(['diff', id]),
    newThread: (id: string, beside: boolean) => calls.push(['new', id, beside]),
    sideChat: (id: string, q: string) => calls.push(['btw', id, q]),
  };
  assert.equal(runHostSlash({ kind: 'open-diff' }, 'T1', host), true);
  assert.equal(runHostSlash({ kind: 'new-thread', beside: true }, 'T1', host), true);
  assert.equal(runHostSlash({ kind: 'side-chat', question: 'why?' }, 'T1', host), true);
  assert.deepEqual(calls, [['diff', 'T1'], ['new', 'T1', true], ['btw', 'T1', 'why?']]);
  // No handler: false, so the composer sends "/files" to the agent as typed.
  assert.equal(runHostSlash({ kind: 'open-files' }, 'T1', host), false);
  assert.equal(runHostSlash({ kind: 'toggle-terminal' }, 'T1', undefined), false);
  assert.equal(runHostSlash({ kind: 'send-prompt', prompt: 'x' }, 'T1', host), false);
});

test('the embed contract defaults: no focus, plain chrome; detach defaults to Threads', () => {
  const went: string[] = [];
  const port = loadSource(new URL('../src/lib/runtimeThreadPort.ts', import.meta.url), {
    './router.svelte': { router: { go: (r: string) => went.push(r) } },
  });
  assert.deepEqual({ ...port.EMBED_DEFAULTS }, { focusOnMount: false, compact: false });
  port.openInThreads('T9');
  assert.deepEqual(went, ['threads/T9']);
});
