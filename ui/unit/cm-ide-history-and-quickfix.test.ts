import assert from 'node:assert/strict';
import { test } from 'node:test';
import { history, undo } from '@codemirror/commands';
import { EditorState } from '@codemirror/state';
import {
  clearIdeClipboardHistory,
  forgetIdeUndoJournal,
  isBpTargetGutterElement,
  persistIdeUndoJournal,
  readIdeClipboardHistory,
  recordIdeClipboardEntry,
  restoreIdeStateFromJournal,
} from '../src/lib/components/cm-ide-history.ts';
import {
  computeDeterministicQuickFixes,
  previewConcreteEdits,
} from '../src/lib/components/cm-ide-quickfix.ts';
import {
  applyBatchFallbackManifestUpdates,
  mergeDepItems,
  normalizeIdeDepAction,
  normalizeRemoteDepItem,
  parseManifestDependencies,
} from '../src/lib/components/cm-ide-deps.ts';
import { sanitizeAndSplitHoverDoc } from '../src/lib/components/cm-ide-hover.ts';
import { analyzeJavaDiagnostics } from '../src/lib/components/cm-ide-semantic.ts';
import {
  IDE_PREFS_STORAGE_KEY,
  readIdeEditorPreferences,
  writeIdeEditorPreferences,
} from '../src/lib/components/cm-ide-theme.ts';
import {
  clearIdeSensitiveWebStorage,
  IDE_CURSOR_POS_KEY,
  IDE_UNDO_JOURNAL_KEY,
  isSecretIdePath,
} from '../src/lib/components/cm-ide-drafts.ts';

function createMemoryStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() { return map.size; },
    clear() { map.clear(); },
    getItem(k: string) { return map.get(k) ?? null; },
    key(idx: number) { return Array.from(map.keys())[idx] ?? null; },
    removeItem(k: string) { map.delete(k); },
    setItem(k: string, v: string) { map.set(k, v); },
  };
}

function runUndoOnState(state: EditorState): EditorState {
  let next = state;
  const applied = undo({
    state,
    dispatch: (tr) => { next = tr.state; },
  });
  assert.equal(applied, true, 'expected CodeMirror undo() command to apply');
  return next;
}

test('Durable Undo journal preserves CodeMirror ⌘Z undo stack across simulated reload/restart and after ⌘S save', () => {
  const storage = createMemoryStorage();
  const exts = [history({ minDepth: 200, newGroupDelay: 0 })];
  const filePath = '/ws/service/handler.go';

  const v0 = 'package service\n\nfunc Handle() string {\n\treturn "v0"\n}\n';
  let st = EditorState.create({ doc: v0, extensions: exts });

  // Edit 1 -> "v1"
  st = st.update({ changes: { from: st.doc.toString().indexOf('"v0"'), to: st.doc.toString().indexOf('"v0"') + 4, insert: '"v1"' } }).state;
  // Edit 2 -> "v2" and simulate `⌘S` save (`savedContent = st.doc.toString()`)
  st = st.update({ changes: { from: st.doc.toString().indexOf('"v1"'), to: st.doc.toString().indexOf('"v1"') + 4, insert: '"v2"' } }).state;
  const savedOnDisk = st.doc.toString();
  persistIdeUndoJournal(filePath, st, savedOnDisk, 128, storage);

  // Simulate full Otto.app restart / page reload: restore from `localStorage` journal!
  const restoredSaved = restoreIdeStateFromJournal(filePath, savedOnDisk, exts, storage);
  assert.ok(restoredSaved);
  assert.equal(restoredSaved.scrollTop, 128);
  assert.equal(restoredSaved.restoredDraft, null);
  assert.equal(restoredSaved.state.doc.toString(), savedOnDisk);

  // Pressing `⌘Z` twice after reload must step cleanly back through "v1" -> "v0"!
  const afterUndo1 = runUndoOnState(restoredSaved.state);
  assert.ok(afterUndo1.doc.toString().includes('"v1"'));
  const afterUndo2 = runUndoOnState(afterUndo1);
  assert.equal(afterUndo2.doc.toString(), v0);

  // Also verify unsaved draft recovery when closed dirty before restart:
  const dirtyState = st.update({
    changes: { from: st.doc.toString().indexOf('"v2"'), to: st.doc.toString().indexOf('"v2"') + 4, insert: '"dirty-v3"' },
  }).state;
  persistIdeUndoJournal(filePath, dirtyState, savedOnDisk, 64, storage);

  const restoredDirty = restoreIdeStateFromJournal(filePath, savedOnDisk, exts, storage);
  assert.ok(restoredDirty);
  assert.ok(restoredDirty.restoredDraft?.includes('"dirty-v3"'));
  const revertedFromDirty = runUndoOnState(restoredDirty.state);
  assert.equal(revertedFromDirty.doc.toString(), savedOnDisk);

  forgetIdeUndoJournal(filePath, storage);
  assert.equal(restoreIdeStateFromJournal(filePath, savedOnDisk, exts, storage), null);
});

