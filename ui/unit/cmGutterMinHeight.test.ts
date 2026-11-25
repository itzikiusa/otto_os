import assert from 'node:assert/strict';
import { test } from 'node:test';
import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { placeholderEditorTheme } from '../src/lib/components/cm-editor-helpers.ts';

// WebKit (the app's WKWebView) re-lays out every CodeMirror gutter column whose
// `min-height: 100%` resolves against the ~80k px gutter box, on ANY layout of the page:
// 17 ms per typed character / cursor move / status-bar update in a 4k-line file, 4 ms without.
// Every editor carries placeholderEditorTheme, which must cancel that min-height.
test('editor theme drops the gutter column min-height', () => {
  const state = EditorState.create({ extensions: [placeholderEditorTheme] });
  const css = state.facet(EditorView.styleModule).map((m) => m.getRules()).join('\n');
  assert.match(css, /\.cm-gutter\s*\{\s*min-height:\s*0;?\s*\}/);
});
