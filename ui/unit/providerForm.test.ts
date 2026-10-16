import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  formToProvider,
  joinArgs,
  providerToForm,
  splitArgs,
} from '../src/modules/settings/providerForm.ts';

// Settings → Agents → Providers: Edit… then Save provider must write back the
// entry it opened. A whitespace join/split split any arg with a space into
// several argv elements and dropped keys the sheet doesn't show.

test('Edit → Save round-trips every field, including args with spaces and quotes', () => {
  const def = {
    cmd: 'opencode',
    args: ['--session', '{sid}', '--system', 'be terse', "it's", ''],
    resume_args: ['--resume', '{sid}'],
    update_command: 'npm i -g opencode',
    model_args: ['--model', '{model}'],
  };
  const form = providerToForm(def);
  assert.equal(form.args, `--session {sid} --system 'be terse' 'it'\\''s' ''`);
  assert.equal(form.resumeArgs, '--resume {sid}');
  assert.equal(form.updateCmd, 'npm i -g opencode');
  assert.equal(form.modelArgs, '--model {model}');
  assert.deepEqual(formToProvider(form, def), def);
});

test('unset optional fields stay unset; clearing a field removes its key', () => {
  const bare = { cmd: 'kilo', args: [] };
  assert.deepEqual(formToProvider(providerToForm(bare), bare), bare);

  const full = { cmd: 'grok', args: ['-x'], resume_args: ['-r'], update_command: 'up', model_args: ['-m', '{model}'] };
  const cleared = { ...providerToForm(full), resumeArgs: '  ', updateCmd: '', modelArgs: '' };
  assert.deepEqual(formToProvider(cleared, full), { cmd: 'grok', args: ['-x'] });
});

test('keys the sheet does not edit survive a save', () => {
  const prev = { cmd: 'x', args: [], extra: { keep: true } } as unknown as Parameters<typeof formToProvider>[1];
  const out = formToProvider({ cmd: ' y ', args: '', resumeArgs: '', updateCmd: '', modelArgs: '' }, prev);
  assert.deepEqual(out, { cmd: 'y', args: [], extra: { keep: true } });
});

test('typed quotes group words and never reach the CLI literally', () => {
  assert.deepEqual(splitArgs(`--system "be terse" -p 'a b'  c\\ d`), ['--system', 'be terse', '-p', 'a b', 'c d']);
  assert.deepEqual(splitArgs(`"say \\"hi\\""`), ['say "hi"']);
  assert.deepEqual(splitArgs(`'' ""`), ['', '']);
  assert.deepEqual(splitArgs('   '), []);
  assert.equal(joinArgs(null), '');
  for (const list of [['a b', `q'q`, 'back\\slash', '"dq"', '{cwd}'], ['']]) {
    assert.deepEqual(splitArgs(joinArgs(list)), list);
  }
});