test('Persistent 30-item Clipboard History deduplicates repeats, caps at 30 entries, and clears cleanly', () => {
  const storage = createMemoryStorage();
  for (let i = 1; i <= 35; i++) {
    recordIdeClipboardEntry(`snippet_${i}\nline_two_${i}`, `/ws/file_${i}.ts`, storage);
  }
  const ring = readIdeClipboardHistory(storage);
  assert.equal(ring.length, 30, 'clipboard history ring must cap at 30 items');
  assert.equal(ring[0].text, 'snippet_35\nline_two_35');
  assert.equal(ring[0].lineCount, 2);

  // Re-copying an older entry moves it to index 0 without duplicating!
  recordIdeClipboardEntry('snippet_20\nline_two_20', '/ws/file_20.ts', storage);
  const deduped = readIdeClipboardHistory(storage);
  assert.equal(deduped.length, 30);
  assert.equal(deduped[0].text, 'snippet_20\nline_two_20');
  assert.equal(deduped.filter((x) => x.text === 'snippet_20\nline_two_20').length, 1);

  clearIdeClipboardHistory(storage);
  assert.equal(readIdeClipboardHistory(storage).length, 0);
});

test('isBpTargetGutterElement enables 1-click breakpoints everywhere in .cm-gutters except explicit action buttons', () => {
  function fakeGutterNode(matchedSelector: string | null, insideGutters = true): EventTarget {
    const el = {
      closest(sel: string): object | null {
        if (sel === '.cm-gutters') return insideGutters ? {} : null;
        if (matchedSelector && sel.split(',').map((s) => s.trim()).includes(matchedSelector)) return {};
        return null;
      },
    };
    // Make `instanceof Element` check work without browser DOM if Element is undefined in Node.
    if (typeof Element !== 'undefined') Object.setPrototypeOf(el, Element.prototype);
    return el as unknown as EventTarget;
  }

  if (typeof Element === 'undefined') {
    (globalThis as Record<string, unknown>).Element = class MockElement {};
  }

  assert.equal(isBpTargetGutterElement(fakeGutterNode(null, true)), true, 'clicking line number or bp gutter toggles bp');
  assert.equal(isBpTargetGutterElement(fakeGutterNode(null, false)), false, 'clicking outside .cm-gutters never toggles bp');
  assert.equal(isBpTargetGutterElement(fakeGutterNode('.cm-ide-run-btn', true)), false, 'clicking ▶ run icon does not toggle bp');
  assert.equal(isBpTargetGutterElement(fakeGutterNode('.cm-fold-marker', true)), false, 'clicking fold chevron does not toggle bp');
  assert.equal(isBpTargetGutterElement(fakeGutterNode('.cm-ide-lightbulb-btn', true)), false, 'clicking 💡 lightbulb opens quick-fix instead');
  assert.equal(isBpTargetGutterElement(fakeGutterNode('.cm-ide-dep-gutter-btn', true)), false, 'clicking 🎯↑ parent POM jump does not toggle bp');
});

