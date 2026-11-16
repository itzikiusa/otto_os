// IDE on a phone, "works" → "good" (round 5, IDE-PHONE): the caret stays above the
// keyboard, typing mode folds the chrome, the editor's verbs by touch, the paused step
// bar, soft-wrap hanging indent and the text-size step.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  caretNeedsReveal, caretRevealMargin, leadingIndentColumns, revealYMargin, visibleBand, wrapHangColumns, WRAP_INDENT_MAX,
} from '../src/lib/components/cm-ide-touch.ts';
import { ideTouchVerbRows, touchShapeIdeMenu, withoutHints } from '../src/lib/components/cm-ide-touch-menu.ts';
import { idePhoneChromeFor, nextIdeFontSize, TYPING_FILE_ROW_MIN_H } from '../src/modules/ide/editor/idePhoneChromeModel.ts';
import { pausedLineClearance, touchDebugBar } from '../src/modules/ide/editor/ideTouchDebugBar.ts';

// ── typing: the caret stays in the visible band ─────────────────────────────

test('caret reveal margin scales with the visible height and stays within 16..64 px', () => {
  assert.equal(caretRevealMargin(40), 16);
  assert.equal(caretRevealMargin(200), 40);
  assert.equal(caretRevealMargin(2000), 64);
});

test('the visible band is the scroller cut to the visual viewport (the keyboard covers the rest)', () => {
  assert.deepEqual(visibleBand({ top: 80, bottom: 700 }, 0, 404), { top: 80, bottom: 404 });
  assert.deepEqual(visibleBand({ top: 80, bottom: 300 }, 0, 404), { top: 80, bottom: 300 }, 'already inside');
  assert.deepEqual(visibleBand({ top: 80, bottom: 300 }, 0, 0), { top: 80, bottom: 300 }, 'no visual viewport: the scroller');
});

test('a caret under the keyboard (or near the bottom edge, or above the band) needs a reveal; one comfortably inside does not', () => {
  const band = { top: 80, bottom: 404 };
  assert.equal(caretNeedsReveal({ top: 500, bottom: 518 }, band, 48), true, 'under the keyboard');
  assert.equal(caretNeedsReveal({ top: 380, bottom: 398 }, band, 48), true, 'inside the margin at the bottom');
  assert.equal(caretNeedsReveal({ top: 60, bottom: 78 }, band, 48), true, 'scrolled above');
  assert.equal(caretNeedsReveal({ top: 200, bottom: 218 }, band, 48), false, 'comfortably visible: never fight the person');
  assert.equal(caretNeedsReveal(null, band, 48), true, 'not rendered at all');
  assert.equal(caretNeedsReveal({ top: 100, bottom: 118 }, { top: 100, bottom: 110 }, 48), false, 'band smaller than a line: nothing to do');
});

test('scrollIntoView margin adds the part of the scroller hidden below the band', () => {
  assert.equal(revealYMargin({ top: 80, bottom: 700 }, { top: 80, bottom: 404 }, 40), 336);
  assert.equal(revealYMargin({ top: 80, bottom: 300 }, { top: 80, bottom: 300 }, 40), 40);
});

// ── typing mode: which chrome folds away ─────────────────────────────────────

const base = { stack: true, pageH: 690, short: false, wide: false, typing: false, kbOpen: false, visibleH: 740 };

test('desktop workbench: never typing mode, the tab strip stays', () => {
  assert.deepEqual(idePhoneChromeFor({ ...base, stack: false, typing: true, kbOpen: true }), { typingMode: false, header: true, fileRowInHeader: false, fileRow: true });
});

