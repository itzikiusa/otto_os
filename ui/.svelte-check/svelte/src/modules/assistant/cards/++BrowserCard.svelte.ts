///<reference types="svelte" />
;// A browser chore the assistant is doing in its own browser profile: a live
// thumbnail, the step list (done / now / next), and the hand-off controls —
// Watch (opens the live tab), Take over (pauses the agent so you can type a
// password, 2FA or CAPTCHA) and Hand back.

import ActionCard from './ActionCard.svelte';
import Badge from '../../../lib/components/Badge.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { authedBlobUrl } from '../../../lib/api/client';
import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
import { browser } from '../../../lib/stores/browser.svelte';
import { router } from '../../../lib/router.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { taskStateLabel, taskTone, type BrowserProgress } from '../../../lib/assistant/model';
import type { AssistantTask } from '../../../lib/api/types';
;

  interface Props {
    task: AssistantTask;
    progress: BrowserProgress;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { task, progress }: Props = $props();

  // `takeover` moves the task to needs_you with a `takeover` item until handback.
  const youHaveIt = $derived(task.state === 'needs_you' && (task.kind === 'takeover' || task.needs_you?.kind === 'takeover'));
  const finished = $derived(task.state === 'done' || task.state === 'failed' || task.state === 'cancelled');

  // Thumbnail: a daemon path is fetched with the bearer token; absolute URLs
  // and data: URIs are used as they are. No thumbnail → a quiet wireframe.
  let thumb = $state<string | null>(null);
  let thumbFailed = $state(false);
  $effect(() => {
    const src = progress.thumbnail_url;
    thumbFailed = false;
    if (!src) {
      thumb = null;
      return;
    }
    if (!src.startsWith('/')) {
      thumb = src;
      return;
    }
    let url: string | null = null;
    let live = true;
    authedBlobUrl(src)
      .then((u) => {
        if (live) thumb = url = u;
        else URL.revokeObjectURL(u);
      })
      .catch(() => {
        if (live) thumbFailed = true;
      });
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
    };
  });

  let busy = $state(false);
  async function run(action: 'takeover' | 'handback'): Promise<void> {
    busy = true;
    try {
      await assistant.act(task.id, action);
    } catch (e) {
      toasts.error(action === 'takeover' ? 'Couldn’t take over the browser' : 'Couldn’t hand the browser back', describeError(e));
    } finally {
      busy = false;
    }
  }
  function watch(): void {
    if (!progress.tab_id) return;
    browser.select(progress.tab_id);
    router.go('browser');
  }
;
async () => {

 { const $$_draCnoitcA0C = __sveltets_2_ensureComponent(ActionCard); const $$_draCnoitcA0 = new $$_draCnoitcA0C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"icon":`compass`,"heading":`Browser`,"summary":task.title,"attention":youHaveIt,"testid":`card-browser`,pill:() => { async ()/*Ωignore_positionΩ*/ => {
    if(youHaveIt && task.kind !== 'delegation'){
       { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`You have control`,}});}
    }else{
       { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {      "tone":taskTone(task),"label":taskStateLabel(task),"live":task.state === 'running',}});}
    }
  };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":watch,"disabled":!progress.tab_id,"title":progress.tab_id ? 'Open the live tab in Browser' : 'The live tab isn’t available yet',});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}  
     }
    if(youHaveIt && task.kind !== 'delegation'){
       { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => void run('handback'),"disabled":busy,});busy ? 'Handing back…' : 'Hand back'; }
    } else if (task.kind !== 'delegation'){
       { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void run('takeover'),"disabled":busy || finished,"title":finished ? 'This task has finished' : 'Pause Otto and use the browser yourself',});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`cursor`,"size":12,}});} busy ? 'Taking over…' : 'Take over';
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {pill, footer} = $$_draCnoitcA0.$$prop_def;/*Ωignore_endΩ*/
  
   { svelteHTML.createElement("div", { "class":`wrap`,}); { svelteHTML.createElement("div", { "class":`grid`,});
     { svelteHTML.createElement("figure", {   "class":`shot`,"aria-label":`Live browser thumbnail`,});
      if(progress.url){ { svelteHTML.createElement("figcaption", {     "class":`url mono`,"dir":`ltr`,"title":progress.url,});progress.url; }}
      if(thumb && !thumbFailed){
         { svelteHTML.createElement("img", {    "src":thumb,"alt":`Browser: ${task.title}`,});}
      }else{
         { svelteHTML.createElement("div", {   "class":`wire`,"aria-hidden":`true`,});
           { svelteHTML.createElement("span", {   "class":`b`,"style":`inset-block-start:10px;inset-inline-start:8px;width:40%;height:8px`,}); }
           { svelteHTML.createElement("span", {   "class":`b`,"style":`inset-block-start:26px;inset-inline-start:8px;width:28%;height:40px`,}); }
           { svelteHTML.createElement("span", {   "class":`b`,"style":`inset-block-start:26px;inset-inline-start:36%;width:56%;height:8px`,}); }
           { svelteHTML.createElement("span", {   "class":`b`,"style":`inset-block-start:40px;inset-inline-start:36%;width:40%;height:8px`,}); }
         }
      }
     }
     { svelteHTML.createElement("div", { "class":`side`,});
       { svelteHTML.createElement("ol", { "class":`steps`,});
            for(let s of __sveltets_2_ensureArray(progress.steps)){let i = 1;i;
           { svelteHTML.createElement("li", { "class":s.state,});
             { svelteHTML.createElement("span", {   "class":`mark`,"aria-hidden":`true`,});
              if(s.state === 'done'){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} else if (s.state === 'current'){ { svelteHTML.createElement("span", { "class":`mark-now`,}); }}else{ { svelteHTML.createElement("span", { "class":`mark-todo`,}); }}
             }
             { svelteHTML.createElement("span", { "class":`sr-only`,});s.state === 'done' ? 'Done:' : s.state === 'current' ? 'Now:' : 'Next:'; }
             { svelteHTML.createElement("span", {});s.label; }
           }
        }
       }
      if(youHaveIt && task.kind !== 'delegation'){
         { svelteHTML.createElement("p", { "class":`note warn`,});            }
      } else if (progress.note){
         { svelteHTML.createElement("p", { "class":`note`,});progress.note; }
      }
     }
   } }
  
 ActionCard}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrowserCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrowserCard__SvelteComponent_ = ReturnType<typeof BrowserCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrowserCard__SvelteComponent_;