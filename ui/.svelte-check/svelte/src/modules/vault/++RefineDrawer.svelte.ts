///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
import { announceShown } from '../../lib/announce.svelte';
// "Refine with Otto" — a bottom drawer under the note editor/reading view.
// One refine session per note (server-side): the first Send spawns it (the
// POST is LONG — it resolves when the agent's turn completes), and ~800ms
// after POSTing we poll GET refine-session until the session id lands so the
// live terminal attaches while the agent is still typing. Once a session
// exists the provider is locked (follow-up prompts reuse the same session).
// The drawer is remounted per workspace, vault and note in NoteView, so
// on mount we reattach to any session an earlier open of this note started.
import { onMount } from 'svelte';
import type { Poller } from '../../lib/poll';
import { liveQuery } from '../../lib/live';
import Icon from '../../lib/components/Icon.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { refineNote, refineSession, resetRefineSession, vaultNote, writeVaultNote } from '../../lib/api/vault';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import { toasts } from '../../lib/toast.svelte';
import { vault } from './vault.svelte';
;

  /** What the last refine turn did to the note — the agent's edit lands on disk
   *  directly, so we keep the pre-turn text to show a summary and offer Undo
   *  (patterns §1: agent output is attributed and reversible). */
  interface RefineResult {
    before: string;
    hash: string;
    added: number;
    removed: number;
    summary: string;
    provider: string;
    at: number;
  };
