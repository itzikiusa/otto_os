import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isMutatingApiCall, isProdApiEnv } from '../src/modules/api/prodGuard.ts';

test('isProdApiEnv detects production environment names without false positives', () => {
  assert.equal(isProdApiEnv('Production'), true);
  assert.equal(isProdApiEnv('prod'), true);
  assert.equal(isProdApiEnv('PRD'), true);
  assert.equal(isProdApiEnv('Live'), true);
  assert.equal(isProdApiEnv('API Prod'), true);
  assert.equal(isProdApiEnv('payments-prod-eu'), true);
  assert.equal(isProdApiEnv('us_production'), true);

  assert.equal(isProdApiEnv(''), false);
  assert.equal(isProdApiEnv(null), false);
  assert.equal(isProdApiEnv(undefined), false);
  assert.equal(isProdApiEnv('Staging'), false);
  assert.equal(isProdApiEnv('Preprod'), false);
  assert.equal(isProdApiEnv('pre-prod'), false);
  assert.equal(isProdApiEnv('non-prod'), false);
  assert.equal(isProdApiEnv('Development'), false);
  assert.equal(isProdApiEnv('Local'), false);
  assert.equal(isProdApiEnv('Products'), false);
});

test('isMutatingApiCall flags mutating HTTP/SSE verbs and all gRPC calls', () => {
  assert.equal(isMutatingApiCall({ kind: 'http', method: 'GET' }), false);
  assert.equal(isMutatingApiCall({ kind: 'http', method: 'head' }), false);
  assert.equal(isMutatingApiCall({ kind: 'http', method: 'OPTIONS' }), false);
  assert.equal(isMutatingApiCall({ kind: 'http', method: 'POST' }), true);
  assert.equal(isMutatingApiCall({ kind: 'http', method: 'PUT' }), true);
  assert.equal(isMutatingApiCall({ kind: 'http', method: 'PATCH' }), true);
  assert.equal(isMutatingApiCall({ kind: 'http', method: 'DELETE' }), true);
  assert.equal(isMutatingApiCall({ kind: 'grpc', method: 'POST' }), true);
  assert.equal(isMutatingApiCall({ kind: 'grpc', method: 'GET' }), true);
  assert.equal(isMutatingApiCall({ kind: 'sse', method: 'GET' }), false);
  assert.equal(isMutatingApiCall({ kind: 'sse', method: 'POST' }), true);
  assert.equal(isMutatingApiCall({ kind: 'websocket', method: 'GET' }), false);
});
