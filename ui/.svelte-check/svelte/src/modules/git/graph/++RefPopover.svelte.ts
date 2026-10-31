///<reference types="svelte" />
;// Collapsed-row ref popover: the branches (then tags) on one commit, opened
// from a row's "▾ +N" expander. GraphView mounts it at the component root
// (position:fixed) so it clears the scroll container. A full-screen backdrop
// closes it on any outside click, Escape closes it (dialogFocus), focus moves
// in, is trapped, and returns to the expander. Behaviour lives in
// refPopoverState.svelte.ts; this file renders and places it.

import { untrack } from 'svelte';
import { rowMenu } from '../../../lib/rowMenu';
import { dialogFocus } from '../../../lib/dialogFocus';
import { ui } from '../../../lib/stores/ui.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { clampPopover, popoverMaxHeight, type RefMenu, type RefPopover } from './refPopoverState.svelte';
;

  interface Props {
    pop: RefPopover;
    menu: RefMenu;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { pop, menu }: Props = $props();

  // Clamp into the viewport once the popover's size is known. The height cap
  // is set first (so the measured height already respects it), then the
  // corner is clamped on the next frame.
  let el = $state<HTMLDivElement | null>(null);
  let x = $state(0);
  let y = $state(0);
  let maxH = $state(320);
  $effect(() => {
    const m = menu;
    x = m.x;
    y = m.y;
    maxH = popoverMaxHeight(window.innerHeight, m.y);
    requestAnimationFrame(() => {
      if (!el || pop.menu !== m) return;
      const at = clampPopover(m, { w: el.offsetWidth, h: el.offsetHeight }, { w: window.innerWidth, h: window.innerHeight });
      x = at.x;
      y = at.y;
    });
  });

  // A modal surface: the shell's global shortcuts stand down while it's open.
  $effect(() => {
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });
;
async () => {

 { svelteHTML.createElement("button", {       "type":`button`,"class":`ref-pop-backdrop`,"aria-label":`Close`,"onclick":pop.close,}); }
 {const $$action_0 = __sveltets_2_ensureAction(dialogFocus(svelteHTML.mapElementTag('div'),(pop.close)));{ const $$_div0 = svelteHTML.createElement("div", __sveltets_2_union($$action_0), {            "class":`ref-popover`,"role":`dialog`,"aria-modal":`true`,"aria-label":`Commit references`,"style":`left: ${x}px; top: ${y}px; max-height: ${maxH}px;`,});el = $$_div0;
  if(menu.branches.length > 0){
     { svelteHTML.createElement("div", { "class":`ref-pop-group`,});  }
       for(let chip of __sveltets_2_ensureArray(menu.branches)){chip.kind + chip.label;
       { svelteHTML.createElement("div", { "class":`ref-action-row`,});
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                "type":`button`,"class":`ref-pop-row kind-${chip.kind}`,"title":chip.current
          ? `Checked out · ${chip.label} — double-click to re-checkout`
          : chip.worktree
            ? `Worktree · ${chip.label} — double-click to open worktree`
            : `Double-click to checkout · ${chip.label}`,"onclick":() => pop.rowSelect(menu.commit),"ondblclick":() => pop.rowCheckout(chip),"oncontextmenu":(e) => pop.rowMenu(e, chip),});chip.current;chip.worktree && !chip.current;
        if(chip.current){
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
        } else if (chip.worktree){
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`worktree`,"size":12,}});}
        } else if (chip.kind === 'remote' || chip.onRemote){
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});}
        }else{
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});}
        }
         { svelteHTML.createElement("span", { "class":`ref-pop-label`,});chip.label; }
        if(chip.current){
           { svelteHTML.createElement("span", { "class":`ref-pop-tag`,});  }
        } else if (chip.worktree){
           { svelteHTML.createElement("span", { "class":`ref-pop-tag`,});  }
        }
       }}
       { svelteHTML.createElement("button", {         "class":`icon-btn ref-action`,"title":`Actions for ${chip.label}`,"aria-label":`Actions for ${chip.label}`,"aria-haspopup":`menu`,"onclick":(e) => pop.rowMenu(e, chip),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
       }
    }
  }
  if(menu.tags.length > 0){
     { svelteHTML.createElement("div", { "class":`ref-pop-group`,});  }
       for(let chip of __sveltets_2_ensureArray(menu.tags)){chip.kind + chip.label;
       { svelteHTML.createElement("div", { "class":`ref-action-row`,});
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {              "type":`button`,"class":`ref-pop-row kind-tag`,"title":`Double-click to checkout tag (detached) · ${chip.label}`,"onclick":() => pop.rowSelect(menu.commit),"ondblclick":() => pop.rowCheckout(chip),"oncontextmenu":(e) => pop.rowMenu(e, chip),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`ref-pop-label`,});chip.label; }
       }}
       { svelteHTML.createElement("button", {         "class":`icon-btn ref-action`,"title":`Actions for ${chip.label}`,"aria-label":`Actions for ${chip.label}`,"aria-haspopup":`menu`,"onclick":(e) => pop.rowMenu(e, chip),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
       }
    }
  }
 }}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RefPopover__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RefPopover__SvelteComponent_ = ReturnType<typeof RefPopover__SvelteComponent_>;
/*Ωignore_endΩ*/export default RefPopover__SvelteComponent_;