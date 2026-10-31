///<reference types="svelte" />
;;
  // The one on/off switch (macOS-style): a real <button role="switch"> so it is
  // keyboard- and screen-reader-operable, with logical (RTL-safe) knob travel.
  interface Props {
    checked: boolean;
    onchange: (next: boolean) => void;
    /** Accessible name — a noun phrase naming WHAT is toggled ("Enable server filesystem"). */
    label: string;
    disabled?: boolean;
    title?: string;
    /** `success` reads as "armed/allowed" (tool enablement); default is the accent. */
    tone?: 'accent' | 'success';
  };function $$render() {


  let { checked, onchange, label, disabled = false, title, tone = 'accent' }: Props = $props();
;
async () => {

  { svelteHTML.createElement("button", {                 "type":`button`,"class":`sw`,"role":`switch`,"aria-checked":checked,"aria-label":label,"title":title ?? label,disabled,"onclick":() => onchange(!checked),});checked;tone === 'success'; }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Switch__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Switch__SvelteComponent_ = ReturnType<typeof Switch__SvelteComponent_>;
/*Ωignore_endΩ*/export default Switch__SvelteComponent_;