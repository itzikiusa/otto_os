// IDE dialogs on a phone (review I4-09 / I4-11 / I4-12 / I4-21): the Local History
// sheet is list → detail with the diff filling it, the run-configuration footer is
// one row (Save + ⋯), the phone ⋯ chooses the run target, and the ⋯ drops rows the
// pane switcher already has.
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  ideCompareLayout,
  ideCompareListName,
  ideCompareStepAfterPick,
  ideCompareTitle,
} from '../src/modules/ide/history/ideCompareLayout.ts';
import { runConfigFooterMenu } from '../src/modules/ide/shell/ideRunConfigFooterMenu.ts';
import { loadSource } from './sourceHarness.ts';

// Values from a vm-loaded module come from another realm: compare plain copies.
const plain = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

describe('compare / Local History layout', () => {
  it('workbench keeps the desktop split with the footer', () => {
    assert.deepEqual(ideCompareLayout(false, true, 'diff'), { showList: true, showDiff: true, split: true, showBack: false, unifiedOnly: false, footer: true });
    assert.deepEqual(ideCompareLayout(false, true, 'list'), ideCompareLayout(false, true, 'diff'), 'the step means nothing on the workbench');
    assert.equal(ideCompareLayout(false, false, 'diff').split, false, 'clipboard compare has no list');
  });

  it('stacked: the diff fills the sheet, the list is a step of its own', () => {
    const diff = ideCompareLayout(true, true, 'diff');
    assert.deepEqual(diff, { showList: false, showDiff: true, split: false, showBack: true, unifiedOnly: true, footer: false });
    const list = ideCompareLayout(true, true, 'list');
    assert.deepEqual([list.showList, list.showDiff, list.showBack], [true, false, false]);
    const clip = ideCompareLayout(true, false, 'list');
    assert.deepEqual([clip.showList, clip.showDiff, clip.showBack, clip.footer], [false, true, false, false], 'no list → no back, the ✕ closes');
  });

  it('a pick on the phone list shows its diff; desktop stays put', () => {
    assert.equal(ideCompareStepAfterPick(true, 'list'), 'diff');
    assert.equal(ideCompareStepAfterPick(false, 'list'), 'list');
  });

  it('back names what the list holds', () => {
    assert.equal(ideCompareListName('local-history'), 'Snapshots');
    assert.equal(ideCompareListName('git-revision'), 'Revisions');
    assert.equal(ideCompareListName('git-branch'), 'Branches');
  });

  it('titles: the disk banner says "On disk", the phone names the file only', () => {
    assert.equal(ideCompareTitle('clipboard', 'main.go', 'app/main.go', 'On disk', false), 'On disk vs current — main.go');
    assert.equal(ideCompareTitle('clipboard', 'main.go', 'app/main.go', null, false), 'Compare with Clipboard — main.go');
    assert.equal(ideCompareTitle('clipboard', 'main.go', 'app/main.go', 'Clipboard', false), 'Compare with Clipboard — main.go');
    assert.equal(ideCompareTitle('local-history', 'main.go', 'app/main.go', null, true), 'Local History — main.go');
    assert.equal(ideCompareTitle('local-history', 'main.go', 'app/main.go', null, false), 'Local History — app/main.go');
    assert.equal(ideCompareTitle('file', 'a.go', 'a.go', null, false), 'Compare Files — a.go');
  });
});

describe('run-configuration footer on a phone', () => {
  const calls: string[] = [];
  const base = {
    busy: false, deletable: true, canDuplicate: true,
    onSave: (a: string) => calls.push(`save:${a}`), onDuplicate: () => calls.push('dup'), onDelete: () => calls.push('del'),
  };

  it('⋯ holds run, debug, duplicate and (separated, danger) delete', () => {
    const rows = runConfigFooterMenu(base);
    assert.deepEqual(rows.map((r) => (r.separator ? '—' : r.label)), ['Save & run', 'Save & debug', 'Duplicate', '—', 'Delete…']);
    assert.equal(rows.at(-1)!.danger, true);
    rows[0].action!(); rows[1].action!(); rows[2].action!(); rows[4].action!();
    assert.deepEqual(calls, ['save:run', 'save:debug', 'dup', 'del']);
  });

  it('a new or auto-detected draft has no Delete; busy disables every row', () => {
    assert.deepEqual(runConfigFooterMenu({ ...base, deletable: false, canDuplicate: false }).map((r) => r.label), ['Save & run', 'Save & debug']);
    assert.ok(runConfigFooterMenu({ ...base, busy: true }).filter((r) => !r.separator).every((r) => r.disabled));
  });
});

