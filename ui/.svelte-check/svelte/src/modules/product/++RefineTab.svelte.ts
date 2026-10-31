///<reference types="svelte" />
;
import { rel } from '../../lib/stores/now.svelte';
// RefineTab — left: list of refinement threads with "New thread" / "New from
// discovery run…" controls, in a shared ListPane; right: RefineChat for the
// selected thread. Mirrors DiscoveryTab for the list+detail layout and
// PlanTab for the <select> idiom.
//
// Doesn't use ListPane's `empty` snippet: the discovery-run picker must stay
// visible even with zero threads, so the loading/error/empty/rows branching
// all lives in `children` (unlike ChatTab/MockupsTab, whose empty state has
// nothing else to keep showing).
import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { product } from '../../lib/stores/product.svelte';
import RefineChat from './RefineChat.svelte';
import ListPane from './ui/ListPane.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { confirmer } from '../../lib/confirm.svelte';
import { runStatus } from '../../lib/status';
import type { RefinementThread, DiscoveryRunSummary } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── State ──────────────────────────────────────────────────────────────────
  let threads = $state<RefinementThread[]>([]);
  let activeTid = $state<string | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);

  let creating = $state(false);

  // Discovery runs for the "New from discovery run…" picker
  let discoveryRuns = $state<DiscoveryRunSummary[]>([]);
  let selectedRunId = $state('');

  // ── Load on mount / story change ──────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes.
    product.selectedId;
    // A thread belongs to ONE story: drop the previous story's list/selection so
    // the auto-pick below runs for the new story instead of keeping a stale id.
    untrack(() => {
      threads = [];
      activeTid = null;
      creating = false;
      selectedRunId = '';
      discoveryRuns = [];
    });
    void loadThreads();
    void loadDiscoveryRuns();
  });

  async function loadThreads(): Promise<void> {
    const current = product.captureSelection();
    loading = true;
    loadError = null;
    try {
      const loaded = await product.listRefinementThreads();
      if (!current()) return;
      threads = loaded;
      // Auto-select the first active thread if nothing is selected yet.
      if (!activeTid && threads.length > 0) {
        const first = threads.find((t) => t.status === 'active') ?? threads[0];
        activeTid = first.id;
      }
    } catch (e) {
      if (!current()) return;
      loadError = loadErrorText(e);
    } finally {
      if (current()) loading = false;
    }
  }

  async function loadDiscoveryRuns(): Promise<void> {
    const current = product.captureSelection();
    try {
      const loaded = await product.listDiscoveryRuns();
      if (!current()) return;
      discoveryRuns = loaded;
    } catch {
      if (!current()) return;
      // Discovery runs are optional context — ignore errors here.
      discoveryRuns = [];
    }
  }

  // ── Create new thread ──────────────────────────────────────────────────────

  async function createThread(): Promise<void> {
    const current = product.captureSelection();
    if (creating) return;
    creating = true;
    try {
      const newThread = await product.createRefinementThread({});
      if (!current()) return;
      threads = [newThread, ...threads];
      activeTid = newThread.id;
    } catch (e) {
      if (!current()) return;
      toastError('Couldn’t create thread', e);
    } finally {
      if (current()) creating = false;
    }
  }

  async function createFromRun(): Promise<void> {
    const current = product.captureSelection();
    if (!selectedRunId || creating) return;
    creating = true;
    try {
      const newThread = await product.createRefinementThread({ discovery_run_id: selectedRunId });
      if (!current()) return;
      threads = [newThread, ...threads];
      activeTid = newThread.id;
      selectedRunId = '';
    } catch (e) {
      if (!current()) return;
      toastError('Couldn’t create thread', e);
    } finally {
      if (current()) creating = false;
    }
  }

  // ── Archive ────────────────────────────────────────────────────────────────

  async function archiveThread(tid: string): Promise<void> {
    const current = product.captureSelection();
    const target = threads.find((t) => t.id === tid);
    const label = target?.title || 'Untitled';
    const ok = await confirmer.ask(
      `Archive refinement thread “${label}”? It will be marked archived in this story’s thread list.`,
      { title: 'Archive thread', confirmLabel: 'Archive', danger: false },
    );
    if (!ok || !current()) return;
    try {
      const updated = await product.archiveRefinementThread(tid);
      if (!current()) return;
      threads = threads.map((t) => (t.id === tid ? updated : t));
      if (activeTid === tid) {
        // Switch away from an archived thread.
        const next = threads.find((t) => t.id !== tid && t.status === 'active');
        activeTid = next?.id ?? null;
      }
    } catch (e) {
      if (!current()) return;
      toastError('Couldn’t archive thread', e);
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

;
async () => {

 { svelteHTML.createElement("div", { "class":`refine-tab`,});
  
   { const $$_enaPtsiL1C = __sveltets_2_ensureComponent(ListPane); const $$_enaPtsiL1 = new $$_enaPtsiL1C({ target: __sveltets_2_any(), props: { "title":`Threads`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":createThread,"disabled":creating,"title":`Start a new refinement conversation`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} creating ? 'Creating…' : 'New thread';
       }
    };return __sveltets_2_any(0)},children:() => { async ()/*Ωignore_positionΩ*/ => {
      
      if(discoveryRuns.length > 0){
         { svelteHTML.createElement("div", { "class":`run-picker-row`,});
           { svelteHTML.createElement("select", {       "class":`picker`,"bind:value":selectedRunId,"title":`Seed the thread with a discovery run’s findings`,});/*Ωignore_startΩ*/() => selectedRunId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});    }
               for(let dr of __sveltets_2_ensureArray(discoveryRuns)){dr.run.id;
               { svelteHTML.createElement("option", { "value":dr.run.id,});
                rel(dr.run.created_at);  runStatus(dr.derived_status).label;
               }
            }
           }
           { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":createFromRun,"disabled":creating || !selectedRunId,"title":`Create a thread seeded from the selected discovery run`,});
            
           }
         }
      }

      if((loading || loadError) && threads.length === 0){
         { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {          "what":`threads`,"variant":`compact`,"loading":loading,"error":loadError,"empty":true,"onretry":() => void loadThreads(),}});}
      } else if (threads.length === 0){
         { svelteHTML.createElement("p", { "class":`empty-rail-hint`,});
          discoveryRuns.length > 0 ? 'No threads yet — use New thread or seed one from a discovery run above.' : 'No threads yet — use New thread above to start one.';
         }
      }else{
        if(loadError){
           { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {          "what":`threads`,"variant":`compact`,"loading":loading,"error":loadError,"onretry":() => void loadThreads(),}});}
        }
         { svelteHTML.createElement("div", { "class":`thread-list`,});
             for(let t of __sveltets_2_ensureArray(threads)){t.id;
             { svelteHTML.createElement("div", {     "class":`thread-item`,});activeTid === t.id;t.status === 'archived';
               { svelteHTML.createElement("button", {       "class":`thread-btn`,"aria-current":activeTid === t.id ? 'true' : undefined,"onclick":() => (activeTid = t.id),});
                 { svelteHTML.createElement("span", { "class":`thread-title`,});t.title; }
                 { svelteHTML.createElement("span", { "class":`thread-meta`,});
                   { svelteHTML.createElement("span", {  "class":`thread-status`,});t.status === 'archived';
                    t.status === 'archived' ? 'Archived' : 'Active';
                   }
                   { svelteHTML.createElement("span", {   "class":`thread-date`,"title":new Date(t.updated_at).toLocaleString(),});rel(t.updated_at); }
                 }
               }
              if(t.status === 'active'){
                 { svelteHTML.createElement("button", {       "class":`archive-btn reveal-on-hover`,"onclick":() => archiveThread(t.id),"title":`Archive this thread`,});
                  
                 }
              }
             }
          }
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions, children} = $$_enaPtsiL1.$$prop_def;/*Ωignore_endΩ*/
    
    
   ListPane}

  
   { svelteHTML.createElement("div", { "class":`chat-pane`,});
    if(activeTid){
       { const $$_tahCenifeR2C = __sveltets_2_ensureComponent(RefineChat); new $$_tahCenifeR2C({ target: __sveltets_2_any(), props: {  "tid":activeTid,}});}
    }else{
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {       "icon":`comment`,"title":threads.length === 0 ? 'No refinement threads yet' : 'No thread open',"body":threads.length === 0
          ? 'Start a New thread to refine this story with an agent — optionally seeded from a discovery run.'
          : 'Open a thread on the left, or start a New thread.',}});}
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RefineTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RefineTab__SvelteComponent_ = ReturnType<typeof RefineTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default RefineTab__SvelteComponent_;