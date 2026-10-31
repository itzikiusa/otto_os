///<reference types="svelte" />
;// One on/off row for every Settings page: the checkbox on the leading side,
// then a title and an optional one-line hint — the `.checkbox-row`
// convention from docs/design/guidelines (layout.md → Settings form). Pages
// used to mix hand-rolled switches (with an invisible keyboard focus), bare
// trailing checkboxes and "☑ On" labels; this keeps them one control.
//
// It applies immediately: `onchange` gets the new value. When the handler
// returns a promise the box re-syncs to `checked` afterwards, so a failed
// save the parent reverted (or never applied) can't leave it showing a
// state that isn't in effect.

import type { Snippet } from 'svelte';
;

  interface Props {
    label: string;
    /** One dim line under the label (or pass `children` for rich text). */
    hint?: string;
    checked: boolean;
    disabled?: boolean;
    /** Tooltip — say WHY when disabled. */
    title?: string;
    onchange: (checked: boolean) => void | Promise<void>;
    testid?: string;
    children?: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { label, hint, checked, disabled = false, title, onchange, testid, children }: Props = $props();
  // The hint is a DESCRIPTION, not part of the name: inside the wrapping
  // <label> it would otherwise read as one long accessible name.
  const uid = $props.id();
  const hasHint = $derived(!!children || !!hint);

  async function change(e: Event & { currentTarget: HTMLInputElement }): Promise<void> {
    const el = e.currentTarget;
    const r = onchange(el.checked);
    if (r instanceof Promise) {
      await r;
      el.checked = checked;
    }
  }
;
async () => {

 { svelteHTML.createElement("label", {   "class":`st`,title,});disabled;
   { svelteHTML.createElement("input", {             "type":`checkbox`,checked,disabled,"onchange":change,...__sveltets_2_empty({"data-testid":testid}),"aria-labelledby":`${uid}-label`,"aria-describedby":hasHint ? `${uid}-hint` : undefined,});}
   { svelteHTML.createElement("span", { "class":`st-text`,});
     { svelteHTML.createElement("span", {   "class":`st-label`,"id":`${uid}-label`,});label; }
    if(children){
       { svelteHTML.createElement("span", {   "class":`st-hint`,"id":`${uid}-hint`,});;__sveltets_2_ensureSnippet(children()); }
    } else if (hint){
       { svelteHTML.createElement("span", {   "class":`st-hint`,"id":`${uid}-hint`,});hint; }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SettingToggle__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SettingToggle__SvelteComponent_ = ReturnType<typeof SettingToggle__SvelteComponent_>;
/*Ωignore_endΩ*/export default SettingToggle__SvelteComponent_;