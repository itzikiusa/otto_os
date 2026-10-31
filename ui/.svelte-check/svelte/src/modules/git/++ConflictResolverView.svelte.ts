///<reference types="svelte" />
;// Merge-conflict resolver. LEFT: the conflicted files with a resolved /
// unresolved indicator; RIGHT: the ConflictFilePane for the selected file.
// Footer: Abort merge (confirm) and Complete merge (enabled once every file
// is resolved). Leaving the view (abort/complete) is reported via `onleave`.

import type { GitOpInProgress, MergeConflictStatus } from '../../lib/api/types';
import { toastError } from '../../lib/toastError';
import { announceShown } from '../../lib/announce.svelte';
import { git } from '../../lib/stores/git.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import ConflictFilePane from './ConflictFilePane.svelte';
import { canCompleteOperation } from './operationState';
;

  interface Props {
    repoId: string;
    /** Optional seed list so the view shows files immediately after a merge. */
    initialFiles?: string[];
    initialSource?: string | null;
    /** Which operation the conflicts belong to (merge / rebase / cherry-pick /
     *  revert); null for conflicts with no state file (a stash pop). Refined
     *  from the daemon's merge status once loaded. */
    initialOp?: GitOpInProgress | null;
    /** Called after the merge is completed or aborted (or there's nothing to do). */
    onleave: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, initialFiles = [], initialSource = null, initialOp = null, onleave }: Props = $props();

  let status = $state<MergeConflictStatus | null>(null);
  let loading = $state(true);
  // Deliberately seeded from the prop's INITIAL value — refined from the
  // daemon's merge status as soon as it loads.
  // svelte-ignore state_referenced_locally
  let op = $state<GitOpInProgress | null>(initialOp);

  /** Human name of the operation, for titles/buttons ("merge", "rebase", …). */
  const opName = $derived(
    op === 'rebase' ? 'rebase'
    : op === 'cherry_pick' ? 'cherry-pick'
    : op === 'revert' ? 'revert'
    : op === 'merge' ? 'merge'
    : null,
  );

  // Files still conflicted (pulled from merge status). When a file is resolved
  // we move it from `pending` to `resolved` locally so the user sees progress
  // without a full reload.
  let pending = $state<string[]>([]);
  let resolved = $state<Set<string>>(new Set());
  let selected = $state<string | null>(null);

  let busy = $state<'' | 'abort' | 'complete'>('');

  // ── Mobile (phone + tablet) ───────────────────────────────────────────────
  // The desktop two-pane (files rail | detail) doesn't fit a narrow viewport,
  // so ≤1024 stacks the conflicted-files list as a collapsible strip above the
  // detail pane. Picking a file collapses the list so the resolver gets the
  // screen.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  let secFilesOpen = $state(true);

  function selectFile(f: string): void {
    selected = f;
    if (isMobile) secFilesOpen = false;
  }

  // A failed merge-status read with no seed to fall back on: shown with Retry
  // instead of a neutral "No conflicted files." while the repo is mid-merge.
  let loadError = $state<string | null>(null);
  let reloadKey = $state(0);

  $effect(() => {
    const id = repoId;
    void reloadKey;
    loading = true;
    loadError = null;
    void git
      .getMergeStatus(id)
      .then((s) => {
        status = s;
        op = s.op ?? null;
        // Prefer the daemon's authoritative list; fall back to the seed.
        const files = s.conflicted_files;
        pending = [...files];
        resolved = new Set();
        selected = files[0] ?? null;
        // No merge in progress and nothing to resolve → bounce out.
        if (!s.merging && files.length === 0) {
          onleave();
        }
      })
      .catch((e) => {
        loadError = loadErrorText(e);
        // Fall back to the seed list from the merge that brought us here.
        status = { merging: initialFiles.length > 0, source: initialSource, conflicted_files: initialFiles };
        pending = [...initialFiles];
        resolved = new Set();
        selected = initialFiles[0] ?? null;
      })
      .finally(() => {
        loading = false;
      });
  });

  const sourceLabel = $derived(status?.source ?? initialSource);
  // A/B side labels for the hunk pickers: A = the checked-out branch (ours),
  // B = the branch being merged in (theirs). Best-effort — plain OURS/THEIRS
  // when the repo status / merge source isn't known.
  const oursLabel = $derived(git.statusById[repoId]?.branch ?? 'OURS');
  const theirsLabel = $derived(sourceLabel ?? 'THEIRS');
  const allFiles = $derived([...pending]);
  const allResolved = $derived(canCompleteOperation(op, allFiles, resolved));

  function isResolved(path: string): boolean {
    return resolved.has(path);
  }

  function handleResolved(path: string): void {
    resolved = new Set(resolved).add(path);
    // Auto-advance to the next unresolved file for a smoother flow.
    const next = pending.find((f) => !resolved.has(f) && f !== path);
    // The last file: clear the pane — leaving it mounted invited a second
    // "Mark resolved" that 409'd (the file is no longer conflicted).
    selected = next ?? null;
    // Quietly reconcile against the daemon: a second pull / an agent / the CLI
    // may have added or resolved conflicts since this view seeded its list.
    void reconcile();
  }

  /** Re-read merge status and fold changes into the local lists: newly
   *  conflicted files are appended as pending; files no longer conflicted (and
   *  not resolved through this view) are marked resolved. Never removes rows —
   *  progress display stays stable. */
  async function reconcile(): Promise<void> {
    try {
      const s = await git.getMergeStatus(repoId);
      op = s.op ?? null;
      const live = new Set(s.conflicted_files);
      const known = new Set(pending);
      const added = s.conflicted_files.filter((f) => !known.has(f));
      if (added.length > 0) pending = [...pending, ...added];
      const externallyResolved = pending.filter((f) => !live.has(f) && !resolved.has(f));
      if (externallyResolved.length > 0) {
        const next = new Set(resolved);
        for (const f of externallyResolved) next.add(f);
        resolved = next;
      }
    } catch {
      /* best-effort — keep local state */
    }
  }

  async function abort(): Promise<void> {
    const label = opName ?? 'merge';
    const ok = await confirmer.ask(
      `Abort the ${label} and discard any resolutions made so far?`,
      { title: `Abort ${label}`, confirmLabel: `Abort ${label}`, danger: true }
    );
    if (!ok) return;
    busy = 'abort';
    try {
      const s = await git.abortMerge(repoId);
      if (git.primary?.id === repoId) git.primaryStatus = s;
      toasts.info(`${label[0].toUpperCase()}${label.slice(1)} aborted`);
      onleave();
    } catch (e) {
      toastError(`Couldn’t abort the ${label}`, e);
    } finally {
      busy = '';
    }
  }

  async function complete(): Promise<void> {
    if (!allResolved || busy) return;
    // No operation to conclude (stash-pop conflicts): the resolutions are
    // already written + staged — there's nothing to commit here, just leave.
    if (op === null) {
      onleave();
      return;
    }
    busy = 'complete';
    try {
      const result = await git.completeMerge(repoId);
      if (git.primary?.id === repoId) git.primaryStatus = result.repo_status;
      if (result.status === 'conflicts') {
        // Still conflicting (shouldn't normally happen) — refresh in place.
        pending = [...result.conflicted_files];
        resolved = new Set();
        selected = result.conflicted_files[0] ?? null;
        toasts.warn('Still conflicting', 'Some files still have conflicts.');
      } else if (result.repo_status.op_in_progress) {
        pending = [];
        resolved = new Set();
        selected = null;
        toasts.info('Operation paused', result.note ?? 'Amend the current commit, then continue.');
      } else {
        const done = opName ?? 'merge';
        toasts.success(`${done[0].toUpperCase()}${done.slice(1)} completed`, result.commit ? result.commit.slice(0, 8) : undefined);
        onleave();
      }
    } catch (e) {
      await reconcile();
      toastError(`Couldn’t complete the ${opName ?? 'merge'}`, e);
    } finally {
      busy = '';
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {  "class":`resolver`,});isMobile;
   { svelteHTML.createElement("header", { "class":`resolver-head`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":14,}});}
     { svelteHTML.createElement("span", { "class":`head-title`,}); opName ?? 'file';  }
    if(sourceLabel){
       { svelteHTML.createElement("span", { "class":`head-source`,});opName === 'merge' ? 'merging' : 'from';  { svelteHTML.createElement("span", { "class":`mono`,}); { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":sourceLabel,}});} } }
    } else if (op === null){
       { svelteHTML.createElement("span", { "class":`head-source`,});             }
    }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(allFiles.length > 0){
       { svelteHTML.createElement("span", {  "class":`head-count`,});allResolved;
        resolved.size; allFiles.length;  
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`resolver-body`,});
    if(loading){
       { svelteHTML.createElement("div", { "style":`padding: 16px; flex: 1`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"height":28,}});} }
    } else if (loadError && allFiles.length === 0){
      
       { svelteHTML.createElement("div", { "class":`load-fail`,});
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":`the merge status`,"error":loadError,"empty":true,"variant":`page`,"onretry":() => reloadKey++,}});}
       }
    }else{
      if(isMobile){
        
         { svelteHTML.createElement("button", {       "class":`mob-sec-head`,"onclick":() => (secFilesOpen = !secFilesOpen),"aria-expanded":secFilesOpen,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":secFilesOpen ? 'chevronDown' : 'chevronRight',"size":13,}});}
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":13,}});}
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("span", { "class":`mob-sec-count`,});resolved.size; allFiles.length; }
         }
      }
      
       { svelteHTML.createElement("aside", {  "class":`files-panel`,});isMobile && !secFilesOpen;
        if(!isMobile){
           { svelteHTML.createElement("div", { "class":`files-head`,});  }
        }
        if(allFiles.length === 0){
           { svelteHTML.createElement("div", { "class":`dim files-empty`,});   }
        }else{
             for(let f of __sveltets_2_ensureArray(allFiles)){f;
             { svelteHTML.createElement("button", {         "class":`file-row`,"onclick":() => selectFile(f),"title":f,});selected === f;isResolved(f);
               { svelteHTML.createElement("span", { "class":`file-status`,});
                if(isResolved(f)){
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
                }else{
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`dot`,"size":12,}});}
                }
               }
               { svelteHTML.createElement("span", { "class":`mono file-name`,});f; }
             }
          }
        }
       }

      if(isMobile && selected){
        
         { svelteHTML.createElement("button", {       "class":`mob-sec-head mob-detail-head`,"onclick":() => (secFilesOpen = true),"aria-expanded":`true`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":13,}});}
           { svelteHTML.createElement("span", { "class":`mob-detail-title`,});selected.split('/').pop(); }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("span", { "class":`mob-back`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronUp`,"size":12,}});}  }
         }
      }

      
       { svelteHTML.createElement("div", { "class":`file-detail`,});
        if(selected && isResolved(selected)){
          
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(`${selected.split('/').pop()} is resolved.`)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`detail-empty dim`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":14,}});}
             { svelteHTML.createElement("span", {});selected.split('/').pop();   }
           }}
        } else if (selected){
          selected; {
             { const $$_enaPeliFtcilfnoC3C = __sveltets_2_ensureComponent(ConflictFilePane); new $$_enaPeliFtcilfnoC3C({ target: __sveltets_2_any(), props: {        repoId,"path":selected,oursLabel,theirsLabel,"onresolved":handleResolved,}});}
          }
        }else{
           { svelteHTML.createElement("div", { "class":`detail-empty dim`,});
             { svelteHTML.createElement("span", {});allFiles.length > 0 && allFiles.every(isResolved) ? 'All files resolved — continue the operation below.' : 'Select a file to resolve its conflicts.'; }
           }
        }
       }
    }
   }

   { svelteHTML.createElement("footer", { "class":`resolver-foot`,});
    if(op !== null){
       { svelteHTML.createElement("button", {     "class":`btn danger`,"disabled":busy !== '',"onclick":abort,});
        busy === 'abort' ? 'Aborting…' : `Abort ${opName}`;
       }
    }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(!allResolved && allFiles.length > 0){
       { svelteHTML.createElement("span", { "class":`dim foot-hint`,});    op === null ? 'finish' : `complete the ${opName}`;  }
    }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!allResolved || busy !== '',"onclick":complete,});
      busy === 'complete' ? 'Completing…' : op === null ? 'Done' : `Complete ${opName}`;
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConflictResolverView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConflictResolverView__SvelteComponent_ = ReturnType<typeof ConflictResolverView__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConflictResolverView__SvelteComponent_;