///<reference types="svelte" />
;// The Design "Assistant": a specialized agent that builds a design artifact IN
// PLACE — its live shell (the same Terminal as Agents, reused) embedded right
// here on the Product page (never in the Agents section). Two layouts:
//   • full (default) — CREATING a new artifact: a live preview of the source
//     as the agent writes it, beside the shell + composer.
//   • embedded — REFINING the artifact open in the arena: the arena's viewport
//     already IS the live preview (edits arrive over mockup_updated), so this
//     is just the shell + composer, docked in the inspector column.
// The format chips cover every DesignFormat (§2.2); they lock once the
// artifact exists (a refine resumes the existing session and format).

import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
import { toastError } from '../../lib/toastError';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import MockupLivePreview from './MockupLivePreview.svelte';
import { mockupAssist } from '../../lib/stores/mockup-assist.svelte';
import type { DesignFormat, ProductAttachment } from './types';
import { DESIGN_FORMATS } from './types';
import { FORMATS } from '../../lib/design/format';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
;

  interface Props {
    /** Called after a turn commits, with the committed attachment. */
    oncommit?: (att: ProductAttachment) => void;
    onclose: () => void;
    /** Docked (shell + composer only) — see the header comment. */
    embedded?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const { oncommit, onclose, embedded = false }: Props = $props();

  let draft = $state('');

  // Provider for the agent session — honored only when a NEW artifact is
  // created (a refine resumes the existing session). Sourced from the live
  // registry so custom providers (e.g. grok) work.
  let provider = $state(defaultAgentProvider());

  // The format (and provider) can only be chosen before the artifact exists; once
  // it does (or we're refining), it's locked.
  const locked = $derived(mockupAssist.attachmentId !== null);

  /** Format-aware starters (design §4.1: "format-aware placeholder"). */
  const STARTERS: Record<DesignFormat, string[]> = {
    html: [
      'A dashboard with KPI cards and a recent-activity table',
      'A settings page with tabs for profile, security, and billing',
      'A sign-up form with validation states',
    ],
    mermaid: [
      'A sequence diagram of the checkout flow',
      'A state diagram for a withdrawal: queued → sent → settled',
      'An ER diagram of players, wallets and payouts',
    ],
    excalidraw: [
      'Wireframe two mobile screens as frames: onboarding and home',
      'A user-journey board with sticky notes grouped by stage',
      'A system context diagram with boxes and labeled arrows',
    ],
    scene3d: [
      'A greybox level: ground, three platforms, cover crates and a goal marker',
      'A product shot: a plinth, a hero box and three-point lighting',
      'A kiosk in a lobby with a floor, two walls and a screen',
    ],
  };
  const HINT: Record<DesignFormat, string> = {
    html: 'a self-contained HTML screen',
    mermaid: 'a Mermaid diagram',
    excalidraw: 'an Excalidraw board (frames as artboards, 8-pt grid)',
    scene3d: 'a scene3d document (metres, y-up, origin at the floor)',
  };

  function setFormat(f: DesignFormat): void {
    if (!locked) mockupAssist.format = f;
  }

  async function send(): Promise<void> {
    const p = draft.trim();
    if (!p || mockupAssist.busy) return;
    draft = '';
    try {
      const att = await mockupAssist.ask(p, provider);
      oncommit?.(att);
    } catch (e) {
      toastError('Couldn’t run the design agent', e);
    }
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }
  function useStarter(s: string): void {
    draft = s;
  }
;
async () => {

 { svelteHTML.createElement("section", {  "class":`mockup-assist`,});embedded;
   { svelteHTML.createElement("header", { "class":`ma-head`,});
     { svelteHTML.createElement("span", { "class":`ma-title`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":14,}});} embedded ? 'Assistant' : 'Design agent'; }
     { svelteHTML.createElement("div", {     "class":`ma-format`,"role":`group`,"aria-label":`Design format`,});
         for(let f of __sveltets_2_ensureArray(DESIGN_FORMATS)){f;
         { svelteHTML.createElement("button", {          "aria-pressed":mockupAssist.format === f,"disabled":locked,"onclick":() => setFormat(f),"title":FORMATS[f].hint,});mockupAssist.format === f;f === 'html' ? 'HTML' : FORMATS[f].label; }
      }
     }
     { svelteHTML.createElement("label", {   "class":`ma-provider`,"title":`Agent provider (locked once the artifact exists)`,});
       { svelteHTML.createElement("select", {     "bind:value":provider,"aria-label":`Design provider`,"disabled":locked || mockupAssist.busy,});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let p of __sveltets_2_ensureArray(agentProviders())){p; { svelteHTML.createElement("option", { "value":p,});p; }}
       }
     }
    if(mockupAssist.busy){ { svelteHTML.createElement("span", { "class":`ma-working`,}); { const $$_toDgnikroWeviL3C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL3C({ target: __sveltets_2_any(), props: {  "label":`Working…`,}});} }}
     { svelteHTML.createElement("button", {       "class":`ma-close`,"onclick":onclose,"aria-label":`Close design agent`,"title":`Close design agent`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
     }
   }

   { svelteHTML.createElement("div", { "class":`ma-body`,});
    if(!embedded){
      
       { svelteHTML.createElement("div", { "class":`ma-preview`,});
         { const $$_weiverPeviLpukcoM3C = __sveltets_2_ensureComponent(MockupLivePreview); new $$_weiverPeviLpukcoM3C({ target: __sveltets_2_any(), props: {    "format":mockupAssist.format,"content":mockupAssist.liveContent,}});}
       }
    }

    
     { svelteHTML.createElement("aside", { "class":`ma-side`,});
       { svelteHTML.createElement("div", { "class":`ma-shell`,});
        if(mockupAssist.sessionId){
          mockupAssist.sessionId; {
              { const $$_lanimreTyzaL4C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL4C({ target: __sveltets_2_any(), props: {    "sessionId":mockupAssist.sessionId,"readOnly":false,"forceDark":true,"preferDom":true,}});}
          }
        }else{
           { svelteHTML.createElement("div", { "class":`ma-empty`,});
             { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },"icon":`zap`,"title":embedded ? 'Ask for a change and the agent edits this artifact in place' : 'Describe the mockup and the agent builds it here',"body":`It writes ${HINT[mockupAssist.format]} and the preview updates live. Keep chatting to refine it. The agent’s shell appears here once it starts.`,}});
               { svelteHTML.createElement("div", { "class":`ma-starters`,});
                   for(let s of __sveltets_2_ensureArray(STARTERS[mockupAssist.format])){s;
                   { svelteHTML.createElement("button", {   "class":`ma-starter`,"onclick":() => useStarter(s),});s; }
                }
               }
             EmptyState}
           }
        }
       }

       { svelteHTML.createElement("div", { "class":`ma-composer`,});
         { svelteHTML.createElement("textarea", {              "dir":`auto`,"aria-label":`Message to the mockup assistant`,"bind:value":draft,"onkeydown":onKey,"placeholder":locked ? 'Ask for a change…' : `Describe the ${FORMATS[mockupAssist.format].label.toLowerCase()} to create…`,"rows":2,"disabled":mockupAssist.busy,});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("button", {           "class":`ma-send`,"onclick":send,"disabled":mockupAssist.busy || !draft.trim(),"aria-label":`Send`,"title":`Send`,});
           { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":16,}});}
         }
       }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MockupAssistPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MockupAssistPanel__SvelteComponent_ = ReturnType<typeof MockupAssistPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default MockupAssistPanel__SvelteComponent_;