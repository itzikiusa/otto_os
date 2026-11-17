// IDE shortcut truth (review I1-10 / I2-06): every key the IDE shows — in Settings ›
// Keymap, a ⋯ / ⌘K / quick-open hint or a debugger tooltip — comes from ONE table
// (shell/ideShortcuts.ts), and every chord in that table, per preset, really runs
// something: window rows through the shell key handler, editor rows through the
// CodeMirror keymap, the double-⇧ row through IdePage's detector.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { buildIdeKeymapSpecs } from '../src/lib/components/cm-ide-keymap.ts';
import * as shortcutsModule from '../src/modules/ide/shell/ideShortcuts.ts';
import {
  IDE_SHORTCUTS,
  ideChordLabel,
  ideShortcutLabel,
  ideShortcutLabels,
  type IdeChord,
  type IdeKeymapPreset,
  type IdeShortcutRow,
} from '../src/modules/ide/shell/ideShortcuts.ts';
import { loadSource } from './sourceHarness.ts';

const ROWS = IDE_SHORTCUTS as readonly IdeShortcutRow[];
const PRESETS: IdeKeymapPreset[] = ['jetbrains', 'standard'];
const src = (rel: string) => readFileSync(new URL(`../src/${rel}`, import.meta.url), 'utf8');

class FakeEl {
  tagName: string;
  classes: string[];
  parent: FakeEl | null;
  isContentEditable = false;
  constructor(tagName: string, classes: string[] = [], parent: FakeEl | null = null) {
    this.tagName = tagName; this.classes = classes; this.parent = parent;
  }
  matches(sel: string) { return sel.split(',').some((s) => this.classes.includes(s.trim().replace(/^\./, ''))); }
  closest(sel: string): FakeEl | null { for (let e: FakeEl | null = this; e; e = e.parent) if (e.matches(sel)) return e; return null; }
  querySelector(): FakeEl | null { return null; }
}

function shell(preset: IdeKeymapPreset, session: boolean) {
  const calls: string[] = [];
  const record = (name: string) => (..._args: unknown[]) => { calls.push(name); return Promise.resolve(); };
  const data: Record<string, unknown> = {
    activePath: '/repo/main.go', cursorInfo: { line: 3, col: 1 }, keymapPreset: preset,
    activeDebug: session ? { state: 'paused' } : null, searchEverywhereOpen: false,
    canNavigateBack: true, canNavigateForward: true, target: { root: '/repo' },
  };
  const ide = new Proxy(data, { get: (t, k: string) => (k in t ? t[k] : record(k)) });
  const mod = loadSource(new URL('../src/modules/ide/shell/ideShellCommands.ts', import.meta.url), {
    '../../../lib/idePreferences.svelte': { readIdePreferences: () => ({}), writeIdePreferences: () => {} },
    '../ideStore.svelte': { ide },
    '../search/ideQuickOpen.svelte': {
      openGoToLine: record('openGoToLine'), openGoToFile: record('openGoToFile'),
      openGoToSymbol: record('openGoToSymbol'), openRecentFiles: record('openRecentFiles'),
    },
    '../store/ideAgentController.svelte': { ideAgent: { markSelectionFromEditor: record('markSelection'), openInlineEdit: record('openInlineEdit') } },
    './ideShortcuts': shortcutsModule,
  }, { Element: FakeEl, window: { location: { reload: record('reload') }, dispatchEvent: record('dispatchEvent') }, CustomEvent: class {} });
  return { mod, calls, data };
}

/** Rows that run only while a debug session is live. */
const SESSION_ROWS = new Set(['restartDebug', 'resume', 'stepOver', 'stepInto', 'stepOut', 'runToCursor']);

const CODE: Record<string, string> = { ',': 'Comma', '.': 'Period', '/': 'Slash', '`': 'Backquote', '-': 'Minus', '[': 'BracketLeft', ']': 'BracketRight' };

function press(c: IdeChord, target: FakeEl) {
  let prevented = false;
  const code = /^[a-z]$/i.test(c.key) ? `Key${c.key.toUpperCase()}` : /^\d$/.test(c.key) ? `Digit${c.key}` : CODE[c.key] ?? c.key;
  return {
    target, key: c.shift && c.key === '-' ? '_' : c.key, code,
    metaKey: Boolean(c.mod || c.meta), ctrlKey: Boolean(c.ctrl), altKey: Boolean(c.alt), shiftKey: Boolean(c.shift),
    get defaultPrevented() { return prevented; }, preventDefault() { prevented = true; },
  };
}

