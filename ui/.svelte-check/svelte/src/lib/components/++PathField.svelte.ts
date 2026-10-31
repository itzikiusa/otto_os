///<reference types="svelte" />
;
import type { Snippet } from 'svelte';
import FolderPicker from './FolderPicker.svelte';
;
  interface Props {
    value?: string;
    start?: string;
    files?: boolean;
    disabled?: boolean;
    onpick?: (path: string) => void;
    children: Snippet;
  };function $$render() {

  
  

  let { value = $bindable(''), start = '', files = false, disabled = false, onpick, children }: Props = $props()/*Ωignore_startΩ*/;value;/*Ωignore_endΩ*/;
  let open = $state(false);
  const initial = $derived.by(() => {
    const path = value && !value.includes('{{') ? value : start;
    return files && path.includes('/') ? path.slice(0, path.lastIndexOf('/')) || '/' : path;
  });
;
async () => {
 { svelteHTML.createElement("div", { "class":`path-field`,});
  ;__sveltets_2_ensureSnippet(children());
   { svelteHTML.createElement("button", {        "type":`button`,"class":`btn`,disabled,"title":files ? 'Browse files' : 'Browse folders',"onclick":() => (open = true),});  }
 }
if(open){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":files ? 'Choose file' : 'Choose folder',"start":initial,files,"onpick":(path) => { value = path; onpick?.(path); open = false; },"onclose":() => (open = false),}});}
}

};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('value'), slots: {}, events: {} }}
const PathField__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PathField__SvelteComponent_ = ReturnType<typeof PathField__SvelteComponent_>;
/*Ωignore_endΩ*/export default PathField__SvelteComponent_;