function runTargetModule(ideState: Record<string, unknown>, saved: unknown[]) {
  return loadSource(new URL('../src/modules/ide/shell/ideRunTargetMenu.ts', import.meta.url), {
    '../ideStore.svelte': { ide: ideState },
    '../store/ideLaunchTarget': {
      CURRENT_FILE_CONFIG_ID: 'current-file',
      currentFileRunnable: (tab: { name: string } | null) => (tab ? { ok: true, reason: null } : { ok: false, reason: 'Open a file first' }),
      launchSelectorLabel: (sel: { name: string } | null, tab: { name: string } | null) => (sel ? sel.name : tab ? `Current file (${tab.name})` : 'Current file'),
      // The real one puts the active file's ecosystem first; reversing proves the menu keeps the given order.
      orderDetectedForLanguage: (c: unknown[]) => [...c].reverse(),
      saveLaunchMemory: (_t: unknown, patch: unknown) => saved.push(patch),
    },
  });
}

describe('run target from the phone ⋯', () => {
  const configs = [
    { id: 's1', name: 'Python Settlement Debug', runner: 'python', auto_detected: false },
    { id: 'a1', name: 'Go Run app', runner: 'go_run', auto_detected: true },
    { id: 'a2', name: 'Go Test ./...', runner: 'go_test', auto_detected: true },
  ];

  it('rows: Current file first, then saved, then detected; the selected one is checked', () => {
    const mod = runTargetModule({}, []);
    const rows = mod.runTargetRows({ configs, selectedId: 'a1', fileName: 'main.go', currentFile: { ok: true, reason: null } });
    assert.deepEqual(plain(rows.map((r: { label: string }) => r.label)), ['Current file — main.go', 'Python Settlement Debug', 'Go Run app', 'Go Test ./...']);
    assert.deepEqual(plain(rows.map((r: { checked: boolean }) => r.checked)), [false, false, true, false]);
    assert.deepEqual(plain(rows.map((r: { sectionStart: boolean }) => r.sectionStart)), [false, true, true, false]);
  });

  it('an unknown selection reads as Current file; an unrunnable file is disabled with its reason', () => {
    const mod = runTargetModule({}, []);
    const rows = mod.runTargetRows({ configs, selectedId: 'gone', fileName: null, currentFile: { ok: false, reason: 'Open a file first' } });
    assert.equal(rows[0].label, 'Current file');
    assert.equal(rows[0].checked, true);
    assert.equal(rows[0].disabled, true);
    assert.equal(rows[0].title, 'Open a file first');
  });

  it('picking a row sets the store field and the launch memory, like the popover', () => {
    const saved: unknown[] = [];
    const ide: Record<string, unknown> = {
      runConfigs: configs, selectedConfigId: 'current-file', activeTab: { name: 'main.go', language: 'go' },
      target: { kind: 'local', root: '/repo' }, openEnvironmentsModal: () => {},
    };
    const mod = runTargetModule(ide, saved);
    assert.equal(mod.runTargetRowLabel(), 'Run target: Current file (main.go)');
    const items = mod.runTargetMenuItems() as { label?: string; separator?: boolean; action?: () => void }[];
    const labels = items.map((i) => (i.separator ? '—' : i.label));
    assert.deepEqual(plain(labels), ['Current file — main.go', '—', 'Python Settlement Debug', '—', 'Go Test ./...', 'Go Run app', '—', 'Edit run configurations…']);
    items.find((i) => i.label === 'Go Test ./...')!.action!();
    assert.equal(ide.selectedConfigId, 'a2');
    assert.deepEqual(plain(saved), [{ selected: 'a2' }]);
    assert.equal(mod.runTargetRowLabel(), 'Run target: Go Test ./...');
  });
});

