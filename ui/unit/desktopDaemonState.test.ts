import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  daemonSupervisor,
  installFailed,
  offlineDetail,
  supervisorErrorText,
  type DaemonReport,
} from '../src/modules/desktop/daemonState.ts';

const report = (over: Partial<DaemonReport>): DaemonReport => ({
  healthy: false,
  installed: true,
  daemon_version: null,
  app_version: '0.2.0',
  detail: '',
  ...over,
});

test('the offline screen shows the supervisor detail only while the daemon is down', () => {
  const wedged = report({
    detail: 'daemon loaded but not answering after 40 s — click Restart daemon',
  });
  assert.equal(offlineDetail(wedged), wedged.detail);
  assert.equal(offlineDetail(report({ healthy: true, detail: 'daemon healthy' })), null);
  assert.equal(offlineDetail(report({ detail: '   ' })), null);
  assert.equal(offlineDetail(null), null);
});

test('Reinstall is offered only after a failed install', () => {
  assert.equal(
    installFailed(report({ detail: 'install failed: launchctl bootstrap failed after 20s: 5' })),
    true,
  );
  assert.equal(installFailed(report({ detail: 'install panicked: boom' })), true);
  assert.equal(installFailed(report({ detail: 'daemon loaded but not answering after 40 s' })), false);
  assert.equal(installFailed(report({ healthy: true, detail: 'install failed: stale' })), false);
  assert.equal(installFailed(null), false);
});

test('a failed restart reads as one line', () => {
  assert.equal(supervisorErrorText(new Error(' kickstart: no such process ')), 'kickstart: no such process');
  assert.equal(supervisorErrorText('Could not find service'), 'Could not find service');
  assert.equal(supervisorErrorText(''), 'the daemon did not respond');
});

test('outside the Tauri shell every supervisor call is a no-op', async () => {
  assert.equal(await daemonSupervisor.status(), null);
  assert.equal(await daemonSupervisor.start(), null);
  assert.equal(await daemonSupervisor.restart(), null);
  assert.equal(await daemonSupervisor.reinstall(), null);
  const stop = await daemonSupervisor.onState(() => {});
  stop();
});