test('phone portrait, reading: header + file row; typing with the keyboard up: header goes, the file row stays while it fits', () => {
  assert.deepEqual(idePhoneChromeFor(base), { typingMode: false, header: true, fileRowInHeader: false, fileRow: true });
  assert.deepEqual(idePhoneChromeFor({ ...base, typing: true }), { typingMode: false, header: true, fileRowInHeader: false, fileRow: true }, 'focus alone (hardware keyboard) changes nothing');
  const tall = idePhoneChromeFor({ ...base, typing: true, kbOpen: true, pageH: 360, short: true, visibleH: 403 });
  assert.equal(tall.typingMode, true);
  assert.equal(tall.header, false);
  assert.equal(tall.fileRow, true, 'iPhone Pro Max: one row of file chrome stays');
  const se = idePhoneChromeFor({ ...base, typing: true, kbOpen: true, pageH: 188, short: true, visibleH: 232 });
  assert.equal(se.fileRow, false, 'iPhone SE: every row is a line of code');
  assert.ok(TYPING_FILE_ROW_MIN_H > 232 && TYPING_FILE_ROW_MIN_H <= 403);
});

test('phone on its side: the file row moves into the header (two chrome rows), typing folds both', () => {
  const land = { ...base, pageH: 394, short: true, wide: true, visibleH: 430 };
  assert.deepEqual(idePhoneChromeFor(land), { typingMode: false, header: true, fileRowInHeader: true, fileRow: false });
  const kb = idePhoneChromeFor({ ...land, typing: true, kbOpen: true, pageH: 214, visibleH: 250 });
  assert.deepEqual(kb, { typingMode: true, header: false, fileRowInHeader: false, fileRow: false });
  assert.equal(idePhoneChromeFor({ ...land, typing: true, pageH: 300 }).typingMode, false, 'focus without a keyboard (a tap on the code of a short page) keeps the chrome');
  assert.deepEqual(idePhoneChromeFor({ ...base, pageH: 468, short: true, wide: false }), { typingMode: false, header: true, fileRowInHeader: false, fileRow: true }, 'iPhone SE upright: short but narrow — the row keeps its own line');
});

// ── touch command menu ───────────────────────────────────────────────────────

test('touch verbs: definition, usages, docs, rename, quick fix, then find / replace / go to line / text size', () => {
  const noop = (): void => {};
  const rows = ideTouchVerbRows({ symbol: 'Run', readOnly: false, definition: noop, usages: noop, docs: noop, rename: noop, quickFix: noop, find: noop, replace: noop, goToLine: noop, textSize: noop });
  assert.deepEqual(rows.map((r) => r.label ?? '—'), [
    'Go to Definition', 'Find Usages', 'Show Documentation', 'Rename…', 'Quick Fix…', '—',
    'Find in File…', 'Replace in File…', 'Go to Line…', 'Text Size', '—',
  ]);
  assert.ok(rows.every((r) => !r.hint), 'no shortcut hints on glass');
  const off = ideTouchVerbRows({ symbol: '', readOnly: true, definition: noop, rename: noop, replace: noop, goToLine: noop });
  assert.equal(off.find((r) => r.label === 'Go to Definition')?.disabled, true, 'no name under the caret');
  assert.equal(off.find((r) => r.label === 'Rename…')?.disabled, true);
  assert.equal(off.find((r) => r.label === 'Replace in File…')?.disabled, true, 'read-only file');
  assert.equal(off.find((r) => r.label === 'Go to Line…')?.disabled, false);
});

test('touch menu: verbs first, the desktop rows follow without duplicates, hints or a doubled separator', () => {
  const noop = (): void => {};
  const desktop = [
    { label: 'Show Context Actions', hint: '⌥⏎', action: noop },
    { separator: true },
    { label: 'Copy', hint: '⌘C', action: noop },
    { label: 'Find Usages', hint: '⌥F7', action: noop },
    { separator: true },
    { label: 'Rename…', hint: '⇧F6', action: noop },
    { separator: true },
    { label: 'Refactor', children: [{ label: 'Rename…', hint: '⇧F6', action: noop }] },
  ];
  const menu = touchShapeIdeMenu(desktop, { symbol: 'x', readOnly: false, definition: noop, usages: noop, find: noop });
  const labels = menu.map((r) => r.label ?? '—');
  assert.deepEqual(labels, ['Go to Definition', 'Find Usages', '—', 'Find in File…', '—', 'Copy', '—', 'Refactor']);
  assert.ok(withoutHints(menu).every((r) => !r.hint && (r.children ?? []).every((c) => !c.hint)));
  assert.equal(menu[menu.length - 1].children?.[0].hint, undefined, 'submenu hints go too');
});