test('Deterministic Quick Fixes generate concrete text edits and live diff previews across Java, Go, Rust, JSON, and Shell', () => {
  // 1. Java missing import (`List`)
  const javaDoc = 'package demo;\n\npublic class A {\n    List<String> items;\n}\n';
  const javaFixes = computeDeterministicQuickFixes(javaDoc, '/ws/A.java', 'java', 4, 'List');
  const importList = javaFixes.find((f) => f.title === "Import 'java.util.List'");
  assert.ok(importList);
  const javaPreview = previewConcreteEdits(javaDoc, importList.edits);
  assert.ok(javaPreview.nextText.includes('import java.util.List;'));
  assert.ok(javaPreview.afterSnippet.includes('import java.util.List;'));

  // 2. Go missing stdlib import (`fmt.Println`) + `if err != nil`
  const goDoc = 'package main\n\nfunc main() {\n\tfmt.Println("hi")\n\terr := run()\n}\n';
  const goImportFix = computeDeterministicQuickFixes(goDoc, '/ws/main.go', 'go', 4, 'fmt').find((f) => f.title === 'Import "fmt"');
  assert.ok(goImportFix);
  assert.ok(previewConcreteEdits(goDoc, goImportFix.edits).nextText.includes('import "fmt"'));

  const goErrFix = computeDeterministicQuickFixes(goDoc, '/ws/main.go', 'go', 5, 'err').find((f) => f.id.startsWith('go-if-err'));
  assert.ok(goErrFix);
  assert.ok(previewConcreteEdits(goDoc, goErrFix.edits).nextText.includes('if err != nil {'));

  // 3. Rust `#[derive(Debug, Clone)]` on struct
  const rsDoc = 'pub struct Config {\n    pub port: u16,\n}\n';
  const rsFix = computeDeterministicQuickFixes(rsDoc, '/ws/lib.rs', 'rust', 1, 'Config').find((f) => f.id.startsWith('rs-derive'));
  assert.ok(rsFix);
  assert.ok(previewConcreteEdits(rsDoc, rsFix.edits).nextText.startsWith('#[derive(Debug, Clone)]\npub struct Config'));

  // 4. JSON trailing comma removal
  const jsonDoc = '{\n  "port": 7700,\n}\n';
  const jsonFix = computeDeterministicQuickFixes(jsonDoc, '/ws/cfg.json', 'json', 2).find((f) => f.id.startsWith('json-comma'));
  assert.ok(jsonFix);
  assert.equal(JSON.parse(previewConcreteEdits(jsonDoc, jsonFix.edits).nextText).port, 7700);

  // 5. Shell unquoted `$VAR`
  const shDoc = '#!/usr/bin/env bash\necho $TARGET_DIR\n';
  const shFix = computeDeterministicQuickFixes(shDoc, '/ws/run.sh', 'shell', 2).find((f) => f.id.startsWith('sh-quote'));
  assert.ok(shFix);
  assert.ok(previewConcreteEdits(shDoc, shFix.edits).nextText.includes('"$TARGET_DIR"'));
});

test('Manifest dependency wire contract normalizes string deprecated note, latest_patch, block_end_line, and canonical action IDs', () => {
  const pom = [
    '<project>',
    '  <properties>',
    '    <jackson.version>2.14.0</jackson.version>',
    '  </properties>',
    '  <dependencies>',
    '    <dependency>',
    '      <groupId>com.fasterxml.jackson.core</groupId>',
    '      <artifactId>jackson-databind</artifactId>',
    '      <version>${jackson.version}</version>',
    '    </dependency>',
    '  </dependencies>',
    '</project>',
  ].join('\n');

  const local = parseManifestDependencies('/ws/pom.xml', pom);
  const merged = mergeDepItems(local, [
    {
      name: 'com.fasterxml.jackson.core:jackson-databind',
      current_version: '2.14.0',
      latest_version: '2.18.2',
      latest_patch: '2.14.3',
      deprecated: 'Replaced by faster streaming module',
      line: 8,
      block_start_line: 6,
      block_end_line: 10,
      property_name: 'jackson.version',
      vulnerabilities: [{ id: 'GHSA-1234', severity: 'HIGH', summary: 'Deser issue', fixed_version: '2.14.3' }],
    },
    normalizeRemoteDepItem({
      name: 'org.legacy:unmatched-lib',
      current_version: '1.0.0',
      latest_version: '2.0.0',
      latest_patch: '1.0.4',
      deprecated: 'End of life library',
      line: 11,
      block_start_line: 11,
      block_end_line: 15,
    }),
  ]);

  const jackson = merged.find((d) => d.name.endsWith('jackson-databind'))!;
  assert.equal(jackson.deprecated, true);
  assert.equal(jackson.deprecation_note, 'Replaced by faster streaming module');
  assert.equal(jackson.latest_patch, '2.14.3');
  assert.equal(jackson.block_start_line, 6);
  assert.equal(jackson.end_line, 10);

  const unmatched = merged.find((d) => d.name === 'org.legacy:unmatched-lib')!;
  assert.equal(unmatched.deprecated, true);
  assert.equal(unmatched.deprecation_note, 'End of life library');
  assert.equal(unmatched.end_line, 15);

  const patchUpdate = applyBatchFallbackManifestUpdates(pom, [jackson], 'update_direct_patch');
  assert.equal(patchUpdate.count, 1);
  assert.ok(patchUpdate.updated.includes('<jackson.version>2.14.3</jackson.version>'));

  assert.equal(normalizeIdeDepAction('update_single', 'go'), 'update_one');
  assert.equal(normalizeIdeDepAction('upgrade', 'npm'), 'update_one');
  assert.equal(normalizeIdeDepAction('update_all', 'go'), 'update_all_latest');
  assert.equal(normalizeIdeDepAction('update_direct', 'go'), 'update_direct_latest');
  assert.equal(normalizeIdeDepAction('sync', 'maven'), 'maven_sync');
  assert.equal(normalizeIdeDepAction('sync', 'gradle'), 'gradle_sync');
});

