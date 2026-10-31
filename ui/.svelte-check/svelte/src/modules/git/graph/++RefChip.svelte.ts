///<reference types="svelte" />
;// One colour-coded ref chip (local / remote / tag / HEAD / stash / worktree),
// shared by the commit rows (inline and collapsed) and the commit detail
// header so the renderings never drift apart. `lane` tints the side facing
// the graph in the colour of the node the ref sits on; `tracking` adds the
// ahead/behind counts of a local branch.

import Icon from '../../../lib/components/Icon.svelte';
import type { RefChip } from '../ref-menus';
;

  interface Props {
    chip: RefChip;
    label: string;
    title: string;
    lane?: string;
    tracking?: { ahead: number; behind: number } | null;
  };function $$render() {

  
  
  
  
  
  
  

  let { chip, label, title, lane, tracking = null }: Props = $props();
;
async () => {

  { svelteHTML.createElement("span", {      "class":`ref-chip kind-${chip.kind}`,title,});chip.current;chip.worktree && !chip.current;__sveltets_2_ensureType(String, Number, lane);
  if(chip.current){ { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}
  if(chip.worktree && !chip.current){ { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`worktree`,"size":12,}});}}
  if(chip.kind === 'remote' || chip.onRemote){ { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});}}
  if(chip.kind === 'tag'){ { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":12,}});}}
  if(chip.kind === 'stash'){ { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`stash`,"size":12,}});}}
   { svelteHTML.createElement("span", { "class":`chip-label`,});label; }
  if(tracking && (tracking.ahead > 0 || tracking.behind > 0)){
     { svelteHTML.createElement("span", {   "class":`chip-ab`,"title":`${tracking.ahead} ahead · ${tracking.behind} behind upstream`,});
      if(tracking.ahead > 0){ { svelteHTML.createElement("span", { "class":`ab-ahead`,}); tracking.ahead; }}
      if(tracking.behind > 0){ { svelteHTML.createElement("span", { "class":`ab-behind`,}); tracking.behind; }}
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RefChip__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RefChip__SvelteComponent_ = ReturnType<typeof RefChip__SvelteComponent_>;
/*Ωignore_endΩ*/export default RefChip__SvelteComponent_;