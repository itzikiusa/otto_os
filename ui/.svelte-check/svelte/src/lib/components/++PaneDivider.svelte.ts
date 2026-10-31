///<reference types="svelte" />
;// The draggable + keyboard-operable divider on the INLINE-END edge of a list /
// index pane (the "window splitter"): pointer drag, ←/→ (⇧ big step), Home/End,
// Enter or double-click to reset, with the width remembered per `storageKey`.
// The page owns the pane's width (`bind:width`) and applies it to the pane:
//
//   let listW = $state(loadPaneWidth('proof.railW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
//   <aside style:width="{listW}px">…</aside>
//   <PaneDivider bind:width={listW} storageKey="proof.railW" label="Resize the runs list" />

import { LIST_PANE, paneResizer, pxWide, RESIZE_TITLE, savePaneWidth } from '../paneResizer';
import { startMouseDrag } from '../dragCursor';
;

  interface Props {
    width: number;
    storageKey: string;
    label: string;
    min?: number;
    max?: number;
    defaultWidth?: number;
    /** The pane sits on the inline-END side of the divider (it grows when the
     *  divider moves toward the inline start), e.g. a right-hand detail pane. */
    invert?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let {
    width = $bindable(),
    storageKey,
    label,
    min = LIST_PANE.min,
    max = LIST_PANE.max,
    defaultWidth = LIST_PANE.default,
    invert = false,
  }: Props = $props()/*Ωignore_startΩ*/;width;/*Ωignore_endΩ*/;

  const clamp = (w: number): number => Math.max(min, Math.min(max, Math.round(w)));
  function set(w: number): void {
    width = clamp(w);
    savePaneWidth(storageKey, width);
  }
  function startDrag(e: MouseEvent): void {
    if (e.button !== 0) return;
    const startX = e.clientX;
    const startW = width;
    const rtl = getComputedStyle(e.currentTarget as HTMLElement).direction === 'rtl';
    const dir = (rtl ? -1 : 1) * (invert ? -1 : 1);
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => (width = clamp(startW + dir * (ev.clientX - startX))),
      onEnd: () => savePaneWidth(storageKey, width),
    });
  }
;
async () => {


  {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: width, min, max, invert, step: 10, bigStep: 40, onChange: set, onReset: () => set(defaultWidth), onDragStart: startDrag, text: pxWide })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {            "class":`pane-divider`,"role":`separator`,"aria-orientation":`vertical`,"aria-label":label,"title":RESIZE_TITLE,}); }}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('width'), slots: {}, events: {} }}
const PaneDivider__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PaneDivider__SvelteComponent_ = ReturnType<typeof PaneDivider__SvelteComponent_>;
/*Ωignore_endΩ*/export default PaneDivider__SvelteComponent_;