test('Preferences round-trip stays in lockstep with IdeSettingsModal keys and honors hover/Java inspection toggles', () => {
  const prevStorage = (globalThis as Record<string, unknown>).localStorage;
  const mem = createMemoryStorage();
  (globalThis as Record<string, unknown>).localStorage = mem;
  try {
    writeIdeEditorPreferences({
      showBreadcrumbs: true,
      darkColorScheme: 'jetbrains-darcula',
      lightColorScheme: 'vscode-light-modern',
    });
    const rawAfterSubbar = JSON.parse(mem.getItem(IDE_PREFS_STORAGE_KEY)!) as Record<string, unknown>;
    assert.equal(rawAfterSubbar.breadcrumbs, true);
    assert.equal(rawAfterSubbar.preferredDarkScheme, 'jetbrains-darcula');
    assert.equal(rawAfterSubbar.preferredLightScheme, 'vscode-light-modern');

    // Simulate IdeSettingsModal unchecking Breadcrumbs (`breadcrumbs: false`) and Java inspections
    mem.setItem(
      IDE_PREFS_STORAGE_KEY,
      JSON.stringify({
        ...rawAfterSubbar,
        breadcrumbs: false,
        compactHover: false,
        javaUnresolvedImports: false,
        lombokSupport: false,
        unusedImports: false,
      }),
    );
    const synced = readIdeEditorPreferences();
    assert.equal(synced.showBreadcrumbs, false, 'IdeSettingsModal breadcrumbs=false must win over stale showBreadcrumbs=true');
    assert.equal(synced.compactHover, false);
    assert.equal(synced.javaUnresolvedImports, false);
    assert.equal(synced.lombokSupport, false);
    assert.equal(synced.unusedImports, false);

    const javaSrc = 'package demo;\nimport bogus.Nope;\nimport java.util.List;\npublic class Demo {}\n';
    assert.equal(analyzeJavaDiagnostics(javaSrc, { javaUnresolvedImports: false, unusedImports: false }).diagnostics.length, 0);

    const longDoc = `${'Detailed Go documentation sentence explaining context cancellation semantics cleanly. '.repeat(5)}\n\nSecond paragraph.`;
    const expandedHover = sanitizeAndSplitHoverDoc({ symbol: 'Run', signature: 'func Run()', rawDoc: longDoc, compact: false })!;
    assert.ok(expandedHover.summary.length > 240, 'compact=false preserves full first paragraph without 240-char truncation');
  } finally {
    (globalThis as Record<string, unknown>).localStorage = prevStorage;
  }
});

