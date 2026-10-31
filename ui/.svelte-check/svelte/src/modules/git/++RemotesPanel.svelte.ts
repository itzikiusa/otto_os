///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// Remote CRUD for one repo (`GET|POST /repos/{id}/remotes`). URLs arrive with
// any `user:password@` userinfo already stripped by the daemon, so nothing
// credentialed is ever rendered here. Push/checkout still use `origin`
// explicitly — renaming or removing it does not re-point them.
import type { RemoteInfo, RemoteOpReq } from '../../lib/api/types';
import { loadErrorText } from '../../lib/loadError';
import { toastError } from '../../lib/toastError';
import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { invalidatePr } from './pr-cache';
;

  interface Props {
    repoId: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, onclose }: Props = $props();

  let remotes = $state<RemoteInfo[]>([]);
  let loading = $state(true);
  let busy = $state('');
  let error = $state<string | null>(null);
  /** The initial list load failed — rendered as an error with Retry instead of
   *  the misleading "This repository has no remotes" empty state. */
  let loadFailed = $state(false);
  let loadRev = $state(0);

  $effect(() => {
    const id = repoId;
    void loadRev;
    loading = true;
    loadFailed = false;
    api
      .get<RemoteInfo[]>(`/repos/${id}/remotes`)
      .then((rows) => {
        remotes = rows;
        error = null;
      })
      .catch((e: unknown) => {
        error = loadErrorText(e);
        loadFailed = true;
      })
      .finally(() => {
        loading = false;
      });
  });

  /** `label` is the success title; `verb` names the failure ("Couldn’t add the remote"). */
  async function send(req: RemoteOpReq, label: string, verb: string): Promise<void> {
    busy = req.name;
    error = null;
    try {
      remotes = await api.post<RemoteInfo[]>(`/repos/${repoId}/remotes`, req);
      if (req.name === 'origin') invalidatePr(repoId);
      loadFailed = false;
      toasts.success(label, req.name);
    } catch (e) {
      error = loadErrorText(e);
      toastError(`Couldn’t ${verb}`, e);
    } finally {
      busy = '';
    }
  }

  async function addRemote(): Promise<void> {
    const name = await confirmer.promptText('Name for the new remote', {
      title: 'Add remote',
      confirmLabel: 'Next',
      placeholder: 'upstream',
    });
    if (!name) return;
    const url = await confirmer.promptText(`Fetch/push URL for \`${name}\``, {
      title: 'Add remote',
      confirmLabel: 'Add',
      placeholder: 'https://github.com/owner/repo.git',
    });
    if (!url) return;
    await send({ op: 'add', name, url }, 'Remote added', 'add the remote');
  }

  async function editUrl(r: RemoteInfo): Promise<void> {
    const url = await confirmer.promptText(`New URL for \`${r.name}\``, {
      title: 'Edit remote URL',
      confirmLabel: 'Save',
      initial: r.fetch_url,
    });
    if (!url || url === r.fetch_url) return;
    await send({ op: 'set_url', name: r.name, url }, 'Remote updated', 'update the remote');
  }

  async function removeRemote(r: RemoteInfo): Promise<void> {
    const ok = await confirmer.ask(`Remove remote \`${r.name}\`? Local branches are kept.`, {
      title: 'Remove remote',
      confirmLabel: 'Remove',
      danger: true,
    });
    if (!ok) return;
    await send({ op: 'remove', name: r.name }, 'Remote removed', 'remove the remote');
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Remotes`,"width":520,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":addRemote,"disabled":!!busy,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`rp`,});
    if(loading){
       { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":2,"height":38,}});}
    } else if (loadFailed){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`remotes`,"error":error || 'No details were reported.',"empty":true,"onretry":() => loadRev++,}});}
    } else if (remotes.length === 0){
       { svelteHTML.createElement("p", { "class":`rp-empty`,});            }
    }else{
       { svelteHTML.createElement("ul", { "class":`rp-list`,});
           for(let r of __sveltets_2_ensureArray(remotes)){r.name;
           { svelteHTML.createElement("li", { "class":`rp-row`,});
             { svelteHTML.createElement("div", { "class":`rp-id`,});
               { svelteHTML.createElement("span", { "class":`rp-name`,});r.name; }
               { svelteHTML.createElement("span", {   "class":`mono rp-url`,"title":r.fetch_url,});r.fetch_url; }
              if(r.push_url && r.push_url !== r.fetch_url){
                 { svelteHTML.createElement("span", {   "class":`mono rp-url`,"title":r.push_url,}); r.push_url; }
              }
             }
             { svelteHTML.createElement("button", {     "class":`btn ghost small`,"disabled":busy === r.name,"onclick":() => editUrl(r),});
               
             }
             { svelteHTML.createElement("button", {       "class":`btn ghost small danger`,"disabled":busy === r.name,"onclick":() => removeRemote(r),});
              
             }
           }
        }
       }
    }

    if(error && !loadFailed){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`rp-err`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} error; }}
    }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RemotesPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RemotesPanel__SvelteComponent_ = ReturnType<typeof RemotesPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default RemotesPanel__SvelteComponent_;