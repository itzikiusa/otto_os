///<reference types="svelte" />
;;
  // NeedsYouLink, loaded after its host page (Mission Control, MCP approvals,
  // the Assistant rail, Agents → Work Queue). The link pulls the Today store,
  // which joins every "needs you" source (assistant, MCP, scheduled tasks,
  // design and mission-control APIs); imported statically it put all of that
  // into each host page's chunk — Mission Control +17 %, MCP +9 % gzip vs main
  // (bundle budget). It is a small secondary affordance, so nothing renders
  // while its chunk loads (no skeleton flash); a failed load leaves it out —
  // Home → Needs you stays one sidebar click away.
  interface Props {
    /** Extra class for placement tweaks by the host surface. */
    class?: string;
  };function $$render() {


  let { class: cls = '' }: Props = $props();

  const link = import('./NeedsYouLink.svelte');
;
async () => {

    { try { const $$_value = await (link);{ const m = $$_value; 
   { const $$_tluafed_m0C = __sveltets_2_ensureComponent(m.default); new $$_tluafed_m0C({ target: __sveltets_2_any(), props: {  "class":cls,}});}
}} catch($$_e) { 
  
}}
};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LazyNeedsYouLink__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LazyNeedsYouLink__SvelteComponent_ = ReturnType<typeof LazyNeedsYouLink__SvelteComponent_>;
/*Ωignore_endΩ*/export default LazyNeedsYouLink__SvelteComponent_;