import test from 'node:test';
import assert from 'node:assert/strict';
import { runPreRequest, runPostResponse } from '../src/lib/api/scripts.ts';

const request = () => ({ method: 'GET', url: 'https://example.test', headers: [], body: '' });
const response = { code: 200, status: 'OK', responseTime: 2, headers: {}, bodyText: '{"ok":true}' };

test('pre-script keeps pm request/header and variable behavior', () => {
  const req = request(); const vars: Record<string, string> = {};
  const result = runPreRequest("pm.request.method='POST'; pm.request.headers.upsert({key:'X-Test',value:'yes'}); pm.environment.set('next',42); console.log('done')", req, vars);
  assert.equal(result.error, undefined); assert.equal(req.method, 'POST');
  assert.deepEqual(req.headers, [{ key: 'X-Test', value: 'yes', enabled: true }]);
  assert.equal(vars.next, '42'); assert.deepEqual(result.logs, ['done']);
});

test('console bursts stop retaining entries at the script output budget', () => {
  const result = runPreRequest("for(let i=0;i<1001;i++) console.log('line')", request(), {});
  assert.match(result.error ?? '', /output.*budget/i);
  assert.ok(result.logs.length <= 1000);
});

test('oversized log text is rejected before retention', () => {
  const result = runPreRequest("console.log('x'.repeat(300*1024))", request(), {});
  assert.match(result.error ?? '', /output.*budget/i);
  assert.ok(result.logs.join('').length <= 256*1024);
});

test('post-script test bursts share bounded output collection', () => {
  const result = runPostResponse("for(let i=0;i<1001;i++) pm.test('ok',()=>pm.expect(pm.response.code).toBe(200))", response, {});
  assert.match(result.error ?? '', /output.*budget/i);
  assert.ok(result.tests.length <= 1000);
});

test('post-script keeps passed and failed pm tests plus chaining', () => {
  const vars: Record<string, string> = {};
  const result = runPostResponse("pm.test('ok',()=>pm.expect(pm.response.json().ok).toBe(true)); pm.test('fails',()=>pm.expect(2).toBe(3)); pm.variables.set('next','yes')", response, vars);
  assert.equal(result.error, undefined); assert.equal(result.tests[0].passed, true);
  assert.equal(result.tests[1].passed, false); assert.equal(vars.next, 'yes');
});
