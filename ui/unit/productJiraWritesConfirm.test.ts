// S18-302: every live Jira write in the story Overview (status, assignee,
// title, description, custom fields, comments) notifies watchers and can fire
// Jira automations, so each one must go through `confirmOutward` first. This
// pins it per function: a new issue-client write without a confirm fails.
// The writes live in two places since architecture-3-05: OverviewTab (title,
// description, comments) and the extracted Jira editor (status, assignee,
// fields), and every `/issue/*` URL is built by lib/api/issues.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

const overview = readFileSync(new URL('../src/modules/product/OverviewTab.svelte', import.meta.url), 'utf8');
const editor = readFileSync(new URL('../src/modules/product/jira/issueEditorState.svelte.ts', import.meta.url), 'utf8');

const WRITE = /issues\.(updateFields|updateDescription|addComment|applyTransition|setAssignee)\(/;

/** Body of every `async name(` (function or class method) at the given indent. */
function functions(text: string, re: RegExp): Array<{ name: string; body: string }> {
  const out: Array<{ name: string; body: string }> = [];
  const starts = [...text.matchAll(re)];
  for (let i = 0; i < starts.length; i++) {
    const from = starts[i].index ?? 0;
    const to = i + 1 < starts.length ? (starts[i + 1].index ?? text.length) : text.length;
    out.push({ name: starts[i][1], body: text.slice(from, to) });
  }
  return out;
}

function assertConfirmedFirst(writers: Array<{ name: string; body: string }>): void {
  for (const f of writers) {
    const confirmAt = f.body.indexOf('confirmOutward(');
    const writeAt = f.body.search(WRITE);
    assert.ok(confirmAt >= 0 && confirmAt < writeAt, `${f.name} writes to Jira without confirmOutward first`);
  }
}

test('every Overview Jira write is confirmed before it is sent', () => {
  const script = overview.split('</script>')[0];
  const writers = functions(script, /\n {2}async function (\w+)\s*\(/g).filter((f) => WRITE.test(f.body));
  assert.deepEqual(writers.map((f) => f.name).sort(), ['addComment', 'saveDesc', 'saveTitle']);
  assertConfirmedFirst(writers);
  // No hand-built issue URL survives in the tab: the client encodes segments.
  assert.doesNotMatch(script, /`\/issue\//);
});

test('every Jira editor write (status, assignee, fields) is confirmed before it is sent', () => {
  const writers = functions(editor, /\n {2}async (\w+)\s*\(/g).filter((f) => WRITE.test(f.body));
  assert.deepEqual(writers.map((f) => f.name).sort(), ['applyTransition', 'assignUser', 'saveField']);
  assertConfirmedFirst(writers);
});

test('the issue client encodes every path segment and query value', async () => {
  const calls: Array<[string, string, unknown?]> = [];
  const record = (method: string) => (path: string, body?: unknown) => {
    calls.push([method, path, body]);
    return Promise.resolve([]);
  };
  const issues = loadSource(new URL('../src/lib/api/issues.ts', import.meta.url), {
    './client': {
      api: { get: record('GET'), post: record('POST'), put: record('PUT'), patch: record('PATCH'), del: record('DELETE') },
      authedBlobUrl: (path: string) => { calls.push(['BLOB', path]); return Promise.resolve('blob:x'); },
    },
    '../listingPage': { unwrapListing: (b: unknown) => (Array.isArray(b) ? { items: b, truncated: false } : b) },
  });
  await issues.issueFull('acc/1', 'GS-1?x=1');
  await issues.updateFields('acc', 'A B', { summary: 'x' });
  await issues.attachmentBlobUrl('acc', 'K-1', '../secret');
  await issues.listIssueProjects('a&b=c', { meta: true });
  await issues.searchIssues({ accountId: 'acc', q: 'a&b', project: 'P#1', startAt: 0 });
  await issues.devStatus('acc', 'K-1', null);
  await issues.deleteIssueAccount('x/y');
  assert.deepEqual(calls.map(([m, p]) => `${m} ${p}`), [
    'GET /issue/acc%2F1/GS-1%3Fx%3D1/full',
    'PUT /issue/acc/A%20B/fields',
    'BLOB /issue/acc/K-1/attachment/..%2Fsecret',
    'GET /issue/projects?account_id=a%26b%3Dc&meta=1',
    'GET /issue/search?account_id=acc&q=a%26b&project=P%231&start_at=0',
    'GET /issue/acc/K-1/devstatus',
    'DELETE /issue/accounts/x%2Fy',
  ]);
  assert.equal(JSON.stringify(calls[1][2]), JSON.stringify({ fields: { summary: 'x' } }));
});
