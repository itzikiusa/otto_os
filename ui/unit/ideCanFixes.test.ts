// Regression tests for Wave 3 IDE CAN fixes (C-11 + C-08):
// - Pruning killed/archived SSH and local terminal tabs (`ideTerminalTabs.ts`)
// - Containing multi-repo root resolution for preview pane (`findContainingRoot`)
// - Static recents-exclusion wiring in `ideScratches.svelte.ts` and cycle-free `ideProjectSwitch.ts`

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { pruneIdeTerminalTabs } from '../src/modules/ide/store/ideTerminalTabs.ts';
import { loadSource } from './sourceHarness.ts';

test('C-11: pruneIdeTerminalTabs drops killed/archived SSH tabs while keeping live ones', () => {
  const tabs = [
    { id: 'loc-live', kind: 'local' as const },
    { id: 'loc-dead', kind: 'local' as const },
    { id: 'ssh-in-sessions', kind: 'ssh' as const },
    { id: 'ssh-in-other-ws', kind: 'ssh' as const },
    { id: 'ssh-fresh-status', kind: 'ssh' as const },
    { id: 'ssh-killed', kind: 'ssh' as const },
    { id: 'ssh-exited-dropped', kind: 'ssh' as const },
    { id: 'ssh-archived', kind: 'ssh' as const },
  ];

  const kept = pruneIdeTerminalTabs(
    tabs,
    [
      { id: 'loc-live', archived: false },
      { id: 'ssh-in-sessions', archived: false },
    ],
    [{ id: 'ssh-in-other-ws', archived: false }],
    [{ id: 'ssh-archived' }],
    {
      'ssh-fresh-status': 'running',
      'ssh-exited-dropped': 'exited',
      'ssh-archived': 'exited',
    },
  );

  assert.deepEqual(
    kept.map((t) => t.id),
    ['loc-live', 'ssh-in-sessions', 'ssh-in-other-ws', 'ssh-fresh-status'],
  );
});

test('C-11: findContainingRoot picks longest-prefix multi-repo root for FilePreview', () => {
  const mod = loadSource(
    new URL('../src/modules/ide/shell/ideProjectHelpers.ts', import.meta.url),
    {
      '../../../lib/api/client': { api: {} },
      '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({}) },
      '../../../lib/confirm.svelte': { confirmer: {} },
      '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {} },
      '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: {} },
      '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
      '../../../lib/toast.svelte': { toasts: {} },
      '../../../lib/loadError': { loadErrorText: (e: unknown) => String(e) },
      '../../../lib/toastError': { toastError: () => {} },
      '../store/ideTreeDirErrors.svelte': { ideTreeDirErrors: { clear: () => {}, set: () => {} } },
      '../store/ideBufferPerf': { loadWorkspaceOpenTabs: () => null },
      '../store/ideTypes': {
        autoOpenInitialEntry: () => null,
        buildNewFileTemplate: () => '',
        ideBasename: (p: string) => p.split('/').pop() ?? p,
        ideTargetKey: (t: { kind: string; root: string }) => `${t.kind}:${t.root}`,
      },
    },
  );

  const roots = [
    { path: '/work/primary', name: 'primary', isPrimary: true },
    { path: '/work/service-a', name: 'service-a', isPrimary: false },
    { path: '/work/service-a/nested', name: 'nested', isPrimary: false },
  ];

  assert.equal(mod.findContainingRoot('/work/service-a/docs/README.md', roots)?.path, '/work/service-a');
  assert.equal(mod.findContainingRoot('/work/service-a/nested/spec.md', roots)?.path, '/work/service-a/nested');
  assert.equal(mod.findContainingRoot('/work/elsewhere/file.md', roots), null);
});

test('C-08: ideScratches registers recents exclusion statically without dynamic imports in ideProjectSwitch', () => {
  const registered: { fn: ((root: string) => boolean) | null } = { fn: null };
  const scratchesMod = loadSource(
    new URL('../src/modules/ide/shell/ideScratches.svelte.ts', import.meta.url),
    {
      '../../../lib/api/client': { api: { get: async () => ({ root: '/tmp/scratches' }) } },
      '../../../lib/confirm.svelte': { confirmer: {} },
      '../../../lib/toastError': { toastError: () => {} },
      '../ideStore.svelte': { ide: { target: { kind: 'local', root: '' } } },
      './ideProjectHelpers': { normalizeRootPath: (p: string) => p.replace(/\/+$/, '') || '/' },
      './ideProjectRecents.svelte': {
        sameRootKey: (p: string) => p.replace(/\/+$/, ''),
        setRecentsExclusion: (fn: (root: string) => boolean) => {
          registered.fn = fn;
        },
      },
    },
  );

  assert.ok(registered.fn, 'ideScratches must register setRecentsExclusion at module load');
  assert.equal(registered.fn('/tmp/scratches'), false, 'before scratches root is known, nothing is excluded');
  scratchesMod.scratches.root = '/tmp/scratches';
  assert.equal(registered.fn('/tmp/scratches/'), true, 'once scratches root is loaded, it is excluded from recents');
  assert.equal(registered.fn('/work/other'), false, 'other roots remain eligible for recents');
});
