// A remote server that needs an OAuth sign-in says so on its row.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isRemote, needsSignIn } from '../src/modules/mcp/runMode.ts';

test('reads the daemon’s “Needs sign-in” health error', () => {
  assert.equal(needsSignIn({ health_error: 'Needs sign-in: the server asked for authorization' }), true);
  assert.equal(needsSignIn({ health_error: 'connect refused' }), false);
  assert.equal(needsSignIn({ health_error: null }), false);
});

test('offers sign-in only to remote servers', () => {
  assert.equal(isRemote({ transport: 'sse' }), true);
  assert.equal(isRemote({ transport: 'http' }), true);
  assert.equal(isRemote({ transport: 'stdio' }), false);
});
