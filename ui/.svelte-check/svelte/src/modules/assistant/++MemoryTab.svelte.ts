///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// The Memory tab: everything Otto knows about you, visible and editable.
//  • Profile — `profile.md`, facts you own (Otto only proposes additions).
//  • Waiting for review — memories Otto suggested (memory approval on) or
//    that a Hermes import queued; nothing is kept until you accept it.
//  • Memories — short facts Otto saved, each with where it came from;
//    search, forget one, or forget everything matching a phrase (with Undo).
//  • Import from Hermes — a one-off, read-only scan of ~/.hermes/memories
//    that only QUEUES entries for review. Hermes itself is never changed.
import { announce, focusAppContent } from '../../lib/announce.svelte';
import { sentenceCase } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
import { guardUnsaved } from '../../lib/leaveGuard';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { assistant, describeError } from '../../lib/stores/assistant.svelte';
import { assistantApi } from '../../lib/api/assistant';
import { ApiError } from '../../lib/api/client';
import type { AssistantMemory } from '../../lib/api/types';
;

  interface Props {
    onopenthread: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onopenthread }: Props = $props();

  $effect(() => {
    void assistant.loadMemory();
    void assistant.loadHermes();
  });

  const mem = $derived(assistant.memory);
  const memories = $derived(mem.data?.memories ?? []);
  const pendingMem = $derived(mem.data?.pending ?? []);

  // ── profile ────────────────────────────────────────────────────────────────
  let draft = $state<string | null>(null);
  const saved = $derived(mem.data?.profile.content ?? '');
  const profile = $derived(draft ?? saved);
  const dirty = $derived(draft !== null && draft !== saved);
  $effect(() => guardUnsaved(() => dirty, { what: 'your profile' }));
  let saving = $state(false);
  let justSaved = $state(false);
  let profileError = $state('');
  async function saveProfile(): Promise<void> {
    if (!dirty || draft === null || saving) return;
    const submitted = draft;
    saving = true;
    profileError = '';
    try {
      await assistant.saveProfile(submitted);
      // A save response acknowledges only the submitted text; keep newer edits.
      if (draft === submitted) draft = null;
      justSaved = draft === null;
    } catch (e) {
      profileError =
        e instanceof ApiError && e.status === 409
          ? 'Couldn’t save: your profile changed somewhere else. Copy your edits, reload, and apply them again.'
          : `Couldn’t save your profile. ${describeError(e)}`;
    } finally {
      saving = false;
    }
  }

  // ── memories ───────────────────────────────────────────────────────────────
  let query = $state('');
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? memories.filter((m) => m.text.toLowerCase().includes(q) || m.tags.some((t) => t.toLowerCase().includes(q))) : memories;
  });
  const ORIGIN: Record<AssistantMemory['source']['kind'], string> = { agent: 'Otto', user: 'You', hermes: 'Hermes' };
  const threadTitle = (id: string | null): string | null => (id ? (assistant.thread(id)?.title ?? 'a thread') : null);

  // No confirm: forgetting is undoable from the bar right below (patterns.md
  // §7: don't nag on a reversible action).
  async function forgetOne(m: AssistantMemory): Promise<void> {
    try {
      const token = await assistant.forgetMemory(m.id);
      undoBar = { label: `Forgot “${m.text}”`, tokens: [token] };
    } catch (e) {
      toasts.error('Couldn’t forget that memory', describeError(e));
    }
  }

  // The last forget, undoable inline (toasts have no actions).
  let undoBar = $state<{ label: string; tokens: string[] } | null>(null);
  let undoing = $state(false);
  // Spoken at the edge (the bar is inserted with its text, which is silent).
  $effect(() => {
    if (undoBar) announce(`${undoBar.label}. Undo is available.`);
  });
  async function undoForget(): Promise<void> {
    if (!undoBar) return;
    undoing = true;
    try {
      await assistant.restoreMemory(undoBar.tokens);
      undoBar = null;
    } catch (e) {
      toasts.error('Couldn’t restore', describeError(e));
    } finally {
      undoing = false;
    }
  }

  async function forgetMatching(): Promise<void> {
    const q = await confirmer.promptText('What should Otto forget? Every memory that matches is removed — you can undo it right after.', {
      title: 'Forget…',
      confirmLabel: 'Forget',
      initial: query.trim(),
    });
    if (!q) return;
    try {
      const res = await assistantApi.forget(q);
      await assistant.loadMemory();
      const n = res.forgotten.length;
      undoBar = n ? { label: `Forgot ${n} ${n === 1 ? 'memory' : 'memories'} matching “${q}”`, tokens: res.undo_tokens } : null;
      if (!n) toasts.info('Nothing matched', `No memory mentions “${q}”.`);
    } catch (e) {
      toasts.error('Couldn’t forget', describeError(e));
    }
  }

  // ── review queue ───────────────────────────────────────────────────────────
  let deciding = $state<string | null>(null);
  /** Which button of the `deciding` row is in flight (its label reads "…ing"). */
  let decidingAs = $state<'keep' | 'reject' | null>(null);
  async function accept(m: AssistantMemory): Promise<void> {
    deciding = m.id;
    decidingAs = 'keep';
    try {
      await assistant.acceptMemory(m.id);
    } catch (e) {
      toasts.error('Couldn’t keep that memory', describeError(e));
    } finally {
      deciding = null;
      decidingAs = null;
    }
  }
  async function reject(m: AssistantMemory): Promise<void> {
    deciding = m.id;
    decidingAs = 'reject';
    try {
      await assistant.forgetMemory(m.id);
    } catch (e) {
      toasts.error('Couldn’t reject that memory', describeError(e));
    } finally {
      deciding = null;
      decidingAs = null;
    }
  }

  // ── Hermes (read-only preview → queue for review) ──────────────────────────
  const hermes = $derived(assistant.hermes);
  const fresh = $derived((hermes.data?.entries ?? []).filter((e) => !e.duplicate));
  let importing = $state(false);
  async function queueHermes(): Promise<void> {
    const ok = await confirmer.ask(
      `Queue ${fresh.length} ${fresh.length === 1 ? 'entry' : 'entries'} from ~/.hermes/memories for review? Nothing is kept until you accept each one, and Hermes isn’t changed.`,
      { title: 'Import from Hermes', confirmLabel: 'Queue for review', danger: false },
    );
    if (!ok) return;
    importing = true;
    try {
      const res = await assistantApi.hermesImport();
      await Promise.all([assistant.loadMemory(), assistant.loadHermes()]);
      toasts.success(`${res.queued} queued for review`, res.duplicates ? `${res.duplicates} already known, skipped.` : undefined);
    } catch (e) {
      toasts.error('Couldn’t read the Hermes memories', describeError(e));
    } finally {
      importing = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`memory`,...__sveltets_2_empty({"data-testid":`assistant-memory`}),});
  if(mem.state === 'loading' && !mem.data){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":5,"height":40,"label":`memory`,}});}
  } else if (mem.state === 'unsupported'){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`book`,"title":`Memory isn’t available yet`,"body":`This daemon doesn’t have the assistant’s memory. Update Otto to see and edit what it remembers about you.`,}});}
  } else if (mem.state === 'error' && !mem.data){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`memory`,"error":mem.error || 'No data came back.',"empty":true,"onretry":() => void assistant.loadMemory(),}});}
  }else{
     { svelteHTML.createElement("section", { "class":`block`,});
       { svelteHTML.createElement("div", { "class":`block-head`,});
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("h2", { "class":`h`,});  }
           { svelteHTML.createElement("p", { "class":`help`,});                 }
         }
         { svelteHTML.createElement("span", {   "class":`status`,"aria-live":`polite`,});if(saving){ } else if (dirty){ } else if (justSaved){ } }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void saveProfile(),"disabled":!dirty || saving,"title":`Save (⌘S)`,});  }
       }
       { svelteHTML.createElement("label", {   "class":`sr-only`,"for":`profile-md`,});  }
       { svelteHTML.createElement("textarea", {                 "id":`profile-md`,"class":`input profile mono`,"rows":8,"value":profile,"oninput":(e) => {
          draft = e.currentTarget.value;
          justSaved = false;
        },"onkeydown":(e) => {
          if (e.key === 's' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void saveProfile();
          }
        },"placeholder":'- Prefers aisle seats and quiet hotel rooms\n- Usually free after 17:00 on weekdays',"dir":`auto`,}); }
      if(profileError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: profileError, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`err`,});profileError; }}}
     }

    if(pendingMem.length){
       { svelteHTML.createElement("section", {   "class":`block`,...__sveltets_2_empty({"data-testid":`memory-review`}),});
         { svelteHTML.createElement("div", { "class":`block-head`,});
           { svelteHTML.createElement("div", {});
             { svelteHTML.createElement("h2", { "class":`h`,});    { svelteHTML.createElement("span", { "class":`count warn`,});pendingMem.length; } }
             { svelteHTML.createElement("p", { "class":`help`,});        }
           }
         }
         { svelteHTML.createElement("ul", { "class":`list`,});
             for(let m of __sveltets_2_ensureArray(pendingMem)){m.id;
             { svelteHTML.createElement("li", { "class":`row`,});
               { svelteHTML.createElement("div", { "class":`main`,});
                 { svelteHTML.createElement("div", { "class":`text`,});m.text; }
                 { svelteHTML.createElement("div", { "class":`meta`,});
                   { svelteHTML.createElement("span", {});ORIGIN[m.source.kind] ?? sentenceCase(m.source.kind); }
                  if(m.source.file){ { svelteHTML.createElement("span", {   "class":`mono`,"dir":`ltr`,}); m.source.file; }}
                   { svelteHTML.createElement("span", {});  { svelteHTML.createElement("time", {   "datetime":m.created_at,"title":new Date(m.created_at).toLocaleString(),});rel(m.created_at); } }
                 }
               }
               { svelteHTML.createElement("div", { "class":`acts`,});
                 { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void reject(m),"disabled":deciding === m.id,"aria-label":`Reject “${m.text}”`,});deciding === m.id && decidingAs === 'reject' ? 'Rejecting…' : 'Reject'; }
                 { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => void accept(m),"disabled":deciding === m.id,"aria-label":`Keep “${m.text}”`,});deciding === m.id && decidingAs === 'keep' ? 'Keeping…' : 'Keep'; }
               }
             }
          }
         }
       }
    }

     { svelteHTML.createElement("section", { "class":`block`,});
       { svelteHTML.createElement("div", { "class":`block-head`,});
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("h2", { "class":`h`,});  { svelteHTML.createElement("span", { "class":`count`,});memories.length; } }
           { svelteHTML.createElement("p", { "class":`help`,});              }
         }
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void forgetMatching(),"disabled":!memories.length,});  }
       }
      if(undoBar){
         { svelteHTML.createElement("div", { "class":`undo-bar`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`undo`,"size":14,}});}
           { svelteHTML.createElement("span", { "class":`grow`,});undoBar.label; }
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void undoForget(),"disabled":undoing,});undoing ? 'Restoring…' : 'Undo'; }
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => { undoBar = null; void focusAppContent(); },"aria-label":`Dismiss`,"title":`Dismiss`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
         }
      }
      if(memories.length){
         { svelteHTML.createElement("div", { "class":`search`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":14,}});}
           { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`search-in`,"type":`search`,"bind:value":query,"placeholder":`Filter memories…`,"aria-label":`Filter memories`,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }
      if(!memories.length){
         { svelteHTML.createElement("p", { "class":`none`,});                 }
      } else if (!shown.length){
         { svelteHTML.createElement("p", { "class":`none`,});   query.trim();  { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => (query = ''),});  } }
      }else{
         { svelteHTML.createElement("ul", { "class":`list`,});
             for(let m of __sveltets_2_ensureArray(shown)){m.id;
             { svelteHTML.createElement("li", { "class":`row`,});
               { svelteHTML.createElement("div", { "class":`main`,});
                 { svelteHTML.createElement("div", { "class":`text`,});m.text; }
                 { svelteHTML.createElement("div", { "class":`meta`,});
                   { svelteHTML.createElement("span", {});ORIGIN[m.source.kind] ?? sentenceCase(m.source.kind); }
                   { svelteHTML.createElement("span", {});  { svelteHTML.createElement("time", {   "datetime":m.created_at,"title":new Date(m.created_at).toLocaleString(),});rel(m.created_at); } }
                  if(m.source.thread_id){
                     { svelteHTML.createElement("span", {});  }
                     { svelteHTML.createElement("button", {     "class":`link`,"onclick":() => onopenthread(m.source.thread_id!),"title":`Open the conversation it came from`,}); threadTitle(m.source.thread_id); }
                  }
                      for(let t of __sveltets_2_ensureArray(m.tags)){let i = 1;i; { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {      "variant":`outline`,"label":t,"title":t,}});}}
                 }
               }
               { svelteHTML.createElement("button", {       "class":`icon-btn forget`,"onclick":() => void forgetOne(m),"aria-label":`Forget “${m.text}”`,"title":`Forget`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
               }
             }
          }
         }
      }
     }

    if(hermes.state === 'ready' && hermes.data?.available){
       { svelteHTML.createElement("section", {   "class":`block`,...__sveltets_2_empty({"data-testid":`hermes-import`}),});
         { svelteHTML.createElement("div", { "class":`block-head`,});
           { svelteHTML.createElement("div", {});
             { svelteHTML.createElement("h2", { "class":`h`,});   }
             { svelteHTML.createElement("p", { "class":`help`,});
               hermes.data.entries.length; hermes.data.entries.length === 1 ? 'entry' : 'entries'; 
               { svelteHTML.createElement("span", {   "class":`mono`,"dir":`ltr`,});  } hermes.data.files.map((f) => f.name).join(', ');
              if(fresh.length < hermes.data.entries.length){ hermes.data.entries.length - fresh.length;  }
                    
             }
           }
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void queueHermes(),"disabled":importing || !fresh.length,});
            importing ? 'Queueing…' : fresh.length ? `Review ${fresh.length}…` : 'All imported';
           }
         }
       }
    } else if (hermes.state === 'error'){
       { svelteHTML.createElement("p", { "class":`none`,});      { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => void assistant.loadHermes(),});  } }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MemoryTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MemoryTab__SvelteComponent_ = ReturnType<typeof MemoryTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default MemoryTab__SvelteComponent_;