import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  isImmediateSlashCommand,
  mergeSlashCommands,
  parseInteractiveSlashInput,
} from '../src/lib/slashActions.ts';

describe('slashActions', () => {
  it('merges interactive commands ahead of server builtins and keeps user/project skills', () => {
    const merged = mergeSlashCommands([
      { name: 'model', description: 'Switch the model', source: 'builtin' },
      { name: 'vim', description: 'Toggle vim mode', source: 'builtin' },
      { name: 'commit', description: 'Write a commit', source: 'project' },
    ]);
    const model = merged.find((c) => c.name === 'model');
    assert.equal(model?.source, 'interactive');
    assert.equal(merged.some((c) => c.name === 'vim'), false);
    assert.equal(merged.some((c) => c.name === 'commit' && c.source === 'project'), true);
    assert.equal(isImmediateSlashCommand('model'), true);
    assert.equal(isImmediateSlashCommand('commit'), false);
  });

  it('parses interactive slash intents with and without arguments', () => {
    assert.deepEqual(parseInteractiveSlashInput('/model'), { kind: 'open-model-picker' });
    assert.deepEqual(parseInteractiveSlashInput('/model claude-opus-5-5'), {
      kind: 'set-model',
      model: 'claude-opus-5-5',
    });
    assert.deepEqual(parseInteractiveSlashInput('/effort xhigh'), {
      kind: 'set-effort',
      effort: 'xhigh',
    });
    assert.deepEqual(parseInteractiveSlashInput('/plan Refactor auth flow'), {
      kind: 'set-mode',
      mode: 'plan',
      prompt: 'Refactor auth flow',
    });
    assert.deepEqual(parseInteractiveSlashInput('/status'), { kind: 'toggle-status' });
    assert.deepEqual(parseInteractiveSlashInput('/diff'), { kind: 'open-diff' });
    assert.deepEqual(parseInteractiveSlashInput('/fork'), { kind: 'new-thread', beside: true });
    assert.equal(parseInteractiveSlashInput('/commit fix tests'), null);
  });
});

describe('slashActions /btw side chat', () => {
  it('parses /btw with its question', () => {
    assert.deepEqual(parseInteractiveSlashInput('/btw why is the build slow?'), {
      kind: 'side-chat',
      question: 'why is the build slow?',
    });
    assert.deepEqual(parseInteractiveSlashInput('/btw'), { kind: 'side-chat', question: '' });
    assert.equal(isImmediateSlashCommand('btw'), false);
  });

  it('lists /btw only when the host does not omit it, and then keeps a CLI command of that name', () => {
    assert.equal(mergeSlashCommands(null).some((c) => c.name === 'btw' && c.source === 'interactive'), true);
    const omitted = mergeSlashCommands([{ name: 'btw', description: 'CLI side question', source: 'builtin' }], {
      omitInteractive: new Set(['btw']),
    });
    assert.equal(omitted.find((c) => c.name === 'btw')?.source, 'builtin');
  });
});
