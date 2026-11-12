// Exercise the production async save boundary; DOM rendering is covered by E2E.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';
import * as sandboxExemptHelpers from '../src/modules/settings/sandboxExempt.ts';
import { machineBody, machineFormFrom } from '../src/modules/settings/machineSettings.ts';

const source = readFileSync(new URL('../src/modules/settings/Daemon.svelte', import.meta.url), 'utf8')
  .split('<script lang="ts">')[1].split('</script>')[0];
const ast = ts.createSourceFile('Daemon.ts', source, ts.ScriptTarget.Latest, true);
const save = ast.statements.find(s => ts.isFunctionDeclaration(s) && s.name?.text === 'save');

test('edits made during a daemon settings save remain unsaved', async () => {
  assert.ok(save);
  const response = deferred<Record<string, unknown>>();
  let sent: any;
  let metaReread = false;
  let postureReread = false;
  const context = createContext({
    dirty: true, portError: '', formError: '', listenerDirty: true, sandboxDirty: true, sessionsDirty: true,
    enabled: true, port: 7701, savedListener: { enabled: true, port: 7700 },
    sandboxEnabled: true, sandboxNetwork: 'loopback', sandboxRest: {}, sandboxStored: {}, sandboxExempt: [],
    ...sandboxExemptHelpers, savedSandbox: { enabled: false, network: 'full' },
    persistEnabled: false, manualGrace: 3600, archiveDays: 14,
    savedSessions: { persist: true, manualGrace: 86400, archiveDays: 0 },
    saving: false, allSettings: {}, auth: { meta: {}, refreshMeta: async () => { metaReread = true; return true; } },
    daemonStatus: { refreshPosture: () => { postureReread = true; } },
    machineDirty: false, machine: machineFormFrom({}), savedMachine: machineFormFrom({}), machineBody,
    machineStatus: { refresh() {} },
    canRestartDaemon: false, restartDaemonNow: async () => false,
    idleSuspend: {}, policyFromSettings: (v: unknown) => v,
    api: { put: async (_: string, body: unknown) => { sent = body; return response.promise; } },
    toasts: { success() {} }, toastError: (_: string, error: unknown) => { throw error; },
  });
  runInContext(ts.transpileModule(save.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  const pending = runInContext('save()', context);
  context.port = 7702;
  context.sandboxNetwork = 'none';
  context.manualGrace = 7200;
  context.archiveDays = 30;
  response.resolve(sent);
  await pending;
  assert.equal(sent.session_auto_archive_days, 14);
  assert.equal(context.savedSessions.archiveDays, 14);
  assert.notEqual(context.archiveDays, context.savedSessions.archiveDays);
  // The status bar must not claim a bound listener on Save: the page
  // re-reads /meta + the posture instead of writing auth.meta optimistically.
  assert.equal(context.auth.meta.network_listener, undefined);
  assert.ok(metaReread && postureReread);
  assert.equal(sent.network_listener.port, 7701);
  assert.equal(context.savedListener.port, 7701);
  assert.equal(context.savedSandbox.network, 'loopback');
  assert.equal(context.savedSessions.manualGrace, 3600);
  assert.notEqual(context.port, context.savedListener.port);
  assert.notEqual(context.sandboxNetwork, context.savedSandbox.network);
  assert.notEqual(context.manualGrace, context.savedSessions.manualGrace);
});

test('machine resource settings ride the same Save, only the changed keys', async () => {
  assert.ok(save);
  let sent: any;
  let statusReread = false;
  const notes: string[] = [];
  const saved = machineFormFrom({});
  const context = createContext({
    dirty: true, formError: '', listenerDirty: false, sandboxDirty: false, sessionsDirty: false,
    enabled: false, port: 7700, savedListener: { enabled: false, port: 7700 },
    sandboxEnabled: false, sandboxNetwork: 'full', sandboxStored: {}, sandboxExempt: [], ...sandboxExemptHelpers,
    savedSandbox: { enabled: false, network: 'full', exempt: [] },
    persistEnabled: true, manualGrace: 86400, archiveDays: 0,
    savedSessions: { persist: true, manualGrace: 86400, archiveDays: 0 },
    machineDirty: true, machine: { ...saved, maxLive: 3, buildMode: 'off' }, savedMachine: saved, machineBody,
    machineStatus: { refresh() { statusReread = true; } },
    saving: false, allSettings: {}, auth: { meta: {}, refreshMeta: async () => true },
    daemonStatus: { refreshPosture() {} }, canRestartDaemon: false, restartDaemonNow: async () => false,
    idleSuspend: {}, policyFromSettings: (v: unknown) => v,
    api: { put: async (_: string, body: unknown) => { sent = body; return body; } },
    toasts: { success: (_: string, text: string) => { notes.push(text); } },
  });
  runInContext(ts.transpileModule(save.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  await runInContext('save()', context);
  assert.deepEqual(JSON.parse(JSON.stringify(sent)), { agent_background_max_live: 3, agent_build_parallelism: 'off' });
  assert.equal(context.savedMachine.maxLive, 3);
  assert.ok(statusReread, 'the live line re-reads the new limits');
  assert.match(notes[0], /next background agent/);
});
