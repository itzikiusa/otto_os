///<reference types="svelte" />
;// The DB Assistant: a file-backed, embedded agent that investigates the active
// database connection. Its live SHELL (the same Terminal as Agents, reused) sits
// right here BESIDE the query editor/results — the user types directly to the
// agent in that terminal (a real two-way session), exactly like Canvas's
// ConversationPanel. The agent runs read-only against the DB via a seeded `q`
// tool and writes its proposed SQL, surfaced below with Insert / Run. The
// session is hidden from the Agents section (meta.source = 'db_assist').

import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Terminal from '../../lib/components/Terminal.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { radioKey } from '../../lib/radioKey';
import { database } from '../../lib/stores/database.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import type { DbAssistMode } from '../../lib/api/types';
import { allProviders } from '../../lib/providers';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let draft = $state('');

  // Surface a hint when a turn runs unusually long — the agent may be wedged
  // and Stop is the way out (it releases the input; the session stays live).
  let busyLong = $state(false);
  $effect(() => {
    if (!database.assistBusy) {
      busyLong = false;
      return;
    }
    const t = setTimeout(() => (busyLong = true), 20_000);
    return () => clearTimeout(t);
  });

  // Mode → the panel's title, empty-state hint, and the Ask placeholder.
  const MODE: Record<DbAssistMode, { title: string; hint: string; placeholder: string }> = {
    nl: {
      title: 'Ask in English',
      hint: 'Describe the query you want in plain English. The agent reads the full schema, can sample real data read-only, and proposes a runnable query below.',
      placeholder: 'e.g. top 10 customers by total order value last month',
    },
    ask: {
      title: 'Ask Otto',
      hint: 'Ask anything about this database — its schema, the data, or how to write a query. The agent inspects the live DB read-only and answers in its shell.',
      placeholder: 'Ask about the schema or the data…',
    },
    investigate: {
      title: 'Examine with Otto',
      hint: 'The agent is seeded with the current query and a sample of its result. Ask it to explain, dig in, or find an issue — it can sample more data read-only.',
      placeholder: 'What should the agent look into?',
    },
  };
  const info = $derived(MODE[database.assistMode]);

  // Provider picker (chosen BEFORE the first turn; the choice locks once a session
  // exists). Same source/defaulting as NewSession.
  const providers = $derived(allProviders());
  const defaultProvider = $derived(
    (typeof ws.current?.settings?.default_provider === 'string' &&
      ws.current.settings.default_provider) ||
      auth.meta?.default_provider ||
      '',
  );
  // Preselect the configured default agent when the panel opens with none chosen.
  $effect(() => {
    if (!database.assistProvider && providers.length > 0) {
      const def = defaultProvider && providers.includes(defaultProvider) ? defaultProvider : null;
      database.setAssistProvider(def ?? (providers.includes('claude') ? 'claude' : providers[0]));
    }
  });

  // Once the session is live the empty state (provider picker + Ask box) gives way
  // to the real, interactive terminal — that IS the conversation surface from here.
  const started = $derived(database.assistSessionId !== null);

  /** Closing discards the live session and its working files — confirm when
   *  there is a conversation or a proposal that would be lost. */
  async function close(): Promise<void> {
    if (database.assistSessionId || database.assistProposedSql.trim()) {
      const ok = await confirmer.ask(
        'Closing ends the agent’s session and deletes its working files, including the proposed query. Copy anything you want to keep first.',
        { title: 'Close the DB assistant?', confirmLabel: 'Close and discard', cancelLabel: 'Keep open', danger: true },
      );
      if (!ok) return;
    }
    await database.closeAssist();
  }

  async function send(): Promise<void> {
    const p = draft.trim();
    if (!p || database.assistBusy) return;
    draft = '';
    await database.startAssist(p);
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }
;
async () => {

 { svelteHTML.createElement("section", { "class":`db-assist`,});
   { svelteHTML.createElement("header", { "class":`da-head`,});
     { svelteHTML.createElement("span", { "class":`da-title`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":14,}});} info.title; }
    if(!started && providers.length > 1){
       { svelteHTML.createElement("select", {         "class":`da-provider`,"value":database.assistProvider,"onchange":(e) => database.setAssistProvider((e.currentTarget as HTMLSelectElement).value),"title":`Which agent investigates this database`,});
           for(let p of __sveltets_2_ensureArray(providers)){p;
           { svelteHTML.createElement("option", { "value":p,});p; }
        }
       }
    }
    if(database.assistBusy){
       { svelteHTML.createElement("span", { "class":`da-working`,}); { const $$_toDgnikroWeviL3C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL3C({ target: __sveltets_2_any(), props: {  "label":`Working…`,}});} }
      
       { svelteHTML.createElement("button", {       "class":`btn small da-stop`,"onclick":() => database.stopAssist(),"title":`Stop waiting on this turn — releases the input (a started session stays live)`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} 
       }
    }
     { svelteHTML.createElement("button", {         "class":`btn small da-summarize`,"onclick":() => void database.summarizeAssist(),"disabled":database.assistBusy || !database.assistId,"title":`Write a summary of this investigation and download it as Markdown`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":13,}});} 
     }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => void close(),"aria-label":`Close DB assistant`,"title":`Close — discards the session and working files`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
     }
   }

  
   { svelteHTML.createElement("div", { "class":`da-shell`,});
    if(database.assistSessionId){
      database.assistSessionId; {
          { const $$_lanimreT2C = __sveltets_2_ensureComponent(Terminal); new $$_lanimreT2C({ target: __sveltets_2_any(), props: {    "sessionId":database.assistSessionId,"readOnly":false,"forceDark":true,"preferDom":true,}});}
      }
    }else{
       { svelteHTML.createElement("div", { "class":`da-empty`,});
        
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`db`,"title":info.title,"body":info.hint,}});}
        if(providers.length > 1){
           { svelteHTML.createElement("div", {     "class":`da-providers`,"role":`radiogroup`,"aria-label":`Agent`,});
                for(let p of __sveltets_2_ensureArray(providers)){let pi = 1;p;
               { svelteHTML.createElement("button", {              "class":`da-prov`,"role":`radio`,"aria-checked":database.assistProvider === p,"tabindex":database.assistProvider === p || (pi === 0 && !providers.includes(database.assistProvider)) ? 0 : -1,"onkeydown":radioKey,"onclick":() => database.setAssistProvider(p),});database.assistProvider === p;p; }
            }
           }
        }
         { svelteHTML.createElement("div", { "class":`da-ask`,});
           { svelteHTML.createElement("textarea", {              "dir":`auto`,"aria-label":`Ask the database assistant`,"bind:value":draft,"onkeydown":onKey,"placeholder":info.placeholder,"rows":3,"disabled":database.assistBusy,});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           { svelteHTML.createElement("button", {     "class":`btn primary da-send`,"onclick":send,"disabled":database.assistBusy || !draft.trim(),});
            if(database.assistBusy){ }else{ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":14,}});} }
           }
         }
        if(busyLong){
           { svelteHTML.createElement("p", { "class":`sub warn`,});           
                    }
        }
         { svelteHTML.createElement("p", { "class":`sub`,});           
                   }
       }
    }
   }

  
  if(database.assistProposedSql.trim()){
     { svelteHTML.createElement("div", { "class":`da-sql`,});
       { svelteHTML.createElement("div", { "class":`da-sql-head`,});
         { svelteHTML.createElement("span", { "class":`da-sql-label`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`db`,"size":12,}});}   }
         { const $$_enilyBtnegA3C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA3C({ target: __sveltets_2_any(), props: {    "provider":database.assistProvider,"label":`Draft`,}});}
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => {
            database.assistProposedSql = '';
            database.assistNote = '';
          },"title":`Drop this proposed query (nothing is changed in your editor)`,});
          
         }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void database.insertAssistSql(),"title":`Put this query into the active editor tab`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}   
         }
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => void database.runAssistSql(),"title":`Insert this query into the editor and run it read-only — a write asks you first`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} 
         }
       }
       { svelteHTML.createElement("pre", { "class":`da-sql-text mono`,});database.assistProposedSql; }
      if(database.assistNote.trim()){
         { svelteHTML.createElement("div", { "class":`da-sql-note`,});database.assistNote; }
      }
     }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DbAssistantPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DbAssistantPanel__SvelteComponent_ = ReturnType<typeof DbAssistantPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default DbAssistantPanel__SvelteComponent_;