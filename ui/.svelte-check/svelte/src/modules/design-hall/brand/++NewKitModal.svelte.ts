///<reference types="svelte" />
;// "New brand kit": a name and one of three starter kits (each a complete,
// contrast-checked otto-brand/1 document). Creates the kit as a draft in the
// current workspace — nothing leaves the Mac.

import Modal from '../../../lib/components/Modal.svelte';
import { toastError } from '../../../lib/toastError';
import Icon from '../../../lib/components/Icon.svelte';
import { createArtifact } from '../../../lib/api/design';
import { STARTER_KITS, type StarterKit } from './starters';
import { brandPalette, serializeBrandDoc } from './tokens';
;

  interface Props {
    workspaceId: string;
    projectId?: string | null;
    onclose: () => void;
    oncreated: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { workspaceId, projectId = null, onclose, oncreated }: Props = $props();

  let name = $state('');
  let starter = $state<StarterKit['id']>('vivid');
  let busy = $state(false);

  async function create(): Promise<void> {
    const title = name.trim() || 'Brand kit';
    const kit = STARTER_KITS.find((k) => k.id === starter) ?? STARTER_KITS[0];
    busy = true;
    try {
      const res = await createArtifact({
        workspace_id: workspaceId,
        studio: 'brand',
        format: 'otto-brand',
        title,
        content: serializeBrandDoc(kit.build(title)),
        message: `Started from the “${kit.label}” starter kit`,
        ...(projectId ? { project_id: projectId } : {}),
      });
      oncreated(res.artifact.id);
    } catch (e) {
      toastError('Couldn’t create the brand kit', e);
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`New brand kit`,"width":620,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":() => void create(),"disabled":busy,...__sveltets_2_empty({"data-testid":`brand-new-create`}),});busy ? 'Creating…' : 'Create brand kit'; }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("form", {       "class":`nk`,"onsubmit":(e) => {
      e.preventDefault();
      void create();
    },...__sveltets_2_empty({"data-testid":`brand-new-modal`}),});
     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"class":`input`,"bind:value":name,"placeholder":`Acme brand`,"maxlength":200,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("fieldset", {});
       { svelteHTML.createElement("legend", {});  }
       { svelteHTML.createElement("div", { "class":`kits`,});
           for(let k of __sveltets_2_ensureArray(STARTER_KITS)){k.id;
          const doc = k.build(k.label);
          const pal = brandPalette(doc);
           { svelteHTML.createElement("label", {  "class":`kit`,});starter === k.id;
             { svelteHTML.createElement("input", {       "type":`radio`,"name":`starter`,"value":k.id,});starter = __sveltets_2_any(null);}
             { svelteHTML.createElement("span", {   "class":`sws`,"aria-hidden":`true`,});
                  for(let c of __sveltets_2_ensureArray(([pal.primary, pal.accent, pal.ink, pal.surfaceAlt]))){let i = 1;i; { svelteHTML.createElement("i", { });__sveltets_2_ensureType(String, Number, c); }}
             }
             { svelteHTML.createElement("b", {});k.label; }
             { svelteHTML.createElement("span", { "class":`blurb`,});k.blurb; }
           }
        }
       }
     }
     { svelteHTML.createElement("p", { "class":`hint`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":13,}});}                    }
     { svelteHTML.createElement("button", {     "type":`submit`,"hidden":true,"aria-hidden":`true`,"tabindex":-1,}); }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NewKitModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NewKitModal__SvelteComponent_ = ReturnType<typeof NewKitModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default NewKitModal__SvelteComponent_;