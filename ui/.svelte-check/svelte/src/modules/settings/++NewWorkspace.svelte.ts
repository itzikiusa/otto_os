///<reference types="svelte" />
;// Add-workspace sheet: name + project directory. The backend expands `~`
// and creates the directory if missing, then makes the creator its admin.

import Modal from '../../lib/components/Modal.svelte';
import { toastError } from '../../lib/toastError';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { router } from '../../lib/router.svelte';
import { toasts } from '../../lib/toast.svelte';
;

  interface Props {
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { onclose }: Props = $props();

  let name = $state('');
  let rootPath = $state('~/');
  let busy = $state(false);
  let pickerOpen = $state(false);

  // Suggest a workspace name from the last path segment as the user types,
  // unless they've already typed a name themselves.
  let nameTouched = $state(false);
  $effect(() => {
    if (nameTouched) return;
    const seg = rootPath.replace(/\/+$/, '').split('/').pop() ?? '';
    if (seg !== '' && seg !== '~') name = seg;
  });

  const canCreate = $derived(name.trim() !== '' && rootPath.trim() !== '' && !busy);

  async function create(): Promise<void> {
    if (!canCreate) return;
    busy = true;
    try {
      const w = await ws.createWorkspace(name, rootPath);
      onclose();
      router.go('agents');
      toasts.success('Workspace created', w.name);
    } catch (e) {
      toastError('Couldn’t create workspace', e);
    } finally {
      busy = false;
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void create();
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },"title":`Add Workspace`,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!canCreate,"onclick":create,});
      busy ? 'Creating…' : 'Create Workspace';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`nw-name`,});  }
     { svelteHTML.createElement("input", {              "dir":`auto`,"id":`nw-name`,"class":`input`,"bind:value":name,"oninput":() => (nameTouched = true),"onkeydown":onKeydown,"placeholder":`My Project`,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`nw-path`,});  }
     { svelteHTML.createElement("div", { "class":`path-row`,});
       { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`nw-path`,"class":`input mono`,"bind:value":rootPath,"spellcheck":`false`,"onkeydown":onKeydown,"placeholder":`~/code/my-project`,});/*Ωignore_startΩ*/() => rootPath = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {     "class":`btn`,"type":`button`,"onclick":() => (pickerOpen = true),});  }
     }
     { svelteHTML.createElement("span", { "class":`hint`,});            { svelteHTML.createElement("code", {});  }     }
   }

  
 Modal}

if(pickerOpen){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Choose project directory`,"start":rootPath,"onpick":(p) => {
      rootPath = p;
      nameTouched = false;
      pickerOpen = false;
    },"onclose":() => (pickerOpen = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NewWorkspace__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NewWorkspace__SvelteComponent_ = ReturnType<typeof NewWorkspace__SvelteComponent_>;
/*Ωignore_endΩ*/export default NewWorkspace__SvelteComponent_;