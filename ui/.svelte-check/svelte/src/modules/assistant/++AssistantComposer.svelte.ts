///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// The thread composer: multi-line (Enter sends, Shift+Enter breaks the
// line), a leading `@claude` / `@codex` routes one turn (the daemon strips
// it), the model chip pins the thread, attachments land in the assistant's
// inbox. There is no dictate control until voice can actually record.
import Icon from '../../lib/components/Icon.svelte';
import { guardUnsaved } from '../../lib/leaveGuard';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { assistantApi } from '../../lib/api/assistant';
import { assistant, describeError } from '../../lib/stores/assistant.svelte';
import { formatBytes } from '../../lib/metric-format';
import { parseRouteHint, providerLabel, providerName } from '../../lib/assistant/model';
import PinModelSheet from './PinModelSheet.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import type { AssistantAttachment, AssistantThread } from '../../lib/api/types';
;

  interface Props {
    thread: AssistantThread;
    /** The backing session is still answering — the daemon refuses a new turn (409). */
    busy?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { thread, busy = false }: Props = $props();

  let text = $state('');
  let sending = $state(false);
  let error = $state('');
  let pinning = $state(false);
  let files = $state<AssistantAttachment[]>([]);
  let uploading = $state(0);
  let ta = $state<HTMLTextAreaElement | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);

  $effect(() => guardUnsaved(() => !!text.trim() || files.length > 0 || sending || uploading > 0, { what: 'this message' }));

  const hint = $derived(parseRouteHint(text));
  const hasText = $derived(hint.text.trim() !== '' || files.length > 0);
  const canSend = $derived(!busy && !sending && uploading === 0 && hasText);
  const sendTitle = $derived(busy ? 'Otto is still answering — send when it’s done' : 'Send (Enter)');

  // Grow with the text up to ~8 lines, then scroll inside.
  $effect(() => {
    void text;
    const el = ta;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  });

  async function send(): Promise<void> {
    if (!canSend) return;
    // The raw text goes as typed: the daemon routes on the leading mention and strips it.
    const body = text.trim();
    const attachments = files;
    sending = true;
    error = '';
    text = '';
    files = [];
    try {
      await assistant.send(thread.id, body, attachments);
    } catch (e) {
      // Nothing is lost: the draft comes back with the reason.
      text = text.trim() ? `${body}\n\n${text}` : body;
      files = [...attachments, ...files.filter((file) => !attachments.some((sent) => sent.id === file.id))];
      error = `Couldn’t send. ${describeError(e)}`;
    } finally {
      sending = false;
      ta?.focus();
    }
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    }
  }

  async function onFiles(list: FileList | null): Promise<void> {
    if (!list) return;
    error = '';
    const failures: string[] = [];
    for (const f of Array.from(list)) {
      uploading += 1;
      try {
        const content_base64 = await toBase64(f);
        const att = await assistantApi.attach(thread.id, { name: f.name, content_base64, mime: f.type || undefined });
        files = [...files, att];
      } catch (e) {
        failures.push(`Couldn’t attach ${f.name}. ${describeError(e)}`);
        error = failures.join("\n");
      } finally {
        uploading -= 1;
      }
    }
    if (fileInput) fileInput.value = '';
  }

  function toBase64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const r = new FileReader();
      r.onerror = () => reject(r.error ?? new Error('read failed'));
      r.onload = () => {
        const s = String(r.result ?? '');
        resolve(s.slice(s.indexOf(',') + 1));
      };
      r.readAsDataURL(blob);
    });
  }

  const chipLabel = $derived(thread.route_pinned ? providerLabel(thread.provider, thread.model) : 'Auto');
;
async () => {

 { svelteHTML.createElement("div", { "class":`composer`,});
  if(hint.provider){
     { svelteHTML.createElement("p", {   "class":`route`,...__sveltets_2_empty({"data-testid":`route-hint`}),});
       { const $$_nocIredivorP2C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP2C({ target: __sveltets_2_any(), props: {    "provider":hint.provider,"size":12,}});}      { svelteHTML.createElement("strong", {});providerName(hint.provider); }        
     }
  }
  if(files.length || uploading){
     { svelteHTML.createElement("ul", {   "class":`files`,"aria-label":`Attachments`,});
          for(let f of __sveltets_2_ensureArray(files)){let i = 1;f.id;
         { svelteHTML.createElement("li", { "class":`file`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} { svelteHTML.createElement("span", {   "class":`fname`,"title":f.path,});f.name; } { svelteHTML.createElement("span", { "class":`dim`,});formatBytes(f.size); }
           { svelteHTML.createElement("button", {       "class":`icon-btn x`,"onclick":() => (files = files.filter((_, j) => j !== i)),"aria-label":`Remove ${f.name}`,"title":`Remove ${f.name}`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
         }
      }
      if(uploading){ { svelteHTML.createElement("li", { "class":`file dim`,}); uploading; uploading === 1 ? 'file' : 'files';  }}
     }
  }
   { svelteHTML.createElement("div", { "class":`box`,});
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => fileInput?.click(),"aria-label":`Attach files`,"title":`Attach files`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});}
     }
     { const $$_input2 = svelteHTML.createElement("input", {      "type":`file`,"multiple":true,"hidden":true,"onchange":(e) => void onFiles(e.currentTarget.files),});fileInput = $$_input2;}
     { const $$_textarea2 = svelteHTML.createElement("textarea", {               "dir":`auto`,"bind:value":text,"rows":1,"class":`ta`,"placeholder":viewport.isPhone ? 'Reply to Otto…' : 'Reply to Otto…  (@codex to route one turn)',"aria-label":`Message Otto`,"onkeydown":onKey,});ta = $$_textarea2;/*Ωignore_startΩ*/() => text = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     { svelteHTML.createElement("button", {         "class":`chip-btn`,"onclick":() => (pinning = true),"title":thread.route_pinned ? 'Pinned for this thread — change or unpin' : 'Routing rules pick the model — pin one for this thread',"aria-label":`Model: ${chipLabel}. Change`,...__sveltets_2_empty({"data-testid":`model-chip`}),});
      if(thread.route_pinned){ { const $$_nocIredivorP3C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP3C({ target: __sveltets_2_any(), props: {    "provider":thread.provider,"size":12,}});}}else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`split`,"size":12,}});}}
       { svelteHTML.createElement("span", {});chipLabel; }
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
     }
     { svelteHTML.createElement("button", {         "class":`icon-btn send`,"onclick":() => void send(),"disabled":!canSend,"aria-label":`Send`,"title":sendTitle,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":14,}});}
     }
   }
  if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`err`,});error; }}}
 }
if(pinning){
   { const $$_teehSledoMniP0C = __sveltets_2_ensureComponent(PinModelSheet); new $$_teehSledoMniP0C({ target: __sveltets_2_any(), props: {   thread,"onclose":() => (pinning = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AssistantComposer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AssistantComposer__SvelteComponent_ = ReturnType<typeof AssistantComposer__SvelteComponent_>;
/*Ωignore_endΩ*/export default AssistantComposer__SvelteComponent_;