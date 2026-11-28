// Unity Editor attach (rc.6 A1): the launch target's running suffix and the
// failure card that installs the Unity debugger (not netcoredbg).
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ATTACH_ONLY_TITLE, unityTargetLabel } from '../src/modules/ide/debug/ideUnityAttach.ts';
import { debugErrorCard } from '../src/modules/ide/debug/ideDebugPanelModel.ts';

test('the Unity target says whether an Editor has the project open, once asked', () => {
  const name = 'Attach to Unity Editor';
  assert.equal(unityTargetLabel(name, null), name);
  assert.equal(unityTargetLabel(name, []), 'Attach to Unity Editor · not running');
  assert.equal(
    unityTargetLabel(name, [{ pid: 4242, version: '6000.0.23f1', port: 56242, project_root: '/p' }]),
    'Attach to Unity Editor · 6000.0.23f1 · running',
  );
  assert.equal(ATTACH_ONLY_TITLE, 'Attach targets can only be debugged');
});

test('a missing Unity debugger offers its own install', () => {
  const card = debugErrorCard(
    { message: "The Unity debugger isn't installed.", kind: 'adapter_missing', ecosystem: 'unity' },
    null,
  );
  assert.equal(card?.title, 'Unity debugger isn’t installed');
  assert.deepEqual(card?.primary, { id: 'install', label: 'Install Unity debugger' });
  assert.equal(card?.ecosystem, 'unity');
});