/** Press a window chord the way the page does: ⌘ chords through keys.ts's
 *  `pageChords` claim (handleIdeChord), the rest through the window listener. */
function dispatch(preset: IdeKeymapPreset, row: IdeShortcutRow, chord: IdeChord) {
  const { mod, calls } = shell(preset, SESSION_ROWS.has(row.id));
  const page = new FakeEl('DIV', ['ide-page']);
  const e = press(chord, page);
  if (chord.mod || chord.meta) mod.handleIdeChord(e, false, () => calls.push('toggleExplorer'), () => calls.push('triggerAgent'));
  else mod.handleIdeWindowKeyDown(e, false, () => calls.push('triggerAgent'));
  return { handled: e.defaultPrevented, calls, mod };
}

test('every window chord of every preset has a handler that runs', () => {
  const { mod } = shell('jetbrains', false);
  for (const preset of PRESETS) {
    for (const row of ROWS.filter((r) => r.scope === 'window')) {
      assert.ok(mod.WINDOW_COMMANDS[row.id], `${row.id}: a window row needs a WINDOW_COMMANDS entry`);
      for (const chord of preset === 'standard' ? row.standard : row.jetbrains) {
        const { handled, calls } = dispatch(preset, row, chord);
        assert.ok(handled, `${preset} ${ideChordLabel(chord)} (${row.id}) was not handled`);
        assert.ok(calls.length > 0, `${preset} ${ideChordLabel(chord)} (${row.id}) ran nothing`);
      }
    }
  }
});

test('every editor chord of every preset is bound in the CodeMirror keymap', () => {
  const norm = (k: string) => {
    const parts = k.split('-');
    const key = parts.pop()!.toLowerCase();
    return [...parts.map((p) => (p === 'Cmd' || p === 'Meta' ? 'Mod' : p)).sort(), key].join('-');
  };
  const fromChord = (c: IdeChord) => norm([c.mod && 'Mod', c.ctrl && 'Ctrl', c.alt && 'Alt', c.shift && 'Shift', c.key].filter(Boolean).join('-'));
  for (const preset of PRESETS) {
    const bound = new Set(buildIdeKeymapSpecs(preset, () => ({})).map((s) => norm(String(s.key))));
    for (const row of ROWS.filter((r) => r.scope === 'editor')) {
      for (const chord of preset === 'standard' ? row.standard : row.jetbrains) {
        assert.ok(bound.has(fromChord(chord)), `${preset} ${ideChordLabel(chord)} (${row.id}) is not in the editor keymap`);
      }
    }
  }
});

// IdePage's keyup detector handles the one page row; e2e/ide.spec.ts presses ⇧⇧.
test('the only page row is double-⇧, rendered as "Double ⇧"', () => {
  const pageRows = ROWS.filter((r) => r.scope === 'page');
  assert.deepEqual(pageRows.map((r) => r.id), ['searchEverywhere']);
  assert.equal(ideShortcutLabel('searchEverywhere', 'jetbrains'), 'Double ⇧');
  assert.equal(ideShortcutLabel('searchEverywhere', 'standard'), 'Double ⇧');
});

test('Standard preset: ⌘P / ⌘B / ⌘J / ⌃` and the debug F-keys do what Settings says', () => {
  const run = (id: string, session = false) => {
    const row = ROWS.find((r) => r.id === id)!;
    const { mod, calls } = shell('standard', session);
    const e = press(row.standard[0], new FakeEl('DIV'));
    if (row.standard[0].mod) mod.handleIdeChord(e, false, () => calls.push('toggleExplorer'), () => {});
    else mod.handleIdeWindowKeyDown(e, false, () => {});
    return calls;
  };
  assert.deepEqual(run('goToFile'), ['openGoToFile']);
  assert.deepEqual(run('toggleExplorer'), ['toggleExplorer']);
  assert.deepEqual(run('toggleDock'), ['toggleBottomDock']);
  assert.deepEqual(run('terminal'), ['openBottomTab']);
  assert.deepEqual(run('replaceInPath'), ['openSearchEverywhere']);
  assert.deepEqual(run('debug'), ['startDebug'], 'F5 with nothing running starts Debug');
  assert.deepEqual(run('resume', true), ['debugCommand'], 'F5 while paused resumes');
  assert.deepEqual(run('stop'), ['stopActiveExecution']);
  assert.deepEqual(run('restartDebug', true), ['restartDebug']);
  assert.deepEqual(run('toggleBreakpoint'), ['toggleBreakpoint']);
  assert.deepEqual(run('stepOver', true), ['stepOver']);
  assert.deepEqual(run('stepInto', true), ['stepInto']);
  assert.deepEqual(run('stepOut', true), ['stepOut']);
});

