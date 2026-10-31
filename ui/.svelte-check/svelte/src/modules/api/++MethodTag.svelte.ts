///<reference types="svelte" />
;
  /** The word the tag shows (DELETE/OPTIONS shortened to fit the column) —
   *  also what a list's ⌘F `findText` should use for this cell. */
  export function methodWord(method: string): string {
    const m = (method || 'GET').toUpperCase();
    return m === 'DELETE' ? 'DEL' : m === 'OPTIONS' ? 'OPT' : m;
  }
;;// The HTTP method as a small mono word, toned by what it does (read /
// create / change / destroy — see methodTone). The word itself is the
// signal; colour only reinforces it.


import { methodTone } from '../../lib/api/apiVars';
;

  interface Props {
    method: string;
    /** Fixed-width column in lists so names line up. */
    fixed?: boolean;
  };function $$render() {

  
  
  
  

  let { method, fixed = false }: Props = $props();
  const m = $derived((method || 'GET').toUpperCase());
;
async () => {



 { svelteHTML.createElement("span", {  "class":`mtag ${methodTone(m)}`,});fixed;methodWord(m); }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MethodTag__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MethodTag__SvelteComponent_ = ReturnType<typeof MethodTag__SvelteComponent_>;
/*Ωignore_endΩ*/export default MethodTag__SvelteComponent_;