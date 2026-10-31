///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { badgeTone, runStatus } from '../../lib/status';
import Badge from '../../lib/components/Badge.svelte';
import { onMount, untrack } from 'svelte';
import { sentenceCase, runStateLabel } from '../../lib/labels';
import { pollWhileVisible } from '../../lib/poll';
import { databaseChangesApi as changesApi, type DatabaseChange, type ChangeDetail, type ChangeInput, type ChangeTarget } from '../../lib/api/database-changes';
import { database } from '../../lib/stores/database.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { plural } from '../../lib/plural';
import { loadErrorText } from '../../lib/loadError';

;type $$ComponentProps = {connectionId:string;node?:string|null};function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  let {connectionId,node=null}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/=$props();
  let changes:DatabaseChange[]=$state([]);
  let detail:ChangeDetail|null=$state(null);
  let title=$state(''), description=$state(''), script=$state(''), targetDb=$state('');
  let draftTargets:ChangeTarget[]=$state([]);
  let executors:{id:string;display_name:string;username:string}[]=$state([]);
  let executor=$state(''), note=$state(''), error=$state(''), busy=$state(false), editing=$state(false);
  // `loadError` is the LIST load (inline, with Retry); `error` is the last failed ACTION.
  let loading=$state(true), loadError=$state('');
  // Which action is in flight, so only that button reads "Saving…" / "Working…".
  let pending=$state('');
  let generation=0;
  const selected:DatabaseChange|null=$derived.by(()=>detail?.change ?? null);
  // The unselected main pane summarises the collection instead of a bare "pick one":
  // "5 changes · 2 awaiting review · 1 approved".
  const changesSummary=$derived.by(()=>{
    const count=(status:string)=>changes.filter(c=>c.status===status).length;
    const parts=[plural(changes.length,'change')];
    const review=count('awaiting_review'), approved=count('approved'), unknown=count('outcome_unknown');
    if(review)parts.push(`${review} awaiting review`);
    if(approved)parts.push(`${approved} approved`);
    if(unknown)parts.push(`${unknown} outcome unknown`);
    return parts.join(' · ');
  });
  const child=$derived((node ?? '').replace(/^db:/,'').split('/')[0] || undefined);
  const owner=$derived(selected?.author_id===auth.me?.id);
  const independent=$derived(selected !== null && ![selected.author_id,selected.real_author_id].includes(auth.me?.id ?? '') && ![selected.author_id,selected.real_author_id].includes(auth.realUser?.id ?? ''));
  function can(operation:string) {
    const targets=editing ? draftTargets : selected?.targets ?? [{connection_id:connectionId,node:targetDb || child || ''}];
    return targets.every(t=>resourceAccess.can('connection',t.connection_id,operation,'database',operation==='change_submit'?'edit':'admin',t.node.replace(/^db:/,'') || undefined));
  }
  async function refresh() {
    const mine=++generation;
    // Only the first load shows the skeleton; polls refresh quietly (a failed one raises the stale bar).
    loading=changes.length===0;
    try {
      const rows=await changesApi.list(connectionId);
      if(mine!==generation)return;
      changes=rows;
      loadError='';
      if(selected){
        const next=await changesApi.get(selected.id);
        if(mine===generation){detail=next; await loadCapabilities(next.change);}
      }
    } catch(e) {
      if(mine===generation)loadError=loadErrorText(e);
    } finally {
      if(mine===generation)loading=false;
    }
  }
  async function loadCapabilities(change:DatabaseChange) {
    await Promise.all(change.targets.map(t=>resourceAccess.load('connection',t.connection_id,t.node.replace(/^db:/,''))));
  }
  async function select(change:DatabaseChange) {
    editing=false;error='';const mine=++generation;
    try {
      const next=await changesApi.get(change.id);
      if(mine!==generation)return;
      detail=next;
      executor=next.change.executor_id ?? auth.me?.id ?? '';
      await loadCapabilities(next.change);
      if(next.change.author_id===auth.me?.id){executors=await changesApi.executors(next.change.id);}
    } catch(e) {
      if(mine===generation)error=String(e);
    }
  }
  function draft() {
    detail=null;title='';description='';script='';
    targetDb=child ?? '';executor=auth.me?.id ?? '';
    editing=true;draftTargets=[{connection_id:connectionId,node:child ?? ''}];
    note='';error='';
  }
  function revise() {
    if(!selected)return;
    title=selected.title;description=selected.description;script=selected.script;
    targetDb=selected.targets[0]?.node.replace(/^db:/,'') ?? '';
    draftTargets=structuredClone($state.snapshot(selected.targets));
    editing=true;
  }
  async function perform(key:string,work:()=>Promise<DatabaseChange>) {
    busy=true;pending=key;error='';
    try {
      const change=await work();
      editing=false;
      await select(change);
      await refresh();
    } catch(e) {
      error=String(e);
    } finally {
      busy=false;pending='';
    }
  }
  /** Button label: the idle verb, or its in-flight form while that action runs. */
  const label=(key:string,idle:string,active='Working…')=>pending===key?active:idle;
  function save() {
    const input:ChangeInput={title,description,script,targets:draftTargets};
    void perform('save',()=>selected ? changesApi.revise(selected.id,selected.revision,input):changesApi.create(input));
  }
  function action(which:'submit'|'approve'|'reject'|'execute'|'cancel') {if(selected)void perform(which,()=>changesApi.action(selected!.id,selected!.revision,which,note));}
  $effect(()=>{
    const id=connectionId;
    const currentChild=child;
    untrack(()=>{
      detail=null;editing=false;targetDb=currentChild ?? '';
      void resourceAccess.load('connection',id,currentChild);
      void refresh();
    });
  });
  $effect(()=>{
    for(const target of draftTargets){
      const db=target.node.replace(/^db:/,'');
      if(db)void resourceAccess.load('connection',target.connection_id,db);
    }
  });
  // Hidden-paused, jittered, backed-off polling (lib/poll.ts) instead of a raw 5 s interval.
  onMount(()=>{
    const poller=pollWhileVisible(async()=>{if(!busy&&!editing)await refresh();},{ms:5000,immediate:false,maxBackoff:8});
    return()=>{generation++;poller.stop();};
  });
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (error) announce(error, { assertive: true }); });
;
async () => {
 { svelteHTML.createElement("div", {   "class":`changes`,...__sveltets_2_empty({"data-testid":`database-changes`}),});
   { svelteHTML.createElement("div", { "class":`toolbar`,});
     { svelteHTML.createElement("div", {});
       { svelteHTML.createElement("h2", {});  }
       { svelteHTML.createElement("p", {});        }
     }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":draft,"disabled":busy || !resourceAccess.can('connection',connectionId,'change_submit','database','edit',child),});  }
     { svelteHTML.createElement("button", {     "class":`btn`,"onclick":()=>void refresh(),"disabled":busy,});  }
   }
  if(error){ { svelteHTML.createElement("div", { "class":`error`,});error; }}
   { svelteHTML.createElement("div", { "class":`layout`,});
     { svelteHTML.createElement("nav", { "aria-label":`Database changes`,});
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`database changes`,loading,"error":loadError,"empty":changes.length===0,"onretry":()=>void refresh(),"variant":`compact`,emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", { "class":`hint`,});    }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
        
           for(let change of __sveltets_2_ensureArray(changes)){change.id;
           { svelteHTML.createElement("button", {        "class":`item`,"aria-current":selected?.id===change.id ? 'true' : undefined,"onclick":()=>void select(change),"disabled":busy,});selected?.id===change.id;
             { svelteHTML.createElement("strong", {});change.title; }
             { svelteHTML.createElement("span", {});sentenceCase(change.status);  change.revision; }
           }
        }
       LoadState}
     }
     { svelteHTML.createElement("main", {});
      if(editing){
         { svelteHTML.createElement("h3", {});selected ? 'Revise change' : 'New change'; }
         { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":title,"maxlength":200,});/*Ωignore_startΩ*/() => title = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("label", {});  { svelteHTML.createElement("textarea", {       "dir":`auto`,"bind:value":description,"rows":2,"maxlength":16384,});/*Ωignore_startΩ*/() => description = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
         { svelteHTML.createElement("fieldset", {});
           { svelteHTML.createElement("legend", {});  }
            for(let target of __sveltets_2_ensureArray(draftTargets)){let index = 1;
             { svelteHTML.createElement("div", { "class":`target-row`,});
               { svelteHTML.createElement("label", {}); index+1;
                 { svelteHTML.createElement("select", { "bind:value":target.connection_id,});/*Ωignore_startΩ*/() => target.connection_id = __sveltets_2_any(null);/*Ωignore_endΩ*/
                    for(let conn of __sveltets_2_ensureArray((database.connections.filter(c=>['mysql','postgres'].includes(c.kind))))){ { svelteHTML.createElement("option", { "value":conn.id,});conn.name; }}
                 }
               }
               { svelteHTML.createElement("label", {}); index+1; { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":target.node,"placeholder":`Exact database name`,});/*Ωignore_startΩ*/() => target.node = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
              if(draftTargets.length>1){
                 { svelteHTML.createElement("button", {     "class":`btn`,"aria-label":`Remove target ${index+1}`,"onclick":()=>draftTargets=draftTargets.filter((_,i)=>i!==index),});  }
              }
             }
          }
           { svelteHTML.createElement("button", {     "class":`btn`,"disabled":draftTargets.length>=20,"onclick":()=>draftTargets=[...draftTargets,{connection_id:connectionId,node:''}],});  }
         }
         { svelteHTML.createElement("label", {});  { svelteHTML.createElement("textarea", {             "dir":`ltr`,"class":`script`,"bind:value":script,"rows":14,"spellcheck":`false`,"maxlength":262144,"placeholder":`ALTER TABLE …`,});/*Ωignore_startΩ*/() => script = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
         { svelteHTML.createElement("p", { "class":`hint`,});                   }
         { svelteHTML.createElement("div", { "class":`actions`,});
           { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":save,"disabled":busy || !title.trim() || !script.trim() || draftTargets.some(t=>!t.node.trim()) || !can('change_submit'),});label('save','Save draft','Saving…'); }
           { svelteHTML.createElement("button", {     "class":`btn`,"onclick":()=>editing=false,"disabled":busy,});  }
         }
      } else if (selected && detail){
         { svelteHTML.createElement("div", { "class":`title`,}); { svelteHTML.createElement("h3", {});selected.title; } { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":badgeTone(runStatus(selected.status).tone),"label":sentenceCase(selected.status),}});} }
         { svelteHTML.createElement("p", {});selected.description; }
         { svelteHTML.createElement("p", { "class":`hint`,}); selected.revision;   selected.author_id; }
         { svelteHTML.createElement("div", { "class":`targets`,});
            for(let target of __sveltets_2_ensureArray(selected.targets)){ { svelteHTML.createElement("span", {});target.node.replace(/^db:/,'');  { svelteHTML.createElement("small", {}); target.connection_id;  } }}
         }
         { svelteHTML.createElement("pre", { "class":`approved-script`,});selected.script; }
        if(selected.content_hash){
           { svelteHTML.createElement("details", {});
             { svelteHTML.createElement("summary", {});  }
             { svelteHTML.createElement("p", {}); selected.executor_id; }
             { svelteHTML.createElement("code", {});selected.content_hash; }
             { svelteHTML.createElement("p", {});                }
           }
        }
        if(['draft','validated','rejected'].includes(selected.status) && owner && can('change_submit')){
           { svelteHTML.createElement("label", {});
             { svelteHTML.createElement("select", {   "aria-label":`Executor`,"bind:value":executor,});/*Ωignore_startΩ*/() => executor = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", {"value":"",});    }
                for(let user of __sveltets_2_ensureArray(executors)){ { svelteHTML.createElement("option", { "value":user.id,});user.display_name || user.username; }}
             }
           }
           { svelteHTML.createElement("div", { "class":`actions`,});
             { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy||!executor.trim(),"onclick":()=>void perform('validate',()=>changesApi.validate(selected!.id,selected!.revision,executor.trim())),});label('validate','Validate for executor'); }
            if(selected.status==='validated'){
               { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy,"onclick":()=>action('submit'),});label('submit','Submit for review'); }
            }
           }
        }
        if(selected.status==='awaiting_review' && can('change_approve')){
           { svelteHTML.createElement("label", {});  { svelteHTML.createElement("textarea", {       "dir":`auto`,"bind:value":note,"rows":2,"placeholder":`Decision and reason`,});/*Ωignore_startΩ*/() => note = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
           { svelteHTML.createElement("div", { "class":`actions`,});
             { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy||!independent,"onclick":()=>action('approve'),});label('approve','Approve revision'); }
             { svelteHTML.createElement("button", {     "class":`btn danger`,"disabled":busy||!note.trim(),"onclick":()=>action('reject'),});label('reject','Request revision'); }
           }
          if(!independent){ { svelteHTML.createElement("p", { "class":`hint`,});       }}
        }
        if(selected.status==='approved' && can('change_execute')){
           { svelteHTML.createElement("p", { "class":`hint`,});               }
           { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || selected.executor_id!==auth.me?.id,"onclick":()=>action('execute'),});label('execute','Execute approved revision'); }
        }
         { svelteHTML.createElement("div", { "class":`actions`,});
          if(owner && ['draft','validated','awaiting_review','approved','rejected'].includes(selected.status) && can('change_submit')){
             { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":revise,});  }
             { svelteHTML.createElement("button", {     "class":`btn danger`,"disabled":busy,"onclick":()=>action('cancel'),});label('cancel','Cancel change'); }
          }
          if(selected.status==='running' && can('change_execute')){
             { svelteHTML.createElement("button", {     "class":`btn danger`,"disabled":busy,"onclick":()=>action('cancel'),});label('cancel','Request cancellation'); }
          }
         }
        if(detail.attempts.length){
           { svelteHTML.createElement("h4", {});  }
            for(let attempt of __sveltets_2_ensureArray(detail.attempts)){
             { svelteHTML.createElement("article", {});
               { svelteHTML.createElement("strong", {});attempt.node?.replace(/^db:/,''); } { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":badgeTone(runStatus(attempt.state).tone),"label":runStateLabel(attempt.state),}});}
               { svelteHTML.createElement("p", {});attempt.summary; }
              if(attempt.state==='outcome_unknown' && selected.status==='outcome_unknown' && can('change_execute')){
                 { svelteHTML.createElement("p", { "class":`hint`,});             }
                 { svelteHTML.createElement("label", {});  { svelteHTML.createElement("textarea", {       "dir":`auto`,"bind:value":note,"rows":2,"placeholder":`Describe the schema/data checks you performed`,});/*Ωignore_startΩ*/() => note = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
                 { svelteHTML.createElement("div", { "class":`actions`,});
                   { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy||note.trim().length<10,"onclick":()=>void perform('rec-succeeded',()=>changesApi.reconcile(selected!.id,selected!.revision,attempt.id,'succeeded',note)),});label('rec-succeeded','Record applied'); }
                   { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy||note.trim().length<10,"onclick":()=>void perform('rec-failed',()=>changesApi.reconcile(selected!.id,selected!.revision,attempt.id,'failed',note)),});label('rec-failed','Record not applied'); }
                   { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy||note.trim().length<10,"onclick":()=>void perform('rec-partial',()=>changesApi.reconcile(selected!.id,selected!.revision,attempt.id,'partially_applied',note)),});label('rec-partial','Record partially applied'); }
                 }
              }
             }
          }
        }
         { svelteHTML.createElement("details", {});
           { svelteHTML.createElement("summary", {}); detail.history.length;  }
            for(let event of __sveltets_2_ensureArray(detail.history)){
             { svelteHTML.createElement("p", {});new Date(event.created_at).toLocaleString();  sentenceCase(event.action);  event.actor_id;event.real_actor_id!==event.actor_id ? ` (via ${event.real_actor_id})` : ''; }
          }
         }
      } else if (!loading && !loadError){
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {       "icon":`db`,"title":changes.length ? changesSummary : 'No database changes yet',"body":changes.length ? 'Select one to review its script, approvals and execution history.' : 'A change carries a reviewed SQL script from draft to approved run.',}});}
      }
     }
   }
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DatabaseChanges__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DatabaseChanges__SvelteComponent_ = ReturnType<typeof DatabaseChanges__SvelteComponent_>;
/*Ωignore_endΩ*/export default DatabaseChanges__SvelteComponent_;