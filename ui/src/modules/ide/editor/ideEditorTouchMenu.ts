// The editor's command menu by touch (the phone tab row's ⋯). A phone has no ⌘B, no
// hover and — on iOS — no right-click: a long-press in the code belongs to the system's
// own text selection (Copy / Paste callout), which must keep working, so the verbs live
// behind one visible button. The menu is the editor's own context menu
// (cm-ide-refactor.ts), touch-shaped: Go to definition, Find usages, Show documentation,
// Rename, Quick fix, Find / Replace in file, Go to line and Text size first, then the
// rest of the desktop menu. With no code editor on screen (a preview, a table) it falls
// back to the file-level rows.

import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { idePreferences } from '../../../lib/idePreferences.svelte';
import { openGoToLine } from '../search/ideQuickOpen.svelte';
import { nextIdeFontSize } from './idePhoneChromeModel';

export function bumpIdeFontSize(delta: -1 | 1): void {
  idePreferences.update({ fontSize: nextIdeFontSize(idePreferences.current.fontSize, delta) });
}

/** Rows when no code editor is on screen. */
export function ideTouchFallbackRows(): MenuItem[] {
  return [
    { label: 'Go to Line…', icon: 'step', action: () => openGoToLine() },
    { label: 'Text Size', icon: 'text', children: [
      { label: 'Larger', icon: 'plus', action: () => bumpIdeFontSize(1) },
      { label: 'Smaller', icon: 'minus', action: () => bumpIdeFontSize(-1) },
    ] },
  ];
}

/** The primary pane's live CodeMirror (null: preview, table, analyzer, nothing open). */
function primaryEditorDom(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[data-testid="ide-editor-area"] .ide-pane:not(.split-secondary) .cm-editor');
}

export async function openIdeEditorTouchMenu(anchor: HTMLElement): Promise<void> {
  const r = anchor.getBoundingClientRect();
  const dom = primaryEditorDom();
  if (dom) {
    const [{ EditorView }, refactor] = await Promise.all([
      import('@codemirror/view'),
      import('../../../lib/components/cm-ide-refactor'),
    ]);
    const view = EditorView.findFromDOM(dom);
    if (view && refactor.openIdeEditorMenu(view, r.right, r.bottom + 4, { goToLine: () => openGoToLine(), textSize: bumpIdeFontSize })) return;
  }
  ctxMenu.showAt(anchor, ideTouchFallbackRows(), { align: 'end' });
}
