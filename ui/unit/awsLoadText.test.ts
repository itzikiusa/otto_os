// error-handling-4-02: the AWS views (S3, Logs, Athena) store a failed load
// through awsLoadText — the human line (no WebKit "Load failed", no raw daemon
// 401/404 text), the provider's own 403 text, and the verbatim
// "login required…" message the Sign in action keys on.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) { super(message); this.status = status; }
}

const { awsLoadText } = loadSource(new URL('../src/modules/aws/loadText.ts', import.meta.url), {
  '../../lib/api/aws': { isLoginRequired: (e: unknown) => e instanceof Error && /^login required/i.test(e.message) },
}, { TypeError, Error });

test('a dropped daemon reads as unreachable, not "Load failed"', () => {
  assert.equal(awsLoadText(new TypeError('Load failed')), 'Otto can’t reach the daemon.');
});

test('daemon 401/404 get the human line; a provider 403 keeps its text', () => {
  assert.equal(awsLoadText(new ApiError(401, 'unauthorized: bad token')), 'Your session expired. Sign in again, then retry.');
  assert.equal(awsLoadText(new ApiError(404, 'no route')), 'It no longer exists, or this daemon doesn’t serve it.');
  assert.equal(awsLoadText(new ApiError(403, 'AccessDenied: s3:ListBucket')), 'AccessDenied: s3:ListBucket');
});

test('a login-required failure keeps its raw text so the Sign in action still shows', () => {
  const text = awsLoadText(new ApiError(400, 'login required: run aws sso login'));
  assert.equal(text, 'login required: run aws sso login');
  assert.ok(/^login required/i.test(new Error(text).message));
});
