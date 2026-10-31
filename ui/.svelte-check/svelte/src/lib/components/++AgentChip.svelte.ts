///<reference types="svelte" />
;;
  // The one "Agent" marker (patterns.md §2): sits beside any text an agent
  // wrote so it never passes for a person's. A neutral `.chip` — agent
  // identity is never the accent colour. AgentByline renders it too.
  interface Props {
    /** Override the tooltip. */
    title?: string;
  };function $$render() {


  let { title = 'Written by an agent — review before you apply it' }: Props = $props();
;
async () => {

 { svelteHTML.createElement("span", {  "class":`chip agent-chip`,title,});  }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentChip__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentChip__SvelteComponent_ = ReturnType<typeof AgentChip__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentChip__SvelteComponent_;