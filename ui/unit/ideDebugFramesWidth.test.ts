import assert from 'node:assert/strict';
import { test } from 'node:test';
import { activeThreadTitle, FRAMES_PANE } from '../src/modules/ide/debug/ideDebugPanelModel.ts';

test('the Frames column fits a Go frame name next to its file:line', () => {
  // `testing.(*T).Run.gowrap1  testing.go:2260` was cut to ~9 characters at 190px.
  assert.ok(FRAMES_PANE.default >= 260, 'default width');
  assert.ok(FRAMES_PANE.min >= 220, 'minimum width');
});

test('a cut-off goroutine picker names the active thread on hover', () => {
  const threads = [{ id: 1, name: 'Go 1' }, { id: 4, name: 'Go 4' }];
  assert.equal(activeThreadTitle(threads, { id: 4, reason: 'breakpoint' }), '#4 Go 4 · breakpoint');
  assert.equal(activeThreadTitle(threads, { id: null, reason: null }), '#1 Go 1');
  assert.equal(activeThreadTitle([], { id: null, reason: null }), undefined);
});
