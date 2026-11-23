// Phase 2 Deep Push tests for Browser (`E-04b` SPA mutation guard + payload preview).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { approvalFacts } from '../src/modules/browser/live/approvalFacts.ts';
import { approvalWireMeta, encode, parseServer } from '../src/modules/browser/live/protocol.ts';
import { makeSetGuardMsg, readSpaGuardFlag } from '../src/modules/browser/live/spaGuard.ts';
import type { BrowserLiveSession, McpApproval } from '../src/lib/api/types.ts';

function fakeApproval(argsObj: Record<string, unknown>): McpApproval {
  return {
    id: 'apr_1',
    workspace_id: 'ws_1',
    kind: 'browser_action',
    tool: 'browser_click',
    title: 'POST api.example.com/orders',
    detail: 'Submit checkout order',
    status: 'pending',
    requested_by: 'claude',
    requested_by_kind: 'agent',
    args_redacted_json: JSON.stringify(argsObj),
    created_at: '2026-11-19T20:00:00Z',
    decided_at: null,
    decided_by: null,
    decision_note: null,
  } as unknown as McpApproval;
}

test('approvalFacts formats SPA Fetch/XHR method and redacted JSON body preview', () => {
  const f = approvalFacts(
    fakeApproval({
      origin: 'https://shop.example.com',
      method: 'post',
      resource_type: 'fetch',
      target_host: 'api.example.com',
      body_preview: '{"item":"widget","qty":2,"token":"[REDACTED]"}',
      screenshot: '/tmp/shot.png',
    }),
    'default',
  );
  assert.equal(f.rows.find((r) => r.label === 'What')?.value, 'POST (FETCH) request to api.example.com');
  assert.ok(f.bodyPreview?.includes('"item": "widget"'), 'compact JSON is pretty-printed');
  assert.ok(f.bodyPreview?.includes('[REDACTED]'));
  assert.equal(f.screenshot, true);
});

test('approvalFacts renders WS wire resource_type and body_preview even when MCP detail is null', () => {
  const raw = JSON.stringify({
    type: 'approval',
    approval_id: 'apr_ws',
    status: 'pending',
    title: 'DELETE api.example.com/keys/1',
    resource_type: 'XHR',
    body_preview: '{"confirm":true,"secret":"[REDACTED]"}',
  });
  const parsed = parseServer(raw);
  assert.ok(parsed && parsed.type === 'approval');
  const meta = approvalWireMeta(parsed);
  assert.equal(meta.resourceType, 'XHR');
  assert.ok(meta.bodyPreview?.includes('[REDACTED]'));

  const f = approvalFacts(null, 'ephemeral', meta);
  assert.equal(f.rows.find((r) => r.label === 'What')?.value, 'XHR write request');
  assert.ok(f.bodyPreview?.includes('"confirm": true'));
});

test('approvalFacts truncates oversized body_preview and omits blank payloads', () => {
  const big = 'x'.repeat(800);
  const f1 = approvalFacts(fakeApproval({ method: 'PUT', target_host: 'api.example.com', payload_preview: big }), null);
  assert.equal(f1.bodyPreview?.length, 601);
  assert.ok(f1.bodyPreview?.endsWith('…'));

  const f2 = approvalFacts(fakeApproval({ method: 'DELETE', target_host: 'api.example.com', body_preview: '   ' }), null);
  assert.equal(f2.bodyPreview, null);
});

test('SPA mutation guard WS message encoding and session flag extraction', () => {
  const msg = makeSetGuardMsg(true);
  assert.deepEqual(msg, { type: 'set_guard', guard_spa_mutations: true });
  assert.equal(encode(msg), '{"type":"set_guard","guard_spa_mutations":true}');
  assert.equal(encode(makeSetGuardMsg(false)), '{"type":"set_guard","guard_spa_mutations":false}');

  const sessOn = { id: 'tab-1', guard_spa_mutations: true } as unknown as BrowserLiveSession;
  const sessOff = { id: 'tab-1', guard_spa_mutations: false } as unknown as BrowserLiveSession;
  assert.equal(readSpaGuardFlag(sessOn), true);
  assert.equal(readSpaGuardFlag(sessOff), false);
  assert.equal(readSpaGuardFlag({ id: 'tab-1' } as unknown as BrowserLiveSession), null);
  assert.equal(readSpaGuardFlag(null), null);
});