test('a preset only claims its own keys: JetBrains ⌘J / ⌘B fall through to the app and the editor', () => {
  const { mod, calls } = shell('jetbrains', false);
  for (const key of ['j', 'b']) {
    const e = press({ key, mod: true }, new FakeEl('DIV'));
    assert.equal(mod.handleIdeChord(e, false, () => calls.push('toggleExplorer'), () => {}), false, `⌘${key.toUpperCase()}`);
  }
  assert.deepEqual(calls, []);
});

test('⌘P never fires from a terminal; it retargets the quick-open while that is open', () => {
  const { mod, calls } = shell('jetbrains', false);
  const term = new FakeEl('DIV', ['xterm']);
  const fromTerm = press({ key: 'p', mod: true }, term);
  mod.handleIdeWindowKeyDown(fromTerm, false, () => {});
  assert.equal(fromTerm.defaultPrevented, false, 'the window phase leaves ⌘ chords to keys.ts, which skips a terminal');
  assert.equal(mod.handleIdeChord(press({ key: 'p', mod: true }, new FakeEl('DIV')), true, () => {}), false, 'under another dialog');
  assert.deepEqual(calls, []);
  const open = shell('jetbrains', false);
  open.data.searchEverywhereOpen = true;
  assert.equal(open.mod.handleIdeChord(press({ key: 'g', mod: true }, new FakeEl('INPUT')), true, () => {}), true);
  assert.deepEqual(open.calls, ['openGoToLine'], '⌘G with the picker up switches it to `:` mode');
});

test('within a preset no two window commands share a chord (except a live-session one)', () => {
  for (const preset of PRESETS) {
    const seen = new Map<string, string>();
    for (const row of ROWS.filter((r) => r.scope === 'window' && !SESSION_ROWS.has(r.id))) {
      for (const chord of preset === 'standard' ? row.standard : row.jetbrains) {
        const label = ideChordLabel(chord);
        assert.equal(seen.get(label), undefined, `${preset} ${label}: ${seen.get(label)} and ${row.id}`);
        seen.set(label, row.id);
      }
    }
  }
});

// Settings › Keymap renders IDE_SHORTCUTS (e2e/desktop-ide-quickopen.spec.ts counts its rows).
test('hints are never hard-coded: menus and the action list take them from the table', () => {
  const actions = src('modules/ide/shell/ideActions.ts');
  const ids = new Set(ROWS.map((r) => r.id));
  for (const m of actions.matchAll(/shortcut: '(\w+)'/g)) assert.ok(ids.has(m[1]), `ideActions shortcut '${m[1]}' is not in IDE_SHORTCUTS`);
  const glyph = /'[^'\n]*[⌘⌃⌥⇧][^'\n]*'/;
  for (const file of ['modules/ide/shell/ideActions.ts', 'modules/ide/shell/ideShellMenus.ts', 'modules/ide/git/ideGitQuickMenu.ts']) {
    const body = src(file).split('\n').filter((l) => !l.trim().startsWith('//') && !l.trim().startsWith('*'));
    for (const line of body) assert.doesNotMatch(line, glyph, `${file}: a literal shortcut string → ${line.trim()}`);
  }
});

test('labels: macOS modifier order and every chord listed', () => {
  assert.equal(ideShortcutLabel('goToFile', 'jetbrains'), '⌘P');
  assert.equal(ideShortcutLabels('goToFile', 'jetbrains'), '⌘P / ⇧⌘O');
  assert.equal(ideShortcutLabel('restartDebug', 'standard'), '⇧⌘F5');
  assert.equal(ideShortcutLabels('restartDebug', 'jetbrains'), '—');
  assert.equal(ideShortcutLabel('restartDebug', 'jetbrains'), undefined, 'no hint for a key the preset lacks');
  assert.equal(ideShortcutLabel('terminal', 'standard'), '⌃`');
  assert.equal(ideShortcutLabel('stepOver', 'jetbrains'), 'F8');
  assert.equal(ideShortcutLabels('stepOver', 'jetbrains'), 'F8 / ⌃⌥↓');
});
