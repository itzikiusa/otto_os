// Exercise the production async save boundary; DOM rendering is covered by E2E.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';

test('edits made during a daemon settings save remain unsaved', async () => {
  const source = readFileSync(new URL('../src/modules/settings/Daemon.svelte', import.meta.url), 'utf8')
    .split('<script lang="ts">')[1].split('</script>')[0];
  const ast = ts.createSourceFile('Daemon.ts', source, ts.ScriptTarget.Latest, true);
  const save = ast.statements.find(s => ts.isFunctionDeclaration(s) && s.name?.text === 'save');
  assert.ok(save);
  const response = deferred<Record<string, unknown>>();
  let sent: any;
  const context = createContext({
    dirty: true, portError: '', listenerDirty: true, sandboxDirty: true, sessionsDirty: true,
    enabled: true, port: 7701, savedListener: { enabled: true, port: 7700 },
    sandboxEnabled: true, sandboxNetwork: 'loopback', sandboxRest: {}, savedSandbox: { enabled: false, network: 'full' },
    persistEnabled: false, manualGrace: 3600, savedSessions: { persist: true, manualGrace: 86400 },
    saving: false, allSettings: {}, auth: { meta: {} }, idleSuspend: {}, policyFromSettings: (v: unknown) => v,
    api: { put: async (_: string, body: unknown) => { sent = body; return response.promise; } },
    toasts: { success() {} }, toastError: (_: string, error: unknown) => { throw error; },
  });
  runInContext(ts.transpileModule(save.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  const pending = runInContext('save()', context);
  context.port = 7702;
  context.sandboxNetwork = 'none';
  context.manualGrace = 7200;
  response.resolve(sent);
  await pending;
  assert.equal(sent.network_listener.port, 7701);
  assert.equal(context.savedListener.port, 7701);
  assert.equal(context.savedSandbox.network, 'loopback');
  assert.equal(context.savedSessions.manualGrace, 3600);
  assert.notEqual(context.port, context.savedListener.port);
  assert.notEqual(context.sandboxNetwork, context.savedSandbox.network);
  assert.notEqual(context.manualGrace, context.savedSessions.manualGrace);
});
