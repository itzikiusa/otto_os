// The IDE editor in the stacked (phone / narrow window) layout: soft-wrapped lines, so a
// long line never needs horizontal scrolling on a 390 px screen, and a narrow breakpoint
// lane (20 px instead of 34 px; the run/lightbulb slot gives way to the breakpoint dot) and
// no lint-marker column, keeping the whole gutter (breakpoints + line numbers) within 60 px. The desktop
// workbench is unchanged. One compartment per editor, flipped for every live editor by
// `setIdeCompactLayout` (the IDE page calls it when its layout changes); an editor built
// later — or restored from a cached state — picks up the current mode when it mounts.

import { Compartment, type Extension } from '@codemirror/state';
import { EditorView, ViewPlugin } from '@codemirror/view';
import { keepCaretVisible, wrapIndent } from './cm-ide-touch.ts';

const compactTheme = EditorView.theme({
  '.cm-ide-bp-gutter, .cm-ide-bp-gutter .cm-gutterElement': { inlineSize: '20px', minInlineSize: '20px', maxInlineSize: '20px', paddingInline: '0' },
  '.cm-ide-bp-cell': { inlineSize: '18px', minInlineSize: '18px', maxInlineSize: '18px', gridTemplateColumns: '14px 0', columnGap: '0' },
  '.cm-ide-act-slot': { display: 'none' },
  '.cm-lineNumbers .cm-gutterElement': { paddingInline: '2px 4px', minInlineSize: '2ch' },
  // The lint gutter's markers duplicate the underline + Problems pane; on a phone the
  // column costs more than it tells (CodeMirror marks gutters `display: flex !important`).
  '.cm-gutter-lint': { display: 'none !important' },
  // Code-vision rows ("Implement interface", "N usages") cost a whole row each on a phone;
  // both verbs are in the editor's touch menu (Find usages, Generate).
  '.cm-ide-codevision-row, .cm-ide-codevision': { display: 'none' },
  // The find/replace panel, thumb-sized: 44 px controls that wrap inside the width, and a
  // 16 px field (iOS zooms the page into anything smaller on focus).
  '.cm-panel.cm-search': { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: '6px', paddingBlock: '6px', paddingInline: '8px 48px', position: 'relative' },
  '.cm-panel.cm-search br': { display: 'none' },
  '.cm-panel.cm-search .cm-textfield': { flex: '1 1 100%', minBlockSize: '40px', fontSize: '16px', margin: '0' },
  '.cm-panel.cm-search .cm-button': { minBlockSize: '44px', minInlineSize: '44px', margin: '0' },
  '.cm-panel.cm-search label': { display: 'inline-flex', alignItems: 'center', gap: '4px', minBlockSize: '44px', fontSize: 'var(--fs-s)' },
  '.cm-panel.cm-search button[name=close]': { insetBlockStart: '6px', insetInlineEnd: '6px', inlineSize: '40px', blockSize: '40px', fontSize: '20px' },
  // The hanging indent of wrapped lines (wrapIndent) is a negative `text-indent` on the line.
  // `text-indent` is inherited and applies inside inline-block children too, so without this
  // an inlay hint's or widget's own text is pulled left over the code before it.
  '.cm-line > *': { textIndent: '0' },
  // Hover docs opened by touch: inside the screen, scrollable.
  '.cm-tooltip-hover': { maxInlineSize: 'calc(100vw - 16px)', maxBlockSize: '50vh', overflow: 'auto' },
});

const ON: Extension = [EditorView.lineWrapping, compactTheme, wrapIndent, keepCaretVisible];
const OFF: Extension = [];

const compartment = new Compartment();
const views = new Set<EditorView>();
let compact = false;

/** Whether the stacked (compact) editor is on. */
export function isIdeCompactLayout(): boolean {
  return compact;
}

/** Part of the IDE editor extensions: the compartment plus a tracker of live editors. */
export function ideCompactExtension(): Extension {
  return [
    compartment.of(compact ? ON : OFF),
    ViewPlugin.define((view) => {
      views.add(view);
      // A cached EditorState restored after a layout change carries the old mode.
      if (compartment.get(view.state) !== (compact ? ON : OFF)) queueMicrotask(() => sync(view));
      return { destroy: () => views.delete(view) };
    }),
  ];
}

function sync(view: EditorView): void {
  if (!views.has(view)) return;
  const want = compact ? ON : OFF;
  if (compartment.get(view.state) !== want) view.dispatch({ effects: compartment.reconfigure(want) });
}

/** Switch every live IDE editor to (or from) the stacked layout's editor. */
export function setIdeCompactLayout(on: boolean): void {
  if (on === compact) return;
  compact = on;
  for (const v of views) sync(v);
}
