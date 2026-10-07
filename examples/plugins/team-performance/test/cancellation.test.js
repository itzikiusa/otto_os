'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { makeClient } = require('../lib/jira.js');

test('quality: Jira Stop aborts active I/O without retrying', async () => {
  let started;
  const ready = new Promise((r) => { started = r; });
  let calls = 0;
  const server = http.createServer((_req, res) => { calls++; started(); res.writeHead(200); res.write('['); });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const controller = new AbortController();
  const client = makeClient({ base_url: `http://127.0.0.1:${server.address().port}`, email: 'fixture', token: 'fixture' }, { signal: controller.signal });
  try {
    const call = client.fields();
    const outcome = call.then(() => 'resolved', (e) => e.name);
    await ready;
    controller.abort();
    assert.equal(await Promise.race([outcome, new Promise((r) => setTimeout(() => r('hung'), 300))]), 'AbortError');
    assert.equal(calls, 1);
  } finally {
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
  }
});
