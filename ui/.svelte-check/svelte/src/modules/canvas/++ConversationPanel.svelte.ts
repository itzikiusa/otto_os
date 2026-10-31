///<reference types="svelte" />
;// The canvas "Assistant": the agent's live SHELL (the same Terminal as Agents,
// reused) embedded right here in Canvas — no navigating away — plus an input to
// send a request. You type what you want; the agent edits the scene's file and
// the board re-renders. The Terminal attaches to /ws/term/{id} directly, so it
// shows live output regardless of whether the session is in the workspace store.

import { rel } from '../../lib/stores/now.svelte';
import { onMount } from 'svelte';
import { announce } from '../../lib/announce.svelte';
import { toastError } from '../../lib/toastError';
import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import { canvas } from '../../lib/stores/canvas.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { agentProviders } from '../../lib/providers';
import { confirmer, type ChoiceOption } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { interruptSession } from '../../lib/api/interrupt';
;

  interface Props {
    editor: { generate: (p: string) => Promise<boolean>; isGenerating: () => boolean } | undefined;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { editor, onclose }: Props = $props();

  let draft = $state('');
  let busy = $state(false);

  // Which agent drives this canvas's Ask-AI (single choice; 'shell' isn't an
  // agent so it's excluded). Changing it persists on the scene.
  const providers = $derived(
    agentProviders(),
  );
  async function setProvider(p: string): Promise<void> {
    if (!canvas.currentId || p === canvas.provider) return;
    try {
      await canvas.updateMeta(canvas.currentId, { provider: p });
    } catch {
      /* surfaced elsewhere */
    }
  }

  // Make sure the workspace store knows about the canvas session (best-effort —
  // the Terminal works without it, this just enriches the header/status).
  onMount(() => {
    // By id: the main list only carries sidebar sessions, so a full refresh
    // would not bring this background (`canvas_assist`) row in anyway.
    if (canvas.sessionId && !ws.getSession(canvas.sessionId)) void ws.ensureSession(canvas.sessionId);
  });

  async function send(): Promise<void> {
    const p = draft.trim();
    if (!p || busy || !editor) return;
    const submitted = draft, owner = editor;
    const scene = canvas.currentId, context = canvas.saveContext;
    busy = true;
    try {
      const accepted = await owner.generate(p);
      if (accepted && editor === owner && canvas.currentId === scene && canvas.saveContext === context && draft === submitted) draft = '';
    } catch (e) {
      toastError('Couldn’t ask Otto', e);
    } finally {
      busy = false;
    }
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  const working = $derived(busy || editor?.isGenerating());
  // The working dot is inserted with its label, which VoiceOver doesn't
  // speak: announce the edge instead.
  let wasWorking = false;
  $effect(() => {
    const now = !!working;
    if (now && !wasWorking) announce('Otto is working on the canvas…');
    else if (!now && wasWorking) announce('Otto finished.');
    wasWorking = now;
  });

  // Stop: flag the turn server-side. It commits nothing of its own and
  // broadcasts the document currently stored, so a save made during the turn
  // stays. Then interrupt the agent (Esc into its PTY).
  let stopping = $state(false);
  async function stop(): Promise<void> {
    const sid = canvas.sessionId;
    if (!sid || stopping) return;
    stopping = true;
    try {
      await canvas.stopAssist();
      await interruptSession(sid);
      toasts.info('Stopping Ask Otto', 'Saved changes stay on the board. A drawing that was only previewed is dropped.');
    } catch (e) {
      toastError('Couldn’t stop the agent', e);
    } finally {
      stopping = false;
    }
  }

  // Version history (C5): snapshots taken before each Ask Otto commit, before a
  // restore, and at most every 10 min across manual saves. Offer the newest few.
  const ORIGIN_LABEL: Record<string, string> = {
    agent: 'Before Ask Otto',
    user: 'Manual edit',
    restore: 'Before restore',
  };
  // The shared relative time (content.md §7), not a local ladder.
  const ago = (iso: string): string => rel(iso);
  let restoring = $state(false);
  async function restorePrevious(): Promise<void> {
    const id = canvas.currentId;
    if (!id || restoring) return;
    restoring = true;
    try {
      const versions = await canvas.listVersions(id);
      if (!versions.length) {
        toasts.info('No previous versions yet', 'A version is kept before each Ask Otto turn and every 10 minutes of editing.');
        return;
      }
      const picked = await confirmer.choose(
        'Restore this canvas to an earlier version? The current board is kept as a version too, so you can undo the restore.',
        {
          title: 'Restore previous version',
          options: versions.slice(0, 6).map((v, i): ChoiceOption => ({
            label: `${ORIGIN_LABEL[v.origin] ?? v.origin} · ${ago(v.created_at)}`,
            value: v.id,
            kind: i === 0 ? 'primary' : 'normal',
          })),
        },
      );
      if (!picked.value || canvas.currentId !== id) return;
      await canvas.restoreVersion(id, picked.value);
      toasts.success('Canvas restored');
    } catch (e) {
      toastError('Couldn’t restore the canvas', e);
    } finally {
      restoring = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("aside", { "class":`assistant`,});
   { svelteHTML.createElement("header", { "class":`head`,});
     { svelteHTML.createElement("span", { "class":`title`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":14,}});}  }
     { svelteHTML.createElement("button", {       "class":`close`,"onclick":onclose,"aria-label":`Close assistant`,"title":`Close assistant`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
     }
     { svelteHTML.createElement("div", { "class":`head-controls`,});
    if(providers.length > 1){
       { svelteHTML.createElement("select", {           "class":`provider`,"value":canvas.provider,"onchange":(e) => setProvider(e.currentTarget.value),"aria-label":`Canvas agent`,"title":`Which agent draws this canvas`,});
           for(let p of __sveltets_2_ensureArray(providers)){p;
           { svelteHTML.createElement("option", { "value":p,});p; }
        }
       }
    }
    if(working){
       { svelteHTML.createElement("span", { "class":`working`,}); { const $$_toDgnikroWeviL4C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL4C({ target: __sveltets_2_any(), props: {  "label":`Working…`,}});} }
      if(canvas.sessionId){
         { svelteHTML.createElement("button", {           "class":`hist-btn stop`,"onclick":() => void stop(),"disabled":stopping,"aria-busy":stopping,"aria-label":`Stop Ask Otto`,"title":`Stop Ask Otto`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":14,}});} }
      }
    }
     { svelteHTML.createElement("button", {           "class":`hist-btn history`,"onclick":restorePrevious,"disabled":restoring || working || !canvas.currentId,"aria-label":`Restore previous version`,"title":`Restore previous version…`,});
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`undo`,"size":14,}});}
     }
     }
   }

   { svelteHTML.createElement("div", { "class":`shell`,});
    if(canvas.sessionId){
      canvas.sessionId; {
          { const $$_lanimreTyzaL2C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL2C({ target: __sveltets_2_any(), props: {    "sessionId":canvas.sessionId,"readOnly":false,"forceDark":true,"preferDom":true,}});}
      }
    }else{
       { svelteHTML.createElement("div", { "class":`empty`,});
         { svelteHTML.createElement("p", { "class":`lead`,});         }
         { svelteHTML.createElement("p", { "class":`hint`,});            
                     }
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`composer`,});
     { svelteHTML.createElement("textarea", {              "dir":`auto`,"bind:value":draft,"onkeydown":onKey,"aria-label":`Ask the canvas assistant`,"placeholder":`Ask for a diagram or a change…`,"rows":2,"disabled":busy,});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     { svelteHTML.createElement("button", {           "class":`send`,"onclick":send,"disabled":busy || !draft.trim(),"aria-label":`Send`,"title":`Send`,"aria-keyshortcuts":`Enter`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":16,}});}
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConversationPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConversationPanel__SvelteComponent_ = ReturnType<typeof ConversationPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConversationPanel__SvelteComponent_;