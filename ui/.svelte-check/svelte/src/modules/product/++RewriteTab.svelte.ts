///<reference types="svelte" />
;// Rewrite tab — generate a suggested rewrite of the story, show a two-column
// before/after diff vs the current source version, and allow publishing
// back to Jira/Confluence.

import { product } from '../../lib/stores/product.svelte';
import { toastError } from '../../lib/toastError';
import type { Poller } from '../../lib/poll';
import { liveQuery } from '../../lib/live';
import { onStoryChange } from './storyReset.svelte';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { renderMarkdown } from '../../lib/md';
import DiffView from '../../lib/components/DiffView.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { confirmOutward } from '../../lib/confirmOutward';
import type { ProductStoryVersion } from './types';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // The rewrite/tests/inject run spawns an agent CLI session via the live
  // registry, so the provider must be a real registered agent (built-in or
  // custom like grok) — openai/gemini were never valid here.
  const PROVIDERS = $derived(agentProviders());

  // ── Local UI state ──────────────────────────────────────────────────────────
  let provider = $state<string>(defaultAgentProvider());
  let generating = $state(false);
  let publishing = $state(false);

  // Loaded version bodies
  let sourceVersion = $state<ProductStoryVersion | null>(null);
  let suggestedVersion = $state<ProductStoryVersion | null>(null);
  let loadingBodies = $state(false);

  // Diff view toggle
  let diffView = $state<'split' | 'source' | 'suggested'>('split');

  // ── Polling ──────────────────────────────────────────────────────────────────
  // Shared poll helper (lib/poll.ts): paused while hidden, never stacked,
  // aborted on stop. `pollTimer !== null` still means "polling" to the UI.
  let pollTimer = $state.raw<Poller | null>(null);
  const POLL_INTERVAL_MS = 3000;
  const POLL_MAX_MS = 120_000;
  let pollStartedAt = 0;
  // Count of versions when we triggered a rewrite — to detect a new one appearing.
  let versionsCountAtStart = 0;
  // Consecutive failed reads while waiting (same as PlanTab): after
  // POLL_FAIL_SHOWN the waiting state says why (with Check again), and a
  // timeout after failed reads blames the read — not the agent, which may
  // well have written the rewrite.
  const POLL_FAIL_SHOWN = 2;
  let pollFailures = $state(0);
  let lastPollError = $state<string | null>(null);

  function clearPoll(): void {
    if (pollTimer !== null) {
      pollTimer.stop();
      pollTimer = null;
    }
  }

  /** One poll read. Resolves `false` on a failed read so the poller backs
   *  off (lib/poll.ts) instead of hammering a daemon that is down. */
  async function pollVersions(): Promise<boolean> {
    // A `product_changed` nudge also lands here with no poll running: no
    // timeout and no failure count then.
    const polling = pollTimer !== null;
    if (polling && Date.now() - pollStartedAt > POLL_MAX_MS) {
      const cause = pollFailures > 0 ? lastPollError : null;
      clearPoll();
      if (cause) toasts.warn('Couldn’t check for the rewrite', cause);
      else toasts.warn('Rewrite timed out', 'No suggested version appeared within 2 minutes.');
      return true;
    }
    try {
      await product.loadVersions();
      pollFailures = 0;
      lastPollError = null;
      const suggested = latestSuggested();
      if (suggested) {
        clearPoll();
        await loadVersionBodies(suggested);
      }
      return true;
    } catch (e) {
      if (!polling) return true;
      pollFailures++;
      lastPollError = loadErrorText(e);
      return false;
    }
  }

  /** Stop waiting for the rewrite (neutral: the agent already started keeps
   *  running, and its suggested version still lands here when it's written). */
  function stopWaiting(): void {
    clearPoll();
    toasts.info('Stopped waiting for the rewrite', 'The agent already started keeps running — its suggested version appears here when it’s ready.');
  }

  function startPolling(): void {
    clearPoll();
    versionsCountAtStart = product.versions.length;
    pollStartedAt = Date.now();
    pollFailures = 0;
    lastPollError = null;
    // Immediate first poll, then interval.
    // Immediate first poll, then every POLL_INTERVAL_MS after each settles.
    // `product_changed` (onSectionChange, below) settles it; this chain is
    // a 15 s safety net while events flow, POLL_INTERVAL_MS while they cannot.
    pollTimer = liveQuery({ run: () => pollVersions(), on: [], fallbackMs: POLL_INTERVAL_MS, safetyMs: 15_000, jitter: 0 });
  }

  // Clear on unmount or story change — depends on the story id only
  // (storyReset.svelte.ts).
  onStoryChange(() => product.selectedId, (sid) => {
    // Reset local state when story changes.
    sourceVersion = null;
    suggestedVersion = null;
    clearPoll();
    // Kick off initial load if we have a story.
    if (sid) {
      void initialLoad();
    }
  }, clearPoll);

  // Subscribe to `product_changed { section: 'rewrite' }` WS events.
  $effect(() => {
    const off = product.onSectionChange('rewrite', (_status: string) => {
      void pollVersions(); // final refresh — clears poll if version appeared
    });
    return off;
  });

  // ── Derived ─────────────────────────────────────────────────────────────────
  const story = $derived(product.detail?.story ?? null);
  const source = $derived(product.detail?.source ?? null);

  function latestSuggested(): ProductStoryVersion | null {
    const all = product.versions;
    // Find the most recent 'suggested' kind (highest version_no).
    let best: ProductStoryVersion | null = null;
    for (const v of all) {
      if (v.kind === 'suggested') {
        if (!best || v.version_no > best.version_no) best = v;
      }
    }
    return best;
  }

  /** A failed first load — inline with Retry, never as "no suggestion yet". */
  let loadError = $state<string | null>(null);

  async function initialLoad(): Promise<void> {
    const owned = product.captureSelection();
    try {
      await product.loadVersions();
      if (!owned()) return;
      loadError = null;
      const suggested = latestSuggested();
      if (suggested) {
        await loadVersionBodies(suggested);
      }
    } catch (e) {
      if (owned()) loadError = loadErrorText(e);
    }
  }

  /** Bumped per body load: only the newest load (for the still-selected
   *  story) may assign — a slow load for A never lands on B's page, and an
   *  older poll can't replace a newer suggestion. */
  let bodySeq = 0;

  async function loadVersionBodies(suggested: ProductStoryVersion): Promise<void> {
    const seq = ++bodySeq;
    const owned = product.captureSelection();
    const current = () => seq === bodySeq && owned();
    const src = source;
    loadingBodies = true;
    try {
      // Load suggested body (full).
      const fullSuggested = await product.getVersion(suggested.id);
      if (!current()) return;
      suggestedVersion = fullSuggested;

      // Load source body — use the current source from detail, or fetch by id.
      if (src) {
        const fullSource = await product.getVersion(src.id);
        if (!current()) return;
        sourceVersion = fullSource;
      }
    } catch (e) {
      if (!current()) return;
      if (suggestedVersion) toastError('Couldn’t load version bodies', e);
      else loadError = loadErrorText(e);
    } finally {
      if (seq === bodySeq) loadingBodies = false;
    }
  }

  // ── Actions ──────────────────────────────────────────────────────────────────

  async function generate(): Promise<void> {
    if (generating) return;
    generating = true;
    try {
      await product.rewrite({ provider: provider || null });
      toasts.info('Rewrite triggered', 'Waiting for suggested version to appear…');
      startPolling();
    } catch (e) {
      toastError('Couldn’t start the rewrite', e);
    } finally {
      generating = false;
    }
  }

  // The live target of a publish, named everywhere it's offered or confirmed.
  const isJira = $derived(story?.source_kind === 'jira');
  const targetShort = $derived(isJira ? (story?.source_key ?? 'the issue') : 'the Confluence page');
  const targetLabel = $derived(
    isJira ? `Jira ${story?.source_key ?? ''} “${story?.title ?? ''}”` : `Confluence page “${story?.title ?? ''}”`,
  );
  const targetNoun = $derived(isJira ? 'description' : 'page body');

  async function publish(): Promise<void> {
    if (!suggestedVersion || publishing) return;
    // Publish exactly the version the user confirmed: a `product_changed`
    // poll can swap `suggestedVersion` to n+1 while the dialog is open, and
    // the daemon publishes to the VERSION's own story — so pin both here.
    const v = suggestedVersion;
    const owned = product.captureSelection();
    if (v.story_id !== product.selectedId) {
      toasts.warn('Not published', 'This suggestion belongs to another story — reopen Rewrite and try again.');
      return;
    }
    const first = (v.body_md ?? '').split('\n').filter((l) => l.trim() !== '').slice(0, 4).join('\n');
    const ok = await confirmOutward({
      verb: `Overwrite ${targetShort}`,
      title: `Overwrite ${targetShort}?`,
      where: targetLabel,
      what: `Suggested v${v.version_no} replaces the live ${targetNoun}:\n${first}`,
      who: 'Everyone who can see it; Jira/Confluence notify its watchers.',
      danger: true,
    });
    if (!ok) return;
    if (!owned() || v.story_id !== product.selectedId) {
      toasts.warn('Not published', 'The story changed while the confirmation was open.');
      return;
    }
    publishing = true;
    try {
      await product.publishVersion(v.id);
      toasts.success('Published', 'Suggested version published back to source.');
    } catch (e) {
      toastError('Couldn’t publish the rewrite', e);
    } finally {
      publishing = false;
    }
  }

  // Rendered markdown for the two panes.
  const renderedSource = $derived(
    sourceVersion?.body_md ? renderMarkdown(sourceVersion.body_md) : ''
  );
  const renderedSuggested = $derived(
    suggestedVersion?.body_md ? renderMarkdown(suggestedVersion.body_md) : ''
  );
