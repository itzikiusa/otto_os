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
