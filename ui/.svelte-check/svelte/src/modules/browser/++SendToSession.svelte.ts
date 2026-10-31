///<reference types="svelte" />
;// Send-to-session: a small icon button that opens the shared, viewport-clamped
// context menu (`ctxMenu` — same picker BroadcastModal/SchemaTree use for a
// data-driven list) filtered to live agent sessions, and POSTs the annotation
// to whichever one is picked via `browser.sendAnnotation`. Only sessions the
// workspace considers "live" (running/working/idle) are valid targets — a
// dead/suspended/connection session can't receive a prompt.

import { confirmer } from '../../lib/confirm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { toastError } from '../../lib/toastError';
import { browser } from '../../lib/stores/browser.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import Icon from '../../lib/components/Icon.svelte';

;type $$ComponentProps =  { annotationId: string };function $$render() {

  
  
  
  
  
  

  
  
  
  
  
  
  

  let { annotationId }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let sending = $state(false);

  function eligible() {
    return ws.agentSessions.filter((s) => {
      const st = ws.statusMap[s.id] ?? s.status;
      return st === 'running' || st === 'working' || st === 'idle';
    });
  }

  async function sendTo(sessionId: string): Promise<void> {
    if (sending) return;
    const id = annotationId;
    const workspace = ws.currentId;
    const session = eligible().find(s => s.id === sessionId);
    const mark = browser.annotations.find(a => a.id === id);
    if (!session || !mark) return;
    if (!await confirmer.ask(`Send this mark to ${session.title} (${session.provider}) and start its next turn?\n\n${mark.text.slice(0, 600)}`, { title: 'Send mark', confirmLabel: 'Send mark', danger: false })) return;
    if (annotationId !== id || ws.currentId !== workspace) return;
    sending = true;
    try {
      await browser.sendAnnotation(id, sessionId);
      toasts.success(`Sent to ${session.title}`);
    } catch (e) {
      toastError('Couldn’t send to the session', e);
    } finally {
      sending = false;
    }
  }

  function open(e: MouseEvent): void {
    const sessions = eligible();
    if (sessions.length === 0) {
      toasts.warn('No live sessions', 'Open an agent session to send marks to it.');
      return;
    }
    const items: MenuItem[] = sessions.map((s) => ({
      label: `${s.title} (${s.provider})`,
      icon: 'terminal',
      action: () => void sendTo(s.id),
    }));
    ctxMenu.show(e, items, { filter: sessions.length > 8, filterPlaceholder: 'Filter sessions…' });
  }
;
async () => {

  { svelteHTML.createElement("button", {            "class":`icon-btn`,"title":`Send to session`,"aria-label":`Send to session`,"aria-haspopup":`menu`,"disabled":sending,"onclick":open,});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":12,}});}
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SendToSession__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SendToSession__SvelteComponent_ = ReturnType<typeof SendToSession__SvelteComponent_>;
/*Ωignore_endΩ*/export default SendToSession__SvelteComponent_;