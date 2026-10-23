// W2P7 (architecture-5-04): the Jira issue an Overview tab shows, lifted out
// of OverviewTab into product/jiraIssue.svelte.ts (JiraIssueSection renders
// it). Driven through the real class with fake issue routes: a response for a
// story the tab has left never lands, and dev status loads once, stays loaded
// after a failure (no retry loop) and refetches on Refresh.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function jiraIssue(routes: { issueFull?: (acc: string, key: string) => Promise<any>; devStatus?: (acc: string, key: string, id?: string) => Promise<any> }) {
  const editorResets: number[] = [];
  const mod = loadSource(new URL('../src/modules/product/jiraIssue.svelte.ts', import.meta.url), {
    '../../lib/api/issues': routes,
    './jira/issueEditorState.svelte': { JiraIssueEditor: class { host: unknown; constructor(h: unknown) { this.host = h; } reset() { editorResets.push(1); } } },
  });
  let story: any = { id: 'A', account_id: 'acc', source_key: 'GS-1', title: 'A' };
  let current = 'A';
  const ji = new mod.JiraIssue({ story: () => story, isCurrent: (s: { id: string }) => s.id === current, afterTransition: async () => {} });
  return { ji, editorResets, switchTo(id: string) { story = { id, account_id: 'acc', source_key: `GS-${id}`, title: id }; current = id; } };
}

test('a full issue that lands after the tab moved to another story is dropped', async () => {
  const a = deferred<any>();
  const { ji, switchTo, editorResets } = jiraIssue({ issueFull: () => a.promise });
  const pending = ji.load();
  assert.equal(ji.loading, true);
  switchTo('B');
  ji.reset();
  assert.equal(editorResets.length, 1, 'the field editor resets with the issue');
  a.resolve({ id: '10', summary: 'A’s issue' });
  await pending;
  assert.equal(ji.full, null, 'A’s issue never shows under B');
  assert.equal(ji.loading, false, 'B is free to start its own load');
});

test('a failed issue load says why and stays failed until Retry', async () => {
  let fail = true;
  const { ji } = jiraIssue({ issueFull: async () => { if (fail) throw new Error('Jira is down'); return { id: '10' }; } });
  await ji.load();
  assert.match(ji.error, /Jira is down/);
  assert.equal(ji.full, null);
  fail = false;
  await ji.load();
  assert.equal(ji.error, null);
  assert.deepEqual({ ...ji.full }, { id: '10' });
});

test('dev status loads once, a failure does not re-trigger, and Refresh refetches', async () => {
  const calls: (string | undefined)[] = [];
  let fail = true;
  const { ji } = jiraIssue({
    issueFull: async () => ({ id: '10' }),
    devStatus: async (_a, _k, id) => { calls.push(id); if (fail) throw new Error('no dev info'); return { branches: [], commits: [], pull_requests: [] }; },
  });
  await ji.load();
  await ji.loadDev();
  assert.match(ji.devError, /no dev info/);
  assert.equal(ji.devLoaded, true, 'marked loaded even on error — the open effect would loop otherwise');
  await ji.loadDev();
  assert.equal(calls.length, 1, 'no second fetch until Refresh');
  fail = false;
  await ji.reloadDev();
  assert.equal(calls.length, 2);
  assert.deepEqual(calls, ['10', '10'], 'dev status is asked for the loaded issue id');
  assert.equal(ji.devError, null);
  assert.deepEqual(Object.keys(ji.dev), ['branches', 'commits', 'pull_requests']);
});
