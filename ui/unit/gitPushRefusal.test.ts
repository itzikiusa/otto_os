import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pushRefusal } from '../src/modules/git/push-errors.ts';

test('a non-fast-forward 409 is a rejection the user can act on', () => {
  const msg =
    "conflict: push rejected: the remote branch has commits yours doesn't — pull to bring them in, then push again (! [rejected] main -> main (fetch first))";
  assert.equal(pushRefusal(409, msg), 'rejected');
});

test('a refused lease is told apart from a plain rejection', () => {
  const msg =
    'force push refused: the remote branch moved since you last fetched and integrated it — fetch, review the new commits, then try again (! [rejected] main -> main (stale info))';
  assert.equal(pushRefusal(409, msg), 'lease');
});

test('anything else — auth, network, other 409s — is not a push refusal', () => {
  assert.equal(pushRefusal(502, 'git exited 128: fatal: Authentication failed'), null);
  assert.equal(pushRefusal(502, 'push rejected: the remote branch'), null);
  assert.equal(pushRefusal(409, 'Please commit your changes or stash them'), null);
  assert.equal(pushRefusal(undefined, 'push rejected:'), null);
});
