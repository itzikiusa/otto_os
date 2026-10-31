///<reference types="svelte" />
;;
  interface Props {
    rows?: number;
    height?: number;
    /** Noun for the screen-reader text: "Loading {label}". */
    label?: string;
    /** false when a parent (LoadState) already owns the status announcement. */
    announce?: boolean;
    /** Fade in after ~150 ms (the height is reserved at once) so a fast load never
     *  flashes the placeholder. false when a parent (LoadState) already waits. */
    grace?: boolean;
  };function $$render() {


  let { rows = 3, height = 36, label = '', announce = true, grace = true }: Props = $props();

  // Live regions announce CHANGES to a region that already exists; one that
  // is inserted together with its text is usually silent in VoiceOver/WebKit
  // (S19-305). So the status region mounts empty and the "Loading …" text is
  // added after the same ~150 ms grace the rows fade in with (LoadState does
  // the same) — a fast load also never announces at all.
  let speak = $state(false);
  $effect(() => {
    if (!announce) return;
    const t = setTimeout(() => (speak = true), 150);
    return () => clearTimeout(t);
  });
;
async () => {


  { svelteHTML.createElement("div", {       "class":`skeleton-list`,"role":announce ? 'status' : undefined,"aria-label":announce && label ? `Loading ${label}` : undefined,});grace;
  if(announce && speak){ { svelteHTML.createElement("span", { "class":`sr-only`,}); label ? ` ${label}` : ''; }}
      for(let _ of __sveltets_2_ensureArray(Array(rows))){let i = 1;i;
     { svelteHTML.createElement("div", {   "class":`skeleton-row`,"style":`height:${height}px; --stagger:${i * 90}ms`,}); }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Skeleton__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Skeleton__SvelteComponent_ = ReturnType<typeof Skeleton__SvelteComponent_>;
/*Ωignore_endΩ*/export default Skeleton__SvelteComponent_;