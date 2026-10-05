import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

const preview = { kind: 'jira', story_id: 'story', account_label: 'Reviewed account', account_url: 'https://example.test', body_md: 'Full content\n'.repeat(1000), request: { project_key: 'PRJ', reviewed_content: { title: 'Reviewed title', url: '', source_kind: 'draft' } } };
const node = { node_id: 'gate', detail_version: 'v1' };
function fixture(read: () => Promise<unknown>) {
  return componentFunctions(new URL('../src/modules/workflows/WorkflowsPage.svelte', import.meta.url), ['loadApprovalPreview'], {
    approvalPreviewSequence: 0, approvalPreviewAbort: null, approvalPreview: null, approvalPreviewReady: false,
    approvalPreviewVersion: null, approvalPreviewLoading: false, approvalPreviewError: '', AbortController, loadErrorText: String,
    run: { id: 'run', waiting_approval: true, approval_node_id: 'gate' }, workflowNodeDetail: read,
  });
}

test('workflow approval loads the full exact pending gate preview', async () => {
  const state=fixture(async () => ({detail_version:'v1',body:{output:{publication_preview:preview}}}));
  await state.loadApprovalPreview('run',node);
  assert.equal(state.approvalPreviewReady,true);
  assert.equal(state.approvalPreview.body_md,preview.body_md);
  assert.equal(state.approvalPreview.request.project_key,'PRJ');
});

test('workflow approval rejects a different detail version and supports Retry', async () => {
  let version='v2';
  const state=fixture(async () => ({detail_version:version,body:{output:{publication_preview:preview}}}));
  await state.loadApprovalPreview('run',node);
  assert.equal(state.approvalPreviewReady,false);
  assert.match(state.approvalPreviewError,/changed/);
  version='v1'; await state.loadApprovalPreview('run',node);
  assert.equal(state.approvalPreviewReady,true); assert.equal(state.approvalPreviewError,'');
});

test('departed run cannot adopt a late approval preview', async () => {
  const response=deferred<unknown>(); const state=fixture(() => response.promise);
  const pending=state.loadApprovalPreview('run',node);
  state.run={id:'other',waiting_approval:true,approval_node_id:'gate'};
  response.resolve({detail_version:'v1',body:{output:{publication_preview:preview}}}); await pending;
  assert.equal(state.approvalPreview,null); assert.equal(state.approvalPreviewReady,false);
});

function approvalFixture(productGate = true) {
  const requests: { path: string; body: Record<string, unknown> }[] = [];
  const state = componentFunctions(new URL('../src/modules/workflows/WorkflowsPage.svelte', import.meta.url), ['approveRun'], {
    run: { id: 'same-run', waiting_approval: true, approval_node_id: 'same-gate', nodes: [{node_id:'same-gate',detail_version:'new-summary-v2'}] },
    approving: false, approvingKind: null, approvalNeedsPreview: productGate, approvalPreviewReady: true,
    approvalPreviewLoading: false, approvalPreviewError: '', approvalPreviewVersion: 'displayed-body-v1',
    approvalPreview: productGate ? preview : null,
    api: { post: async (path: string, body: Record<string, unknown>) => { requests.push({path,body}); } },
    toastError() {}, toasts: { success() {}, error() {} }, refetchRun: async () => {},
  });
  return { state, requests };
}

for (const approved of [true, false]) {
  test(`Product ${approved ? 'approve' : 'deny'} submits the displayed preview identity, not the latest summary`, async () => {
    const {state,requests}=approvalFixture();
    await state.approveRun(approved);
    assert.equal(requests.length,1);
    assert.equal(requests[0].body.expected_detail_version,'displayed-body-v1');
    assert.equal(requests[0].body.approved,approved);
  });
}

test('Product approval cannot submit without a successfully displayed preview', async () => {
  const {state,requests}=approvalFixture(); state.approvalPreviewReady=false;state.approvalPreviewVersion=null;
  await state.approveRun(true);
  assert.equal(requests.length,0);
});

test('stale Product approval conflict invalidates the displayed identity before another click', async () => {
  const {state}=approvalFixture();
  state.api.post=async () => { const error=new Error('Preview replaced');Object.assign(error,{status:409});throw error; };
  await state.approveRun(true);
  assert.equal(state.approvalPreviewReady,false);
  assert.equal(state.approvalPreviewVersion,null);
  assert.match(state.approvalPreviewError,/review|preview/i);
});

test('ordinary non-Product approvals keep their existing request shape', async () => {
  const {state,requests}=approvalFixture(false);
  await state.approveRun(true,'Looks good');
  assert.deepEqual(JSON.parse(JSON.stringify(requests[0].body)),{node_id:'same-gate',approved:true,note:'Looks good'});
});
