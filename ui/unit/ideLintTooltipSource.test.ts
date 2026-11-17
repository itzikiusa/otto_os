import assert from 'node:assert/strict';
import { test } from 'node:test';
import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { createIdeSyntaxTheme } from '../src/lib/components/cm-ide-theme.ts';

test('the lint tooltip names a linked diagnostic source once', () => {
  const state = EditorState.create({ extensions: createIdeSyntaxTheme({ dark: false, ideMode: true }) });
  const css = state.facet(EditorView.styleModule).map((m) => m.getRules()).join('\n');
  assert.match(css, /\.cm-diagnostic:has\(\.cm-diagnostic-code-link\) > \.cm-diagnosticSource\s*\{\s*display:\s*none/);
});
