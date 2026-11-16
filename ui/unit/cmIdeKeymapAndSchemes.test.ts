import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, it } from 'node:test';
import { EditorSelection, EditorState, Text, type SelectionRange } from '@codemirror/state';
import { parseManifestDependencies } from '../src/lib/components/cm-ide-deps.ts';
import {
  buildIdeKeymapSpecs,
  duplicateLineOrSelection,
  lspLanguageIdFor,
  normalizeIdeLspServerLang,
  smartExpandSelection,
  splitSelectionIntoLines,
  startNewLineBelow,
  wordAtCursor,
} from '../src/lib/components/cm-ide-keymap.ts';
import {
  DEFAULT_IDE_PREFERENCES,
  IDE_COLOR_SCHEME_OPTIONS,
  IDE_PREFS_STORAGE_KEY,
  isDarkIdeScheme,
  readIdePreferences,
  resolveEffectiveIdeScheme,
  writeIdePreferences,
  type ConcreteIdeColorSchemeId,
} from '../src/lib/components/cm-ide-prefs.ts';
import { hasDeterministicQuickFixAtLine } from '../src/lib/components/cm-ide-quickfix.ts';
import { getCachedSemanticDoc } from '../src/lib/components/cm-ide-semantic-model.ts';

class MemoryStorage {
  private map = new Map<string, string>();
  get length(): number {
    return this.map.size;
  }
  key(index: number): string | null {
    return [...this.map.keys()][index] ?? null;
  }
  getItem(k: string): string | null {
    return this.map.get(k) ?? null;
  }
  setItem(k: string, v: string): void {
    this.map.set(k, String(v));
  }
  removeItem(k: string): void {
    this.map.delete(k);
  }
  clear(): void {
    this.map.clear();
  }
}

function makeFakeView(initialDoc: string, selection: EditorSelection | SelectionRange) {
  let state = EditorState.create({
    doc: initialDoc,
    selection,
    extensions: [EditorState.allowMultipleSelections.of(true)],
  });
  const fakeView = {
    get state() {
      return state;
    },
    dispatch(spec: Parameters<EditorState['update']>[0]) {
      state = state.update(spec).state;
    },
    coordsAtPos() {
      return { left: 42, top: 20, right: 48, bottom: 38 };
    },
  };
  return fakeView;
}

