///<reference types="svelte" />
;// "Sign in" sheet: hosts the `aws sso login --profile …` PTY session the
// daemon spawned (`POST /aws/accounts/{id}/login`) in a live <Terminal>, and
// polls `/test` every 3 s until credentials work — then refreshes the account's
// permission probe and closes itself.

import { onDestroy } from 'svelte';
import Modal from '../../lib/components/Modal.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import { awsApi } from '../../lib/api/aws';
import { aws } from '../../lib/stores/aws.svelte';
import { toasts } from '../../lib/toast.svelte';
;

  interface Props {
    accountId: string;
    sessionId: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { accountId, sessionId, onclose }: Props = $props();

  const account = $derived(aws.account(accountId));
  let lastMsg = $state('Waiting for the browser sign-in to complete…');
  let done = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let closed = false;

  async function poll(): Promise<void> {
    if (closed) return;
    try {
      const r = await awsApi.test(accountId);
      if (r.ok) {
        done = true;
        lastMsg = `Signed in${r.identity ? ` as ${r.identity.arn}` : ''}.`;
        toasts.success('AWS sign-in complete', account?.name ?? accountId);
        void aws.loadPermissions(accountId, true);
        timer = setTimeout(() => {
          if (!closed) onclose();
        }, 900);
        return;
      }
      lastMsg = r.login_required ? 'Still waiting for sign-in…' : r.message;
    } catch (e) {
      lastMsg = e instanceof Error ? e.message : String(e);
    }
    timer = setTimeout(() => void poll(), 3000);
  }

  // First probe after a short grace period (the PTY needs a moment to print
  // the device-code URL).
  timer = setTimeout(() => void poll(), 3000);

  onDestroy(() => {
    closed = true;
    if (timer) clearTimeout(timer);
  });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Sign in — ${account?.name ?? 'AWS'}`,"width":760,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("p", { "class":`hint`,});
                   
       
   }
   { svelteHTML.createElement("div", { "class":`term`,});
    sessionId; {
       { const $$_lanimreTyzaL2C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL2C({ target: __sveltets_2_any(), props: {   sessionId,"preferDom":true,"autoFocus":true,"showToolbar":false,}});}
    }
   }
   { svelteHTML.createElement("p", {    "class":`status`,"aria-live":`polite`,});done;lastMsg; }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LoginSheet__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LoginSheet__SvelteComponent_ = ReturnType<typeof LoginSheet__SvelteComponent_>;
/*Ωignore_endΩ*/export default LoginSheet__SvelteComponent_;