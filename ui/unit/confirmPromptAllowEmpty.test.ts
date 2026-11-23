import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function loadConfirm() {
  return loadSource(new URL('../src/lib/confirm.svelte.ts', import.meta.url), {});
}

test('promptText default resolves empty submit to null', async () => {
  const { confirmer } = loadConfirm();
  const p = confirmer.promptText('Folder', { initial: '   ' });
  confirmer.submit();
  assert.equal(await p, null);
});

test('promptText with allowEmpty: true resolves empty submit to "" and dismiss to null', async () => {
  const { confirmer } = loadConfirm();

  const submittedEmpty = confirmer.promptText('Folder', { initial: '   ', allowEmpty: true });
  confirmer.submit();
  assert.equal(await submittedEmpty, '');

  const cancelled = confirmer.promptText('Folder', { initial: 'docs', allowEmpty: true });
  confirmer.dismiss();
  assert.equal(await cancelled, null);

  // Subsequent default prompt does not inherit allowEmpty from the previous call.
  const nextDefault = confirmer.promptText('Name', { initial: '' });
  confirmer.submit();
  assert.equal(await nextDefault, null);
});

test('promptText with allowEmpty: true still runs validate on empty value', async () => {
  const { confirmer } = loadConfirm();
  let calls = 0;
  const p = confirmer.promptText('Tag', {
    initial: '',
    allowEmpty: true,
    validate: (v: string) => {
      calls++;
      return v === '' && calls === 1 ? 'Confirm clearing' : null;
    },
  });
  confirmer.submit();
  assert.equal(confirmer.open, true);
  assert.equal(confirmer.inputError, 'Confirm clearing');
  confirmer.submit();
  assert.equal(await p, '');
  assert.equal(confirmer.open, false);
});
