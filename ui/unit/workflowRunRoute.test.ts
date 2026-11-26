import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseRunRoute, runDeepLink, runOrigin, runRoutePath } from '../src/modules/workflows/runRoute.ts';

test('the run route names a workflow and a run', () => {
  assert.deepEqual(parseRunRoute(['workflows', 'wf1', 'runs', 'r9']), { workflowId: 'wf1', runId: 'r9' });
  assert.equal(parseRunRoute(['workflows', 'wf1']), null);
  assert.equal(parseRunRoute(['workflows', 'wf1', 'runs']), null);
  assert.equal(parseRunRoute(['workflows', 'wf1', 'versions', 'r9']), null);
  assert.equal(parseRunRoute(['agents', 'wf1', 'runs', 'r9']), null);
  assert.equal(parseRunRoute([]), null);
});

test('the copied link round-trips through the parser', () => {
  assert.equal(runRoutePath('wf1', 'r9'), 'workflows/wf1/runs/r9');
  const link = runDeepLink({ origin: 'http://127.0.0.1:7700', pathname: '/' }, 'wf1', 'r9');
  assert.equal(link, 'http://127.0.0.1:7700/#/workflows/wf1/runs/r9');
  const parts = link.split('#/')[1].split('/').map(decodeURIComponent);
  assert.deepEqual(parseRunRoute(parts), { workflowId: 'wf1', runId: 'r9' });
});

test('a chat-started run says where it came from', () => {
  assert.deepEqual(runOrigin({ channel: 'slack', chat: 'C123', thread: '1715.42', user: 'U1', raw: 'run PR Reviewer: x' }), {
    source: 'Slack', chat: 'C123', thread: '1715.42', user: 'U1', raw: 'run PR Reviewer: x',
  });
  assert.deepEqual(runOrigin({ channel: 'telegram', chat: '-100', thread: null }), {
    source: 'Telegram', chat: '-100', thread: null, user: null, raw: null,
  });
  assert.equal(runOrigin({ channel: 'webhook', chat: 'x' }), null);
  assert.equal(runOrigin({ trigger: 'schedule', result_channel: 'slack', result_chat: 'C1' }), null);
  assert.equal(runOrigin(null), null);
  assert.equal(runOrigin('text'), null);
});

test('a run link can name the open step and one of its reviewers', () => {
  assert.deepEqual(parseRunRoute(['workflows', 'wf1', 'runs', 'r9', 'n7']), { workflowId: 'wf1', runId: 'r9', nodeId: 'n7' });
  assert.deepEqual(parseRunRoute(['workflows', 'wf1', 'runs', 'r9', 'n7', 'correctness']), { workflowId: 'wf1', runId: 'r9', nodeId: 'n7', childId: 'correctness' });
  assert.equal(runRoutePath('wf1', 'r9', 'n7', 'a b'), 'workflows/wf1/runs/r9/n7/a%20b');
  assert.equal(runRoutePath('wf1', 'r9', null, 'x'), 'workflows/wf1/runs/r9');
  const parts = runRoutePath('wf1', 'r9', 'n7', 'c').split('/').map(decodeURIComponent);
  assert.deepEqual(parseRunRoute(parts), { workflowId: 'wf1', runId: 'r9', nodeId: 'n7', childId: 'c' });
});
