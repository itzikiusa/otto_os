///<reference types="svelte" />
;// Otto School — the accessible companion list (ClassroomsBox): the
// "needs you" section and every classroom with its kids as real buttons,
// paged so a big workspace never mounts every row at once. Visible in List
// view; in 3D it stays keyboard / screen-reader reachable.

import { rowMenu } from '../../../../lib/rowMenu';
import Icon from '../../../../lib/components/Icon.svelte';
import EmptyState from '../../../../lib/components/EmptyState.svelte';
import ProviderIcon from '../../../../lib/components/ProviderIcon.svelte';
import { ui } from '../../../../lib/stores/ui.svelte';
import { plural } from '../../../../lib/plural';
import { relTime } from '../../../mission-control/lib';
import { kidAriaLabel, roomSummary, type Kid, type School } from '../../school/model';
import type { SchoolHandle } from '../../school/scene';
;

  interface Props {
    mode: '3d' | 'list';
    empty: boolean;
    school: School;
    queue: Kid[];
    handle: SchoolHandle | null;
    /** Current page per classroom (kept by the box across re-mounts). */
    memberPages: Record<string, number>;
    open: (k: Kid) => Promise<void>;
    /** Release from detention (un-archive): an explicit action, never a row click. */
    free: (k: Kid) => Promise<void>;
    goTo: (k: Kid) => Promise<void>;
    kidMenu: (e: MouseEvent | KeyboardEvent, k: Kid) => void;
    showIn3d: (k: Kid) => Promise<void>;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { mode, empty, school, queue, handle, memberPages = $bindable(), open, free, goTo, kidMenu, showIn3d }: Props = $props()/*Ωignore_startΩ*/;memberPages;/*Ωignore_endΩ*/;
  /** A detention row's main click shows the kid (its card in 3D, its actions in
   *  List view); it never un-archives the session. */
  function rowClick(e: MouseEvent, k: Kid): void {
    if (!k.detention) void open(k);
    else if (mode === '3d' && handle) void showIn3d(k);
    else kidMenu(e, k);
  }

  // Keep complete membership without mounting every session row at once.
  const LIST_PAGE_SIZE = 100;
;
async () => {

 { svelteHTML.createElement("div", {      "class":`list`,"role":`region`,"aria-label":`School list`,});mode === '3d';
  if(mode === 'list' && empty){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"icon":`people`,"title":`School’s out`,"body":`Start a session and a kid takes a seat in its workspace’s classroom.`,}});
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (ui.newSessionOpen = true),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
     EmptyState}
  }
  if(queue.length > 0){
     { svelteHTML.createElement("section", {   "class":`lroom lattn`,"aria-label":`Needs you`,});
       { svelteHTML.createElement("h4", { "class":`lroom-head`,}); { svelteHTML.createElement("span", { "class":`ellipsis`,});  } { svelteHTML.createElement("small", {});plural(queue.length, 'session'); } }
       { svelteHTML.createElement("ul", {});
           for(let k of __sveltets_2_ensureArray(queue)){k.id;
           { svelteHTML.createElement("li", {   "class":`lrow`,...__sveltets_2_empty({"data-attn-id":k.id}),});
             { svelteHTML.createElement("button", {     "class":`lmain`,"aria-label":`Go to ${kidAriaLabel(k, relTime(k.lastActiveAt))} in ${k.workspaceName}`,"onclick":() => void goTo(k),});
               { svelteHTML.createElement("i", {   "class":`dot`,...__sveltets_2_empty({"data-pose":k.pose}),}); }
               { const $$_nocIredivorP5C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP5C({ target: __sveltets_2_any(), props: {    "provider":k.provider,"size":12,}});}
               { svelteHTML.createElement("span", { "class":`ellipsis t`,});k.title; }
               { svelteHTML.createElement("span", { "class":`st`,});k.workspaceName;  k.stateLabel; }
             }
           }
        }
       }
     }
  }
     for(let r of __sveltets_2_ensureArray(school.rooms)){r.id;
    const lastPage = Math.max(0, Math.ceil(r.members.length / LIST_PAGE_SIZE) - 1);
    const page = Math.min(memberPages[r.id] ?? 0, lastPage);
    if(r.members.length > 0 || mode === 'list'){
       { svelteHTML.createElement("section", {   "class":`lroom`,"aria-label":`Classroom ${r.name}`,});
         { svelteHTML.createElement("h4", { "class":`lroom-head`,});
           { svelteHTML.createElement("span", { "class":`ellipsis`,});r.name; }
          if(r.current){ { svelteHTML.createElement("span", { "class":`cur`,});  }}
           { svelteHTML.createElement("small", {});roomSummary(r); }
          if(mode === '3d'){
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void handle?.enterRoom(r.id),"disabled":!handle,});  }
          }
         }
        if(r.members.length > 0){
           { svelteHTML.createElement("ul", {});
               for(let k of __sveltets_2_ensureArray((r.members.slice(page * LIST_PAGE_SIZE, (page + 1) * LIST_PAGE_SIZE)))){k.id;
               {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('li')));{ svelteHTML.createElement("li", __sveltets_2_union($$action_0), {      "class":`lrow`,...__sveltets_2_empty({"data-kid-id":k.id}),"oncontextmenu":(e) => kidMenu(e, k),});
                 { svelteHTML.createElement("button", {         "class":`lmain`,"aria-label":`${k.detention ? 'Show' : 'Open'} ${kidAriaLabel(k, relTime(k.lastActiveAt))}${k.detention ? ', in detention' : ''}`,"onclick":(e) => rowClick(e, k),"onfocus":() => void showIn3d(k),});
                   { svelteHTML.createElement("i", {   "class":`dot`,...__sveltets_2_empty({"data-pose":k.detention ? 'bench' : k.pose}),}); }
                   { const $$_nocIredivorP5C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP5C({ target: __sveltets_2_any(), props: {    "provider":k.provider,"size":12,}});}
                   { svelteHTML.createElement("span", { "class":`ellipsis t`,});k.title; }
                  if(k.background){ { svelteHTML.createElement("span", { "class":`bg-tag`,});  }}
                  if(k.detention){ { svelteHTML.createElement("span", { "class":`bg-tag`,});  }}
                   { svelteHTML.createElement("span", { "class":`st`,});k.stateLabel; }
                 }
                if(k.detention){
                   { svelteHTML.createElement("button", {           "class":`btn small`,"disabled":!k.canManage,"title":k.canManage ? 'Bring this session back from the archive' : 'You can’t manage this session, so you can’t release it',"aria-label":`Release ${k.title} from detention`,"onclick":() => void free(k),});
                    
                   }
                }
                 { svelteHTML.createElement("button", {         "class":`icon-btn`,"aria-label":`More actions for ${k.title}`,"title":`More actions for ${k.title}`,"aria-haspopup":`menu`,"onclick":(e) => kidMenu(e, k),});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});}
                 }
               }}
            }
           }
        }
        if(lastPage > 0){
           { svelteHTML.createElement("nav", {   "class":`member-pages`,"aria-label":`Sessions in classroom ${r.name}`,});
             { svelteHTML.createElement("button", {       "class":`btn small`,"aria-label":`Previous sessions in ${r.name}`,"disabled":page === 0,"onclick":() => (memberPages[r.id] = page - 1),});  }
             { svelteHTML.createElement("span", { "aria-live":`polite`,});page * LIST_PAGE_SIZE + 1; Math.min((page + 1) * LIST_PAGE_SIZE, r.members.length);  r.members.length; }
             { svelteHTML.createElement("button", {       "class":`btn small`,"aria-label":`Next sessions in ${r.name}`,"disabled":page === lastPage,"onclick":() => (memberPages[r.id] = page + 1),});  }
           }
        }
        if(r.overflow > 0){ { svelteHTML.createElement("p", { "class":`more`,});r.overflow;     }}
       }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('memberPages'), slots: {}, events: {} }}
const SchoolList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SchoolList__SvelteComponent_ = ReturnType<typeof SchoolList__SvelteComponent_>;
/*Ωignore_endΩ*/export default SchoolList__SvelteComponent_;