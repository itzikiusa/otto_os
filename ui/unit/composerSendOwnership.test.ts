import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { TranscriptLifecycle } from '../src/lib/stores/transcriptLifecycle.ts';
import * as paneHeader from '../src/lib/paneHeader.ts';

test('a reopened composer shares the pending send claim while other sessions remain usable', () => {
  const { transcript } = loadSource(new URL('../src/lib/stores/transcript.svelte.ts', import.meta.url), {
    './transcriptLifecycle': { TranscriptLifecycle },
    '../paneHeader': paneHeader,
    '../win': { winKey: (key: string) => key },
    '../api/client': { api: {}, isAbortError: () => false },
  });
  assert.equal(transcript.tryBeginSend('A'), true);
  assert.equal(transcript.sending('A'), true);
  assert.equal(transcript.tryBeginSend('A'), false, 'a second mounted composer cannot submit the same pending draft');
  assert.equal(transcript.tryBeginSend('B'), true);
  transcript.finishSend('A');
  assert.equal(transcript.sending('A'), false);
  assert.equal(transcript.sending('B'), true);
  assert.equal(transcript.tryBeginSend('A'), true, 'success or failure releases the source session for another send');
});

test('production send keeps a remounted draft and allows only one pending request', async () => {
  const { readFileSync } = await import('node:fs');
  const ts = (await import('typescript')).default;
  const { deferred } = await import('./sourceHarness.ts');
  const { transcript } = loadSource(new URL('../src/lib/stores/transcript.svelte.ts', import.meta.url), {
    './transcriptLifecycle': { TranscriptLifecycle },
    '../paneHeader': paneHeader,
    '../win': { winKey: (key: string) => key }, '../api/client': { api: {} },
  });
  const source = readFileSync(new URL('../src/modules/agents/conversation/Composer.svelte', import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('composer.ts', source, ts.ScriptTarget.Latest, true);
  const methods = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && ['send', 'restoreComposerFocus'].includes(node.name?.text ?? ''));
  const productionMethods = methods.map((node) => source.slice(node.getStart(parsed), node.end)).join('\n');
  const pending = deferred<void>();
  let calls = 0;
  function mount() {
    const compiled = ts.transpileModule(`
      const ownerId = 'A', text = transcript.draft(ownerId), attachments = [];
      const document = { activeElement: null, body: {} }, composerEl = null;
      let sending = false; const ta = null; let uploading = 0;
      ${productionMethods}
      return send;
    `, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
    return new Function('transcript', 'submitPrompt', 'queueMicrotask', 'autosize', 'toasts', compiled)(
      transcript, () => { calls++; return pending.promise; }, () => {}, () => {}, { error() {} });
  }
  transcript.setDraft('A', 'first');
  const first = mount()();
  const reopened = mount()();
  await Promise.resolve();
  assert.equal(calls, 1);
  transcript.setDraft('A', 'new draft while waiting');
  pending.resolve(); await Promise.all([first, reopened]);
  assert.equal(transcript.draft('A'), 'new draft while waiting');
  assert.equal(transcript.sending('A'), false);
});

test('send is refused while an image upload is still pending', async () => {
  const { readFileSync } = await import('node:fs');
  const ts = (await import('typescript')).default;
  const { transcript } = loadSource(new URL('../src/lib/stores/transcript.svelte.ts', import.meta.url), {
    './transcriptLifecycle': { TranscriptLifecycle },
    '../paneHeader': paneHeader,
    '../win': { winKey: (key: string) => key }, '../api/client': { api: {} },
  });
  const source = readFileSync(new URL('../src/modules/agents/conversation/Composer.svelte', import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const parsed = ts.createSourceFile('composer.ts', source, ts.ScriptTarget.Latest, true);
  const methods = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && ['send', 'restoreComposerFocus'].includes(node.name?.text ?? ''));
  const productionMethods = methods.map((node) => source.slice(node.getStart(parsed), node.end)).join('\n');
  let calls = 0;
  const compiled = ts.transpileModule(`
    const ownerId = 'A', text = transcript.draft(ownerId), attachments = [];
      const document = { activeElement: null, body: {} }, composerEl = null;
    let sending = false; const ta = null; let uploading = 1;
    ${productionMethods}
    return send;
  `, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  transcript.setDraft('A', 'with a screenshot');
  await new Function('transcript', 'submitPrompt', 'queueMicrotask', 'autosize', 'toasts', compiled)(
    transcript, () => { calls++; return Promise.resolve(); }, () => {}, () => {}, { error() {} })();
  assert.equal(calls, 0);
  assert.equal(transcript.draft('A'), 'with a screenshot');
});