// ── paused step bar ──────────────────────────────────────────────────────────

test('step bar: shows while paused on the stacked editor, says file:line, survives a step, hides while typing', () => {
  const paused = { state: 'paused' as const, paused_file: '/repo/svc/pipeline.py', paused_line: 24 };
  assert.deepEqual(touchDebugBar({ dbg: paused, editorPane: true, stepping: false, typingMode: false }), { show: true, paused: true, where: 'pipeline.py:24' });
  assert.equal(touchDebugBar({ dbg: paused, editorPane: false, stepping: false, typingMode: false }).show, false, 'Tools pane / desktop: the panel has the controls');
  assert.equal(touchDebugBar({ dbg: paused, editorPane: true, stepping: false, typingMode: true }).show, false);
  const running = { state: 'running' as const };
  assert.equal(touchDebugBar({ dbg: running, editorPane: true, stepping: false, typingMode: false }).show, false, 'a free-running program: no bar');
  assert.deepEqual(touchDebugBar({ dbg: running, editorPane: true, stepping: true, typingMode: false }), { show: true, paused: false, where: '' }, 'between a step and the next stop the bar stays (no flicker)');
  assert.equal(touchDebugBar({ dbg: { state: 'terminated' }, editorPane: true, stepping: true, typingMode: false }).show, false);
  assert.equal(touchDebugBar({ dbg: null, editorPane: true, stepping: false, typingMode: false }).show, false);
});

test('paused line clearance: scroll only when the line would sit under the bar', () => {
  assert.equal(pausedLineClearance(500, 520), 0);
  assert.equal(pausedLineClearance(515, 520), 3);
  assert.equal(pausedLineClearance(600, 520, 8), 88);
});

// ── reading comfort ──────────────────────────────────────────────────────────

test('wrap indent: continuation rows hang under the line indent + 2, capped; blank lines get none', () => {
  assert.equal(leadingIndentColumns('    x = 1', 4), 4);
  assert.equal(leadingIndentColumns('\t\tif x:', 4), 8);
  assert.equal(leadingIndentColumns('  \tfoo', 4), 4, 'a tab stops at the next tab column');
  assert.equal(leadingIndentColumns('   ', 4), -1);
  assert.equal(wrapHangColumns('func main() {', 4), 2);
  assert.equal(wrapHangColumns('        return x', 4), 10);
  assert.equal(wrapHangColumns(`${' '.repeat(40)}deep`, 4), WRAP_INDENT_MAX);
  assert.equal(wrapHangColumns('', 4), 0);
});

test('text size steps by one point inside the preference bounds', () => {
  assert.equal(nextIdeFontSize(13, 1), 14);
  assert.equal(nextIdeFontSize(10, -1), 10);
  assert.equal(nextIdeFontSize(24, 1), 24);
  assert.equal(nextIdeFontSize(Number.NaN, 1), 14);
});

test('compact editor set carries the phone extensions (caret keeper, wrap indent, find panel, no code-vision rows)', async () => {
  const src = (await import('node:fs')).readFileSync(new URL('../src/lib/components/cm-ide-compact.ts', import.meta.url), 'utf8');
  assert.match(src, /const ON: Extension = \[EditorView\.lineWrapping, compactTheme, wrapIndent, keepCaretVisible\]/);
  assert.match(src, /'\.cm-ide-codevision-row, \.cm-ide-codevision': \{ display: 'none' \}/);
  assert.match(src, /'\.cm-panel\.cm-search \.cm-button': \{ minBlockSize: '44px'/);
  assert.match(src, /fontSize: '16px'/, 'the find field never makes iOS zoom the page');
});
