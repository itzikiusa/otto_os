// S17-306: the Jira account form must ask for a fresh token exactly when the
// daemon will (otto-issues `require_token_for_host_change`): scheme + host +
// port, default ports folded, path changes keep the token.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { credentialOrigin, originChanged } from '../src/modules/settings/credentialOrigin.ts';

test('a scheme change moves the token (the old host:port compare missed it)', () => {
  assert.equal(originChanged('https://acme.atlassian.net', 'http://acme.atlassian.net'), true);
});

test('host and port changes move it; path / case / default port do not', () => {
  assert.equal(originChanged('https://acme.atlassian.net', 'https://evil.example'), true);
  assert.equal(originChanged('https://jira.local:8443', 'https://jira.local:9443'), true);
  assert.equal(originChanged('https://acme.atlassian.net', 'https://ACME.atlassian.net/wiki'), false);
  assert.equal(originChanged('https://acme.atlassian.net', 'https://acme.atlassian.net:443/'), false);
  assert.equal(credentialOrigin('https://Acme.Atlassian.net/x'), 'https://acme.atlassian.net');
});

test('unparseable input: only an identical string is the same', () => {
  assert.equal(originChanged('not a url', 'not a url '), false);
  assert.equal(originChanged('not a url', 'NOT A URL'), true);
  assert.equal(originChanged('https://a.example', 'garbage'), true);
});
