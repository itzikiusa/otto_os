///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { radioKey } from '../../lib/radioKey';
// "New design" sheet: title, studio (+ format where a studio has several),
// an optional starter template, the project to file it in and the story it
// implements. Creates a draft and hands its id back — nothing is generated.
import { untrack } from 'svelte';
import Modal from '../../lib/components/Modal.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import type { DesignStudio } from '../../lib/api/types';
import { STUDIOS, formatLabel, studioInfo } from './model';
import { createDesign, templatesFor } from './create';
import { library } from './library.svelte';
;

  interface Props {
    studio?: DesignStudio;
    format?: string;
    templateId?: string;
    projectId?: string | null;
    storyId?: string | null;
    onclose: () => void;
    oncreated: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    studio: initialStudio = 'frames',
    format: initialFormat,
    templateId: initialTemplate = '',
    projectId: initialProject = null,
    storyId: initialStory = null,
    onclose,
    oncreated,
  }: Props = $props();

  /** Studios a person can start in today (Site and Spatial are planned). */
  const creatable = STUDIOS.filter((s) => s.formats.length > 0);

  // The sheet is seeded once from its props; after that the fields are the user's.
  const seed = untrack(() => ({
    studio: initialStudio,
    format: initialFormat ?? studioInfo(initialStudio).formats[0] ?? 'html',
    templateId: initialTemplate,
    projectId: initialProject ?? '',
    storyId: initialStory ?? '',
  }));
  let title = $state('');
  let studio = $state<DesignStudio>(seed.studio);
  let format = $state(seed.format);
  let templateId = $state(seed.templateId);
  let projectId = $state(seed.projectId);
  let storyId = $state(seed.storyId);
  let busy = $state(false);
  let error = $state<string | null>(null);

  const formats = $derived(studioInfo(studio).formats);
  const templates = $derived(templatesFor(format));
  const stories = $derived(
    Object.values(library.stories).sort((a, b) => a.source_key.localeCompare(b.source_key)),
  );
  const projects = $derived(library.projects.filter((p) => !p.archived));

  function pickStudio(s: DesignStudio): void {
    studio = s;
    const f = studioInfo(s).formats;
    if (!f.includes(format)) format = f[0] ?? 'html';
    templateId = '';
  }

  async function create(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      error = 'Pick a workspace first — new designs are filed under a workspace.';
      return;
    }
    busy = true;
    error = null;
    try {
      const id = await createDesign({
        workspaceId: wsId,
        studio,
        format,
        title: title || (templateId ? (templates.find((t) => t.id === templateId)?.name ?? '') : ''),
        projectId: projectId || null,
        storyId: storyId || null,
        templateId: templateId || undefined,
      });
      toasts.success('Draft created');
      oncreated(id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`New design`,"width":520,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":create,"disabled":busy,...__sveltets_2_empty({"data-testid":`design-new-create`}),});
      busy ? 'Creating…' : 'Create draft';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("form", {     "class":`form`,"onsubmit":(e) => {
      e.preventDefault();
      void create();
    },});
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`dh-new-title`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`dh-new-title`,"class":`input`,"bind:value":title,"placeholder":`Rewards landing hero`,});/*Ωignore_startΩ*/() => title = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("span", {   "class":`lbl`,"id":`dh-new-studio`,});  }
       { svelteHTML.createElement("div", {     "class":`studios`,"role":`radiogroup`,"aria-labelledby":`dh-new-studio`,});
           for(let s of __sveltets_2_ensureArray(creatable)){s.id;
           { svelteHTML.createElement("button", {               "type":`button`,"role":`radio`,"aria-checked":studio === s.id,"tabindex":studio === s.id ? 0 : -1,"onkeydown":radioKey,"class":`pill-toggle`,"onclick":() => pickStudio(s.id),});studio === s.id;s.name; }
        }
       }
       { svelteHTML.createElement("span", { "class":`hint`,});studioInfo(studio).note; }
     }
    if(formats.length > 1){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`dh-new-format`,});  }
         { svelteHTML.createElement("select", {       "id":`dh-new-format`,"class":`input`,"bind:value":format,"onchange":() => (templateId = ''),});/*Ωignore_startΩ*/() => format = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let f of __sveltets_2_ensureArray(formats)){f; { svelteHTML.createElement("option", { "value":f,});formatLabel(f); }}
         }
       }
    }
    if(templates.length){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`dh-new-template`,});  }
         { svelteHTML.createElement("select", {     "id":`dh-new-template`,"class":`input`,"bind:value":templateId,});/*Ωignore_startΩ*/() => templateId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});  }
             for(let t of __sveltets_2_ensureArray(templates)){t.id; { svelteHTML.createElement("option", { "value":t.id,});t.name;  t.description; }}
         }
       }
    }
     { svelteHTML.createElement("div", { "class":`row2`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`dh-new-project`,});  }
         { svelteHTML.createElement("select", {     "id":`dh-new-project`,"class":`input`,"bind:value":projectId,});/*Ωignore_startΩ*/() => projectId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});  }
             for(let p of __sveltets_2_ensureArray(projects)){p.id; { svelteHTML.createElement("option", { "value":p.id,});p.name; }}
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`dh-new-story`,});  }
         { svelteHTML.createElement("select", {         "id":`dh-new-story`,"class":`input`,"bind:value":storyId,"disabled":!stories.length,"title":stories.length ? undefined : 'No product stories are loaded for this workspace',});/*Ωignore_startΩ*/() => storyId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});  }
             for(let s of __sveltets_2_ensureArray(stories)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.source_key;  s.title; }}
         }
       }
     }
    if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`err`,});error; }}}
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NewDesignModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NewDesignModal__SvelteComponent_ = ReturnType<typeof NewDesignModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default NewDesignModal__SvelteComponent_;