test('isSecretIdePath blocks credentials, keystores, private keys, shell histories, and home dot-directories while allowing source files', () => {
  const blocked = [
    '.env',
    '/ws/app/.env.local',
    '/Users/dev/.netrc',
    '/Users/dev/.pgpass',
    '/ws/app/.npmrc',
    '/Users/dev/.pypirc',
    '/Users/dev/.git-credentials',
    '/ws/certs/server.keystore',
    '/ws/certs/truststore.jks',
    '/ws/certs/client.p12',
    '/ws/certs/client.pfx',
    '/ws/certs/tls.pem',
    '/ws/certs/private.key',
    '/Users/dev/.zsh_history',
    '/Users/dev/.bash_history',
    '~/.ssh/id_ed25519',
    '~/.aws/credentials',
    '~/.kube/config',
    '~/.gnupg/secring.gpg',
    '/Users/dev/.ssh/config',
  ];
  for (const p of blocked) {
    assert.equal(isSecretIdePath(p), true, `expected secret path block for ${p}`);
  }

  const allowed = [
    '/ws/app/cmd/key_handler.go',
    '/ws/app/internal/auth/token_service.ts',
    '/ws/app/src/keystore_mapper.rs',
    '/Users/dev/.otto/vault/notes/architecture.md',
  ];
  for (const p of allowed) {
    assert.equal(isSecretIdePath(p), false, `expected normal source file allowed for ${p}`);
  }
});

test('clearIdeSensitiveWebStorage wipes IDE drafts, undo journal, clipboard, projects, roots, and session cursor map', () => {
  const localMem = createMemoryStorage();
  const sessionMem = createMemoryStorage();

  localMem.setItem(IDE_UNDO_JOURNAL_KEY, '{"a":1}');
  localMem.setItem('otto.ide.undoDraftIndex.v1', '{"/ws/a.go":1}');
  localMem.setItem('otto.ide.undoDraft:/ws/a.go', '{"savedContent":"a","draftContent":"b","updatedAt":1}');
  localMem.setItem('otto.ide.clipboardHistory.v1', '[{"id":"1","text":"secret"}]');
  localMem.setItem('otto.ide.projects.v1', '[{"id":"p1"}]');
  localMem.setItem('otto.ide.namedProjects.v1', '[{"id":"p1"}]');
  localMem.setItem('otto.ide.lastRoot', '/ws/secret');
  localMem.setItem('otto.ide.roots:local:/ws/secret', '["/ws/secret/sub"]');
  localMem.setItem('otto_theme', 'dark');
  sessionMem.setItem(IDE_CURSOR_POS_KEY, '[["/ws::a.go",[10,10,120]]]');

  clearIdeSensitiveWebStorage(localMem, sessionMem);

  assert.equal(localMem.getItem(IDE_UNDO_JOURNAL_KEY), null);
  assert.equal(localMem.getItem('otto.ide.undoDraftIndex.v1'), null);
  assert.equal(localMem.getItem('otto.ide.undoDraft:/ws/a.go'), null);
  assert.equal(localMem.getItem('otto.ide.clipboardHistory.v1'), null);
  assert.equal(localMem.getItem('otto.ide.projects.v1'), null);
  assert.equal(localMem.getItem('otto.ide.namedProjects.v1'), null);
  assert.equal(localMem.getItem('otto.ide.lastRoot'), null);
  assert.equal(localMem.getItem('otto.ide.roots:local:/ws/secret'), null);
  assert.equal(sessionMem.getItem(IDE_CURSOR_POS_KEY), null);
  assert.equal(localMem.getItem('otto_theme'), 'dark');
});

test('restoreIdeStateFromJournal clamps out-of-bounds persisted selection offsets without RangeError', () => {
  const mem = createMemoryStorage();
  const shortDisk = 'package main\n\nfunc main() {}\n';
  mem.setItem(
    IDE_UNDO_JOURNAL_KEY,
    JSON.stringify({
      '/ws/main.go': {
        stateJson: {
          doc: shortDisk,
          selection: { main: 0, ranges: [{ anchor: 76475, head: 76475 }] },
        },
        baseSavedContent: shortDisk,
        savedContent: shortDisk,
        unsaved: false,
        scrollTop: 240,
        updatedAt: Date.now(),
      },
    }),
  );

  const restored = restoreIdeStateFromJournal('/ws/main.go', shortDisk, [history()], mem);
  assert.ok(restored);
  assert.equal(restored.state.doc.toString(), shortDisk);
  assert.equal(restored.state.selection.main.head, shortDisk.length);
  assert.equal(restored.scrollTop, 240);
});