describe('cm-ide-keymap: JetBrains vs Standard presets & editing commands', () => {
  it('exposes JetBrains vs Standard preset bindings and dispatches IDE callbacks', () => {
    const calls: string[] = [];
    const callbacks = {
      onNavigateBack: () => calls.push('back'),
      onNavigateForward: () => calls.push('forward'),
      onPasteFromHistory: () => calls.push('pasteHistory'),
      onRenameSymbol: ({ line, column, symbol }: { line: number; column: number; symbol: string }) =>
        calls.push(`rename:${symbol}@${line}:${column}`),
      onToggleBreakpoint: (line: number) => calls.push(`bp:${line}`),
      onQuickFix: ({ line, symbol }: { line: number; symbol?: string }) =>
        calls.push(`qf:${symbol ?? ''}@${line}`),
    };

    const jbSpecs = buildIdeKeymapSpecs('jetbrains', () => callbacks, '/repo/main.go', 'go');
    const stdSpecs = buildIdeKeymapSpecs('standard', () => callbacks, '/repo/main.go', 'go');

    const jbKeys = new Set(jbSpecs.map((b) => b.key));
    const stdKeys = new Set(stdSpecs.map((b) => b.key));

    // JetBrains-specific bindings
    assert.equal(jbKeys.has('Mod-['), true);
    assert.equal(jbKeys.has('Mod-]'), true);
    assert.equal(jbKeys.has('Mod-d'), true);
    assert.equal(jbKeys.has('Mod-Backspace'), true);
    assert.equal(jbKeys.has('Shift-Mod-k'), false);

    // Standard-specific bindings
    assert.equal(stdKeys.has('Mod-d'), true);
    assert.equal(stdKeys.has('Shift-Mod-d'), true);
    assert.equal(stdKeys.has('Shift-Mod-k'), true);
    assert.equal(stdKeys.has('Mod-['), false);
    assert.equal(stdKeys.has('Mod-]'), false);

    // Both presets register paste-from-history for lowercase & uppercase Shift+V
    for (const k of ['Mod-Shift-v', 'Mod-Shift-V', 'Cmd-Shift-v', 'Cmd-Shift-V']) {
      assert.equal(jbKeys.has(k), true, `missing ${k}`);
      assert.equal(stdKeys.has(k), true, `missing ${k}`);
    }

    const view = makeFakeView(
      'package main\nfunc ComputeBonus() {}\n',
      EditorSelection.cursor('package main\nfunc Compute'.length),
    );

    const byKey = (specs: typeof jbSpecs, key: string) => specs.find((s) => s.key === key)!;
    assert.equal(byKey(jbSpecs, 'Mod-[').run!(view as never), true);
    assert.equal(byKey(jbSpecs, 'Mod-]').run!(view as never), true);
    assert.equal(byKey(jbSpecs, 'Mod-Shift-V').run!(view as never), true);
    assert.equal(byKey(jbSpecs, 'Shift-F6').run!(view as never), true);
    assert.equal(byKey(jbSpecs, 'Mod-F8').run!(view as never), true);
    assert.equal(byKey(jbSpecs, 'Alt-Enter').run!(view as never), true);

    assert.deepEqual(calls, [
      'back',
      'forward',
      'pasteHistory',
      'rename:ComputeBonus@2:13',
      'bp:2',
      'qf:ComputeBonus@2',
    ]);

    // In JetBrains preset, Mod-d duplicates the current line; in Standard, Shift-Mod-d duplicates
    assert.equal(byKey(jbSpecs, 'Mod-d').run, duplicateLineOrSelection);
    assert.equal(byKey(stdSpecs, 'Shift-Mod-d').run, duplicateLineOrSelection);
    assert.notEqual(byKey(stdSpecs, 'Mod-d').run, duplicateLineOrSelection);
  });

  it('executes duplicateLineOrSelection, smartExpandSelection, splitSelectionIntoLines, startNewLineBelow & wordAtCursor', () => {
    // 1. wordAtCursor & smartExpandSelection
    const v1 = makeFakeView('const totalBonus = 42;\n', EditorSelection.cursor(8));
    assert.deepEqual(wordAtCursor(v1.state), { word: 'totalBonus', line: 1, column: 9 });
    assert.equal(smartExpandSelection(v1 as never), true);
    assert.equal(
      v1.state.sliceDoc(v1.state.selection.main.from, v1.state.selection.main.to),
      'totalBonus',
    );

    // 2. duplicateLineOrSelection (selected range)
    assert.equal(duplicateLineOrSelection(v1 as never), true);
    assert.equal(v1.state.doc.toString(), 'const totalBonustotalBonus = 42;\n');

    // 3. splitSelectionIntoLines across 3 lines
    const doc3 = 'alpha\nbeta\ngamma';
    const v2 = makeFakeView(doc3, EditorSelection.range(0, doc3.length));
    assert.equal(splitSelectionIntoLines(v2 as never), true);
    assert.equal(v2.state.selection.ranges.length, 3);
    assert.deepEqual(
      v2.state.selection.ranges.map((r) => v2.state.sliceDoc(r.from, r.to)),
      ['alpha', 'beta', 'gamma'],
    );

    // 4. startNewLineBelow preserves leading indentation
    const v3 = makeFakeView('  if ok {\n    run()\n  }', EditorSelection.cursor(14));
    assert.equal(startNewLineBelow(v3 as never), true);
    assert.equal(v3.state.doc.line(3).text, '    ');
    assert.equal(v3.state.doc.lineAt(v3.state.selection.main.head).number, 3);
  });
});