describe('phone ⋯ rows (ideActions)', () => {
  // Every runtime import is a no-op stub: the test reads which rows the ⋯ builds.
  const stub: Record<string, unknown> = new Proxy({}, { get: () => () => undefined });
  const ide = { hasTarget: true, activePath: '/repo/main.go', target: { kind: 'local', root: '/repo' }, keymapPreset: 'jetbrains', bottomDockOpen: false, buildToolKind: null };
  const mod = loadSource(new URL('../src/modules/ide/shell/ideActions.ts', import.meta.url), {
    '../../../lib/stores/workspace.svelte': { SCRATCH_WORKSPACE_ID: 'scratch', ws: { workspaces: [] } },
    '../../../lib/toast.svelte': { toasts: stub },
    '../editor/ideEditorStoreBridge.svelte': stub,
    '../editor/idePreviewMode.svelte': { idePreviewMode: stub },
    '../ideStore.svelte': { ide },
    '../search/ideQuickOpen.svelte': stub,
    '../store/ideAgentController.svelte': { ideAgent: { panelOpen: false } },
    './ideProjectRecents.svelte': stub,
    './ideShellCommands': stub,
    './ideRunTargetMenu': { runTargetRowLabel: () => 'Run target: Go Run app', runTargetMenuItems: () => [{ label: 'Go Run app', checked: true }] },
    './ideShortcuts': { ideKeymapPresetOf: () => 'jetbrains', ideShortcutLabel: () => '⌘X' },
  }, { window: { matchMedia: () => ({ matches: true }) } });
  const labels = (items: { label?: string; separator?: boolean; children?: unknown[] }[]): string[] =>
    items.filter((i) => !i.separator).map((i) => i.label!);
  const noop = () => {};
  const phone = mod.ideMoreMenuFromActions({ stack: true, onShowPane: noop, onTriggerAgent: noop, onAttachFolder: noop, onOpenScratches: noop });
  const desk = mod.ideMoreMenuFromActions({ stack: false, onTriggerAgent: noop, onAttachFolder: noop, onToggleExplorer: noop, explorerVisible: true });

  it('the phone ⋯ starts the Run group with "Run target: …" opening the targets', () => {
    const row = phone.find((i: { label?: string }) => i.label?.startsWith('Run target'));
    assert.ok(row, labels(phone).join(', '));
    assert.equal(row.label, 'Run target: Go Run app');
    assert.deepEqual(plain(row.children), [{ label: 'Go Run app', checked: true }]);
    assert.equal(row.action, undefined, 'a submenu row, not an action');
    assert.ok(labels(phone).indexOf('Run target: Go Run app') < labels(phone).indexOf('Debug current configuration'));
    assert.ok(!labels(desk).some((l) => l.startsWith('Run target')), 'the workbench has the toolbar selector');
  });

  it('the phone ⋯ drops what the pane switcher has and has no Tool windows submenu', () => {
    const all = JSON.stringify(phone);
    assert.doesNotMatch(all, /"AI agent"|"Git changes"/);
    assert.ok(!labels(phone).includes('Tool windows'));
    assert.ok(labels(phone).includes('Live DB watch') && labels(phone).includes('Scratches'), labels(phone).join(', '));
    // The workbench keeps them, in its Tool windows submenu.
    const tools = desk.find((i: { label?: string }) => i.label === 'Tool windows') as { children: { label: string }[] };
    assert.ok(tools.children.some((c) => c.label === 'Git changes'));
    assert.ok(tools.children.some((c) => /AI agent/.test(c.label)));
  });

  it('no shortcut hints on a touch screen', () => {
    const flat = (items: { hint?: string; children?: unknown[] }[]): { hint?: string }[] =>
      items.flatMap((i) => [i, ...flat((i.children ?? []) as { hint?: string }[])]);
    assert.ok(flat(phone).every((i) => !i.hint));
  });
});
