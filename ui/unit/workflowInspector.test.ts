import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  connectionOptions, jiraAccountOptions, jsonValue, linesValue, listValue, loopStepSummarizer, paramList, paramNum, paramStr,
  storyOptions, toggled, withLoopStepParam, withLoopStepSummarizer, withParam, withScoring, withSummarizer,
} from '../src/modules/workflows/inspector/util.ts';
import { componentFunctions } from './componentFunctions.ts';

const node = (params: unknown) => ({ id: 'n', kind: 'review_run', name: '', x: 0, y: 0, params });

test('param readers and writers keep the forms’ empty-means-unset contract', () => {
  const n = node({ base: 'main', ms: '250', skills: ['a', 1, 'b'], summarizer: { provider: 'codex' } });
  assert.equal(paramStr(n, 'base'), 'main');
  assert.equal(paramStr(n, 'missing'), '');
  assert.equal(paramNum(n, 'ms', 0), 250);
  assert.equal(paramList(n, 'skills'), 'a, b');
  assert.deepEqual(withParam(n.params, 'base', ''), { ms: '250', skills: ['a', 1, 'b'], summarizer: { provider: 'codex' } });
  assert.equal(withParam(null, 'x', 1).x, 1);
  assert.deepEqual(jsonValue('{"a":1}'), { a: 1 });
  assert.equal(jsonValue('{"a":'), '{"a":', 'invalid JSON is kept as typed');
  assert.deepEqual(linesValue(' one \n\n two '), ['one', 'two']);
  assert.equal(listValue(' , '), undefined);
  assert.equal(withSummarizer(n, 'provider', ''), undefined, 'clearing the last field drops the summarizer');
  assert.deepEqual(withScoring(n, 'bug', 30), { bug: 30 });
  assert.deepEqual(toggled(['a', 'b'], 'a'), ['b']);
  const step = { kind: 'review_run', params: { providers: ['claude'], summarizer: { provider: 'codex', instructions: 'x' } } };
  assert.deepEqual(withLoopStepParam(step, 'providers', []), { summarizer: step.params.summarizer });
  assert.equal(loopStepSummarizer({ kind: 'review_run', params: withLoopStepSummarizer(step, 'claude') }), 'claude');
  assert.equal(withLoopStepSummarizer(step, ' ').summarizer, undefined);
});

test('pickers name what a person recognises, never only the id', () => {
  const conns = connectionOptions([{ id: 'c1', name: 'orders', kind: 'mysql', environment: 'prod' }] as never);
  assert.deepEqual(conns, [{ id: 'c1', label: 'orders · MySQL · Production' }], 'engine and environment in words, not wire enums');
  assert.equal(connectionOptions([{ id: 'c2', name: 'x', kind: 'mongodb', environment: null }] as never)?.[0].label, 'x · MongoDB · Development');
  assert.equal(connectionOptions(null), null, 'not loaded → null (the picker shows Loading…)');
  assert.equal(storyOptions([{ id: 's1', source_key: 'PROJ-7', title: 'Checkout', url: 'u' }] as never)?.[0].label, 'PROJ-7 — Checkout');
  const accounts = jiraAccountOptions([
    { id: 'a1', provider: 'jira', label: 'Work', email: 'me@x', base_url: 'https://x.atlassian.net' },
    { id: 'a2', provider: 'linear', label: 'Linear', email: '', base_url: '' },
  ] as never);
  assert.deepEqual(accounts?.map((a) => a.id), ['a1'], 'only Jira (Atlassian) accounts can publish or fetch tickets');
});

class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) { super(message); this.status = status; }
}
function finalOutput(read: () => Promise<{ content: string }>) {
  return componentFunctions(new URL('../src/modules/workflows/WorkflowsPage.svelte', import.meta.url), ['loadFinalOutput', 'renderFinalOutputSrcdoc'], {
    finalOutputRunId: null, finalOutputAvailable: false, finalOutputHtml: '', finalOutputError: null,
    run: { id: 'run' }, readRunContext: read, ApiError, loadErrorText: (e: Error) => e.message,
    renderMarkdownGfm: (md: string, opts: { breaks?: boolean }) => (opts?.breaks ? md.replace(/\n/g, '<br>') : md),
  });
}

test('workflow final output renders line breaks and hides silently only when there is none', async () => {
  const ok = finalOutput(async () => ({ content: 'Status: done\nFiles: 3' }));
  await ok.loadFinalOutput('run');
  assert.equal(ok.finalOutputAvailable, true);
  assert.equal(ok.finalOutputHtml, 'Status: done<br>Files: 3', 'agent reports keep their line-by-line breaks');

  const missing = finalOutput(async () => { throw new ApiError(404, 'not found'); });
  await missing.loadFinalOutput('run');
  assert.equal(missing.finalOutputError, null);
  assert.equal(missing.finalOutputRunId, 'run', 'a run without a deliverable is not refetched');

  const denied = finalOutput(async () => { throw new ApiError(403, 'forbidden'); });
  await denied.loadFinalOutput('run');
  assert.equal(JSON.stringify(denied.finalOutputError), JSON.stringify({ runId: 'run', text: 'forbidden' }));
  assert.equal(denied.finalOutputRunId, null, 'a failed read is forgotten so Retry / the next update can try again');
});