;type $$ComponentProps =  { path: string };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { path }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let provider = $state(defaultAgentProvider());
  let prompt = $state('');
  let sending = $state(false);
  let sessionId = $state<string | null>(null);
  /** Bumped on reset — a long-running send from a previous epoch must not
   *  re-apply its (stale) session/state when it finally resolves. */
  let epoch = 0;

  const providers = $derived(agentProviders());

  let result = $state<RefineResult | null>(null);
  let undoing = $state(false);
  /** A queued "Review + fix" prompt waits for an explicit Send. */
  let queued = $state(false);

  /** Rough line delta (multiset diff) — enough for a "+N / −M lines" summary. */
  function lineDelta(before: string, after: string): { added: number; removed: number } {
    const count = new Map<string, number>();
    for (const l of before.split('\n')) count.set(l, (count.get(l) ?? 0) + 1);
    let added = 0;
    for (const l of after.split('\n')) {
      const n = count.get(l) ?? 0;
      if (n > 0) count.set(l, n - 1);
      else added += 1;
    }
    let removed = 0;
    for (const n of count.values()) removed += n;
    return { added, removed };
  }

  // -- session polling (starts ~800ms after the POST goes out) -----------------
  // Event-fed: the refine agent's session announces itself (`session_created`);
  // a disciplined 1 s poll (in-flight guard, hidden pause) only while the
  // event socket is down, a 10 s safety net otherwise.
  let pollDelay: ReturnType<typeof setTimeout> | null = null;
  let pollTimer: Poller | null = null;

  function stopPolling(): void {
    if (pollDelay) clearTimeout(pollDelay);
    pollTimer?.stop();
    pollDelay = null;
    pollTimer = null;
  }

  function startSessionPoll(): void {
    stopPolling();
    pollDelay = setTimeout(() => {
      pollTimer = liveQuery({ run: () => checkSession(), on: ['session_created'], fallbackMs: 1000, safetyMs: 10_000 });
    }, 800);
  }

  async function checkSession(): Promise<void> {
    if (!vault.current) return;
    const myEpoch = epoch, workspace = vault.wsId, id = vault.current.id;
    try {
      const s = await refineSession(workspace, id, path);
      if (myEpoch !== epoch || vault.wsId !== workspace || vault.current?.id !== id) return; // reset raced this poll
      if (s.session_id) {
        sessionId = s.session_id;
        stopPolling();
      }
    } catch {
      /* transient — next tick retries */
    }
  }

  async function send(): Promise<void> {
    const p = prompt.trim();
    if (!p || sending || !vault.current) return;
    sending = true;
    queued = false;
    result = null;
    const myEpoch = epoch;
    const wsId = vault.wsId;
    const vaultId = vault.current.id;
    const current = () => epoch === myEpoch && vault.wsId === wsId && vault.current?.id === vaultId;
    // Snapshot the note as it is on disk BEFORE the agent touches it (Undo).
    let before: string | null = null;
    try {
      before = (await vaultNote(wsId, vaultId, path)).raw;
    } catch {
      /* no snapshot → the turn still runs, just without Undo */
    }
    if (!current()) return;
    startSessionPoll();
    try {
      const r = await refineNote(wsId, vaultId, { path, prompt: p, provider });
      if (!current()) return; // reset or selection changed mid-turn — stale result
      sessionId = r.session_id;
      stopPolling();
      prompt = '';
      toasts.success('Refined', (r.reply.split('\n')[0] || 'done').slice(0, 200));
      if (before !== null) {
        try {
          const after = await vaultNote(wsId, vaultId, path);
          if (!current()) return;
          if (after.raw !== before) {
            result = {
              before,
              hash: after.meta.hash,
              ...lineDelta(before, after.raw),
              summary: (r.reply.split('\n')[0] || '').slice(0, 200),
              provider,
              at: Date.now(),
            };
          }
        } catch {
          /* summary is best-effort */
        }
      }
      // Reload the agent's changes — but never clobber in-flight local edits.
      if (!vault.dirty && !vault.editing && vault.notePath === path) {
        void vault.open(path);
      }
    } catch (e) {
      if (!current()) return;
      stopPolling();
      toastError('Couldn’t refine', e);
    } finally {
      if (current()) sending = false;
    }
  }

  /** Put the pre-turn text back (the agent's edit is the thing being undone). */
  async function undoRefine(): Promise<void> {
    const r = result;
    if (!r || undoing || !vault.current) return;
    if (vault.dirty) {
      toasts.error('Couldn’t undo the refine', 'Save or discard your own edits to this note first.');
      return;
    }
    undoing = true;
    try {
      await writeVaultNote(vault.wsId, vault.current.id, { path, content: r.before, if_hash: r.hash });
      result = null;
      toasts.success('Refine undone', 'The note is back to what it was before the agent’s edit.');
      if (!vault.editing && vault.notePath === path) void vault.open(path);
    } catch (e) {
      toastError('Couldn’t undo the refine', e);
    } finally {
      undoing = false;
    }
  }

  /** Detach the note's session: unblocks a stuck/exited agent — the next Send
   *  starts a FRESH session with whatever provider is selected. */
  async function reset(): Promise<void> {
    if (!vault.current) return;
    // Mid-edit the agent may have half-written the note — say so before
    // detaching its session (an idle session can be dropped without asking).
    if (sending) {
      const ok = await confirmer.ask(
        'The agent is still editing this note. Stopping detaches its session and the edit may be left half-done; Send starts a fresh agent. Your note stays as it is on disk.',
        { title: 'Stop and start over?', confirmLabel: 'Stop and start over', danger: true },
      );
      if (!ok) return;
    }
    epoch += 1;
    stopPolling();
    try {
      await resetRefineSession(vault.wsId, vault.current.id, path);
    } catch (e) {
      toastError('Couldn’t reset', e);
      return;
    }
    sessionId = null;
    sending = false;
    result = null;
  }

  onMount(() => {
    // Reattach an existing refine session for this note (survives drawer close).
    void checkSession();
    // "Review + fix" from the tree: consume the queued prompt but wait for an
    // explicit Send — the agent edits the note on disk, so it never starts on mount.
    const pending = vault.pendingRefine;
    if (pending && pending.path === path) {
      vault.pendingRefine = null;
      prompt = pending.prompt;
      queued = true;
    }
    return () => { epoch++; stopPolling(); };
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`refine-drawer`,});
   { svelteHTML.createElement("div", { "class":`bar`,});
     { svelteHTML.createElement("span", { "class":`spark`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":13,}});} }
     { svelteHTML.createElement("select", {       "bind:value":provider,"disabled":sending,"title":sessionId
        ? 'Picking a different provider starts a fresh agent session on the next Send'
        : 'Agent provider',});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
         for(let p of __sveltets_2_ensureArray(providers)){p;
         { svelteHTML.createElement("option", { "value":p,});p; }
      }
     }
     { svelteHTML.createElement("input", {            "dir":`auto`,"bind:value":prompt,"aria-label":`Refinement request`,"placeholder":`Refine this note… (e.g. tighten the intro, add a troubleshooting section)`,"disabled":sending,"onkeydown":(e) => {
        if (e.key === 'Enter') void send();
      },});/*Ωignore_startΩ*/() => prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("button", {     "class":`send`,"disabled":sending || !prompt.trim(),"onclick":() => void send(),});
      if(sending){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); } }else{ }
     }
    if(sessionId || sending){
       { svelteHTML.createElement("button", {       "class":`btn small`,"title":`Detach this note’s agent session — unblocks a stuck or exited agent; the next Send starts a fresh one`,"onclick":() => void reset(),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}    
       }
    }
   }

  if(queued && !sending){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),('Review and fix is ready — press Send to start.')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`notice`,});                      }}
  }
  if(result){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(`Refined this note: ${result.added} lines added, ${result.removed} removed.`)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`notice result`,...__sveltets_2_empty({"data-testid":`refine-result`}),});
       { const $$_enilyBtnegA2C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA2C({ target: __sveltets_2_any(), props: {      "provider":result.provider,"at":result.at,"label":`Refined this note`,}});}
       { svelteHTML.createElement("span", { "class":`delta`,}); result.added;  result.removed; result.summary ? ` · ${result.summary}` : ''; }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":undoing,"onclick":() => void undoRefine(),});
        if(undoing){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); } }else{ }
       }
     }}
  }

  if(sessionId){
     { svelteHTML.createElement("div", { "class":`term`,});
      sessionId; {
          { const $$_lanimreTyzaL2C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL2C({ target: __sveltets_2_any(), props: {  "sessionId":sessionId,"preferDom":true,}});}
      }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`placeholder`,});
      sending
        ? 'Starting the agent — its terminal will attach here…'
        : 'The agent works in a live session on this note; its terminal appears here after the first prompt.';
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RefineDrawer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RefineDrawer__SvelteComponent_ = ReturnType<typeof RefineDrawer__SvelteComponent_>;
/*Ωignore_endΩ*/export default RefineDrawer__SvelteComponent_;