;
async () => {


 { svelteHTML.createElement("span", {   "class":`sr-only`,"role":`status`,});pollTimer !== null && pollFailures >= POLL_FAIL_SHOWN && lastPollError ? `Couldn’t check progress: ${lastPollError}` : ''; }

if(!story){
   { svelteHTML.createElement("div", { "class":`muted`,});   }
}else{
   { svelteHTML.createElement("div", { "class":`rewrite-tab`,});

    
     { svelteHTML.createElement("section", { "class":`rw-card gen-panel`,});
       { svelteHTML.createElement("div", { "class":`gen-row`,});
         { svelteHTML.createElement("div", { "class":`provider-wrap`,});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`rw-provider-sel`,});  }
           { svelteHTML.createElement("select", {         "id":`rw-provider-sel`,"class":`input rw-sel`,"bind:value":provider,"disabled":generating,});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let p of __sveltets_2_ensureArray(PROVIDERS)){p;
               { svelteHTML.createElement("option", { "value":p,});p; }
            }
           }
         }

         { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":generate,"disabled":generating || pollTimer !== null,});
          if(generating){ } else if (pollTimer !== null){  }else{  }
         }

        if(pollTimer !== null){
           { svelteHTML.createElement("button", {     "class":`btn`,"onclick":stopWaiting,"title":`Stop waiting for the rewrite (the agent already started keeps running)`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}  
           }
           { svelteHTML.createElement("span", { "class":`polling-indicator`,});   }
          if(pollFailures >= POLL_FAIL_SHOWN && lastPollError){
             { svelteHTML.createElement("span", { "class":`poll-error`,});   lastPollError; }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => pollTimer?.now(),});  }
          }
        }
       }
     }

    
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`the suggested rewrite`,"loading":loadingBodies,"error":loadError,"empty":!suggestedVersion,"onretry":() => void initialLoad(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("div", { "class":`muted`,});         }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
      if(suggestedVersion){
        
         { svelteHTML.createElement("section", { "class":`rw-card version-meta`,});
           { svelteHTML.createElement("div", { "class":`vm-row`,});
             { svelteHTML.createElement("div", { "class":`vm-info`,});
               { svelteHTML.createElement("span", { "class":`vm-label`,}); suggestedVersion.version_no; }
              
               { const $$_enilyBtnegA5C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA5C({ target: __sveltets_2_any(), props: {    "at":suggestedVersion.created_at,"testid":`rewrite-byline`,}});}
             }
            if(suggestedVersion.change_notes){
               { svelteHTML.createElement("div", { "class":`change-notes`,});
                 { svelteHTML.createElement("span", { "class":`cn-label`,});  }
                 { svelteHTML.createElement("span", { "class":`cn-body`,});suggestedVersion.change_notes; }
               }
            }
           }
         }

        
         { svelteHTML.createElement("div", { "class":`view-toggle-row`,});
           { svelteHTML.createElement("span", { "class":`field-label`,});  }
           { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Diff view`,});
             { svelteHTML.createElement("button", {     "aria-pressed":diffView === 'split',"onclick":() => (diffView = 'split'),});diffView === 'split';  }
             { svelteHTML.createElement("button", {     "aria-pressed":diffView === 'source',"onclick":() => (diffView = 'source'),});diffView === 'source';  }
             { svelteHTML.createElement("button", {     "aria-pressed":diffView === 'suggested',"onclick":() => (diffView = 'suggested'),});diffView === 'suggested';  }
           }
         }

        
        if(diffView === 'split'){
           { svelteHTML.createElement("div", { "class":`diff-wrap rw-card`,});
             { svelteHTML.createElement("div", { "class":`pane-header diff-pane-header`,});
               { svelteHTML.createElement("span", { "class":`pane-label source-label`,}); sourceVersion ? ` v${sourceVersion.version_no}` : ''; }
               { svelteHTML.createElement("span", { "class":`pane-label suggested-label`,}); suggestedVersion.version_no; }
             }
             { const $$_weiVffiD3C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD3C({ target: __sveltets_2_any(), props: {         "before":sourceVersion?.body_md ?? '',"after":suggestedVersion?.body_md ?? '',"mode":`split`,"contextLines":4,}});}
           }
        } else if (diffView === 'source'){
           { svelteHTML.createElement("div", { "class":`single-pane rw-card`,});
             { svelteHTML.createElement("div", { "class":`pane-header`,});
               { svelteHTML.createElement("span", { "class":`pane-label source-label`,});  }
              if(sourceVersion){
                 { svelteHTML.createElement("span", { "class":`pane-meta`,}); sourceVersion.version_no; }
              }
             }
             { svelteHTML.createElement("div", { "class":`md-body`,});
              if(renderedSource){
                 renderedSource;
              }else{
                 { svelteHTML.createElement("span", { "class":`muted`,});   }
              }
             }
           }
        }else{
           { svelteHTML.createElement("div", { "class":`single-pane rw-card`,});
             { svelteHTML.createElement("div", { "class":`pane-header`,});
               { svelteHTML.createElement("span", { "class":`pane-label suggested-label`,});  }
               { svelteHTML.createElement("span", { "class":`pane-meta`,}); suggestedVersion.version_no; }
             }
             { svelteHTML.createElement("div", { "class":`md-body`,});
              if(renderedSuggested){
                 renderedSuggested;
              }else{
                 { svelteHTML.createElement("span", { "class":`muted`,});   }
              }
             }
           }
        }

        
        if(story && story.source_kind !== 'draft'){
           { svelteHTML.createElement("section", {   "class":`rw-card rw-publish`,...__sveltets_2_empty({"data-testid":`rw-publish`}),});
             { svelteHTML.createElement("p", { "class":`rw-publish-text`,});
                  targetNoun;   { svelteHTML.createElement("strong", {});targetLabel; }  
              suggestedVersion.version_no;      
              if(sourceVersion){       sourceVersion.version_no; }
             }
             { svelteHTML.createElement("button", {       "class":`btn danger`,"onclick":publish,"disabled":publishing,...__sveltets_2_empty({"data-testid":`rw-publish-btn`}),});
              publishing ? 'Publishing…' : `Overwrite ${targetShort}…`;
             }
           }
        }
      }
     LoadState}
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RewriteTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RewriteTab__SvelteComponent_ = ReturnType<typeof RewriteTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default RewriteTab__SvelteComponent_;