describe('cm-ide-prefs: color schemes & canonical preferences normalization', () => {
  const prevDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  let mem: MemoryStorage;

  beforeEach(() => {
    mem = new MemoryStorage();
    Object.defineProperty(globalThis, 'localStorage', {
      value: mem,
      configurable: true,
      writable: true,
    });
  });

  afterEach(() => {
    if (prevDescriptor) {
      Object.defineProperty(globalThis, 'localStorage', prevDescriptor);
    }
  });

  it('resolves all 5 concrete schemes (including VS Code Dark/Light Modern) and auto mode', () => {
    const expectedDark: Record<ConcreteIdeColorSchemeId, boolean> = {
      'jetbrains-islands-dark': true,
      'jetbrains-darcula': true,
      'vscode-dark-modern': true,
      'jetbrains-islands-light': false,
      'vscode-light-modern': false,
    };

    const concreteIds = IDE_COLOR_SCHEME_OPTIONS.filter((o) => o.id !== 'auto').map(
      (o) => o.id as ConcreteIdeColorSchemeId,
    );
    assert.deepEqual(concreteIds.sort(), Object.keys(expectedDark).sort());

    for (const scheme of concreteIds) {
      assert.equal(isDarkIdeScheme(scheme), expectedDark[scheme], `isDarkIdeScheme(${scheme})`);
      assert.equal(resolveEffectiveIdeScheme({ colorScheme: scheme }, 'light'), scheme);
      assert.equal(resolveEffectiveIdeScheme({ colorScheme: scheme }, 'dark'), scheme);
    }

    assert.equal(
      resolveEffectiveIdeScheme(
        {
          colorScheme: 'auto',
          preferredDarkScheme: 'vscode-dark-modern',
          preferredLightScheme: 'vscode-light-modern',
        },
        'dark',
      ),
      'vscode-dark-modern',
    );
    assert.equal(
      resolveEffectiveIdeScheme(
        {
          colorScheme: 'auto',
          preferredDarkScheme: 'vscode-dark-modern',
          preferredLightScheme: 'vscode-light-modern',
        },
        'light',
      ),
      'vscode-light-modern',
    );
  });

  it('normalizes legacy preference aliases and preserves unrelated fields across writes', () => {
    mem.setItem(
      IDE_PREFS_STORAGE_KEY,
      JSON.stringify({
        darkScheme: 'vscode-dark-modern',
        lightScheme: 'vscode-light-modern',
        showInlayHints: false,
        gitStatusInTree: false,
        consolePrettyJson: false,
        localHistoryRetentionDays: 30,
        dbgShowFrames: false,
      }),
    );

    const loaded = readIdePreferences();
    assert.equal(loaded.preferredDarkScheme, 'vscode-dark-modern');
    assert.equal(loaded.darkColorScheme, 'vscode-dark-modern');
    assert.equal(loaded.preferredLightScheme, 'vscode-light-modern');
    assert.equal(loaded.lightColorScheme, 'vscode-light-modern');
    assert.equal(loaded.inlayHints, false);
    assert.equal(loaded.gitTreeBadges, false);
    assert.equal(loaded.prettyJsonLogs, false);
    assert.equal(loaded.localHistoryRetentionDays, 30);
    assert.equal(loaded.dbgShowFrames, false);
    assert.equal(loaded.fontSize, DEFAULT_IDE_PREFERENCES.fontSize);

    // Partial write from editor/dock must keep settings fields intact and strip stale alias keys
    const updated = writeIdePreferences({ wordWrap: true, preferredDarkScheme: 'jetbrains-darcula' });
    assert.equal(updated.wordWrap, true);
    assert.equal(updated.preferredDarkScheme, 'jetbrains-darcula');
    assert.equal(updated.darkColorScheme, 'jetbrains-darcula');
    assert.equal(updated.localHistoryRetentionDays, 30);
    assert.equal(updated.dbgShowFrames, false);

    const rawStored = JSON.parse(mem.getItem(IDE_PREFS_STORAGE_KEY)!) as Record<string, unknown>;
    assert.equal('darkScheme' in rawStored, false);
    assert.equal('gitStatusInTree' in rawStored, false);
  });
});

describe('LSP normalization, manifest single-line deps, and semantic/quick-fix caching', () => {
  it('normalizes LSP language IDs to backend server registry keys (WIRE-1)', () => {
    assert.equal(lspLanguageIdFor('/repo/deploy.sh', true), 'shellscript');
    assert.equal(normalizeIdeLspServerLang('shellscript'), 'shell');
    assert.equal(normalizeIdeLspServerLang('bash'), 'shell');
    assert.equal(lspLanguageIdFor('/repo/View.tsx', true), 'typescriptreact');
    assert.equal(normalizeIdeLspServerLang('typescriptreact'), 'typescript');
    assert.equal(lspLanguageIdFor('/repo/Widget.jsx', true), 'javascriptreact');
    assert.equal(normalizeIdeLspServerLang('javascriptreact'), 'javascript');
    assert.equal(normalizeIdeLspServerLang('scss'), 'css');
    assert.equal(normalizeIdeLspServerLang('less'), 'css');
    assert.equal(normalizeIdeLspServerLang('jsonc'), 'json');
    assert.equal(normalizeIdeLspServerLang('jsonl'), 'json');
    assert.equal(normalizeIdeLspServerLang('go'), 'go');
  });

  it('does not swallow root package.json keys after a single-line `"dependencies": {}` (INPUT-3)', () => {
    const pkgJson = [
      '{',
      '  "name": "demo-pkg",',
      '  "dependencies": {},',
      '  "version": "2.4.0",',
      '  "license": "MIT",',
      '  "devDependencies": {',
      '    "typescript": "^5.7.0"',
      '  }',
      '}',
    ].join('\n');

    const deps = parseManifestDependencies('/repo/package.json', pkgJson);
    assert.equal(deps.length, 1);
    assert.equal(deps[0].name, 'typescript');
    assert.equal(deps[0].scope, 'devDependencies');
  });

  it('reuses WeakMap semantic document cache across repeated reads of the same Text (PERF-2)', () => {
    const doc = Text.of([
      'package main',
      'import "os"',
      'func Run() error {',
      '  err := os.Remove("tmp")',
      '  return err',
      '}',
    ]);
    const first = getCachedSemanticDoc(doc, 'go', undefined, undefined, true, true);
    const second = getCachedSemanticDoc(doc, 'go', undefined, undefined, true, true);
    assert.equal(first.model, second.model);
    assert.equal(first.brackets, second.brackets);
    assert.equal(first.identCounts, second.identCounts);

    // Line-level quick-fix detector flags unchecked `err :=` line without full-document stringify
    assert.equal(hasDeterministicQuickFixAtLine(doc, '/repo/main.go', 'go', 4), true);
    assert.equal(hasDeterministicQuickFixAtLine(doc, '/repo/main.go', 'go', 1), false);
  });
});
