///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
// Explicit merge approval. Opened by dropping one local branch onto another in
// the graph's refs panel — NOTHING merges until the user clicks Merge here.
// Shows source → target, a strategy picker, a dry-run conflict preview, and a
// dirty-tree notice: the merge proceeds via stash → merge → pop (auto_stash),
// it is NOT blocked. A predicted conflict warns and routes into the resolver.
import type {
    LocalMergeStrategy,
    MergePreview,
    MergeResult,
    RepoStatusResp,
  } from '../../lib/api/types';
import { api, ApiError } from '../../lib/api/client';
import { git } from '../../lib/stores/git.svelte';
import { toasts } from '../../lib/toast.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
;

  interface Props {
    repoId: string;
    source: string;
    target: string;
    onclose: () => void;
    /** status 'merged' / 'up_to_date' — refresh + close handled by parent. */
    onmerged: (result: MergeResult) => void;
    /** status 'conflicts' — parent should open the conflict resolver. */
    onconflicts: (result: MergeResult) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, source, target, onclose, onmerged, onconflicts }: Props = $props();

  const STRATEGIES: { id: LocalMergeStrategy; label: string; hint: string }[] = [
    { id: 'merge_commit', label: 'Merge commit', hint: 'Always create a merge commit' },
    { id: 'ff', label: 'Fast-forward', hint: 'FF when possible, else a merge commit' },
    { id: 'ff_only', label: 'Fast-forward only', hint: 'Refuse if a merge commit is needed' },
    { id: 'squash', label: 'Squash', hint: 'Collapse source into one commit' },
  ];

  let strategy = $state<LocalMergeStrategy>('merge_commit');
  let merging = $state(false);
  let error = $state<string | null>(null);
  /** A next-step hint under the raw error, when we recognize the failure. */
  let errorHint = $state<string | null>(null);

  // Read the current repo status for the dirty-tree check. Seed from the store
  // (fast, avoids a flash) and always refresh from the daemon so we never merge
  // on a stale read.
  let status = $state<RepoStatusResp | null>(seedStatus());
  let statusLoading = $state(true);

  function seedStatus(): RepoStatusResp | null {
    return git.primary?.id === repoId ? git.primaryStatus : null;
  }

  // Pre-merge conflict check (dry-run; never touches the working tree). When it
  // predicts conflicts we BLOCK the merge and tell the user to resolve first.
  let preview = $state<MergePreview | null>(null);
  let previewLoading = $state(true);

  $effect(() => {
    const id = repoId;
    statusLoading = true;
    void api
      .get<RepoStatusResp>(`/repos/${id}/status`)
      .then((s) => (status = s))
      .catch(() => {
        /* keep seeded status */
      })
      .finally(() => (statusLoading = false));
  });

  $effect(() => {
    const id = repoId;
    previewLoading = true;
    void git
      .mergePreview(id, source, target)
      .then((p) => (preview = p))
      .catch(() => (preview = null))
      .finally(() => (previewLoading = false));
  });

  const dirty = $derived((status?.changes.length ?? 0) > 0);
  const willConflict = $derived(preview?.conflicts === true);
  // A predicted conflict WARNS, it never blocks: the merge is how you get into
  // the conflict resolver (the daemon leaves the merge in progress and returns
  // status:"conflicts", which the parent turns into the resolver view). Blocking
  // here used to make conflicting merges impossible to perform OR resolve in
  // Otto — the modal told you to "resolve on the source branch first", which
  // there was no in-app way to do.
  const canMerge = $derived(!merging && !statusLoading && !previewLoading);

  async function doMerge(): Promise<void> {
    if (!canMerge) return;
    merging = true;
    error = null;
    errorHint = null;
    try {
      // On a dirty tree, stash → merge → pop (the dry-run already cleared conflicts).
      const result = await git.mergeBranch(repoId, {
        source,
        target,
        strategy,
        auto_stash: dirty,
      });
      if (result.note) toasts.info(result.note);
      if (result.status === 'conflicts') {
        onconflicts(result);
        return;
      }
      // A squash merge reports status 'merged' but commit === null: the changes
      // are STAGED, not committed. Don't imply it's done — tell the user to
      // review & commit in Changes.
      if (result.status === 'merged' && result.commit === null) {
        toasts.success('Squash merge staged', 'Review and commit it from the WIP row at the top of the graph');
      } else {
        const verb = result.status === 'up_to_date' ? 'Already up to date' : 'Merged';
        toasts.success(verb, `${source} → ${target}`);
      }
      onmerged(result);
    } catch (e) {
      error = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
      // Turn the daemon's message into a next step where we recognize it,
      // instead of leaving the raw git line as a dead end.
      if (/already in progress/i.test(error)) {
        errorHint =
          'Finish or abort the in-progress operation first — open the "Resolve conflicts" tab.';
      } else if (/commit your changes|stash|overwritten/i.test(error)) {
        errorHint = 'Commit or stash your working changes, then retry the merge.';
      } else if (/ff-only|fast-forward/i.test(error)) {
        errorHint = 'A fast-forward isn’t possible — pick another strategy and retry.';
      }
    } finally {
      merging = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Merge branch`,"width":480,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn ghost`,"onclick":onclose,"disabled":merging,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":doMerge,"disabled":!canMerge,});
      if(merging){
        dirty ? 'Stashing & merging…' : 'Merging…';
      } else if (willConflict){   } else if (dirty){   }else{ }
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`body`,});
    
     { svelteHTML.createElement("div", { "class":`flow`,});
       { svelteHTML.createElement("span", {   "class":`bchip src`,"title":source,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`mono`,});source; }
       }
       { svelteHTML.createElement("span", { "class":`arrow`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":14,}});} }
       { svelteHTML.createElement("span", {   "class":`bchip tgt`,"title":target,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`mono`,});target; }
       }
     }

     { svelteHTML.createElement("p", { "class":`note`,});
           { svelteHTML.createElement("span", { "class":`mono em`,});target; }  
       { svelteHTML.createElement("span", { "class":`mono em`,});source; }
     }

    
     { svelteHTML.createElement("div", { "class":`section-label`,});  }
     { svelteHTML.createElement("div", { "class":`strats`,});
         for(let s of __sveltets_2_ensureArray(STRATEGIES)){s.id;
         { svelteHTML.createElement("label", {  "class":`strat`,});strategy === s.id;
           { svelteHTML.createElement("input", {       "type":`radio`,"name":`strategy`,"value":s.id,});strategy = __sveltets_2_any(null);}
           { svelteHTML.createElement("span", { "class":`strat-main`,});
             { svelteHTML.createElement("span", { "class":`strat-label`,});s.label; }
             { svelteHTML.createElement("span", { "class":`strat-hint dim`,});s.hint; }
           }
         }
      }
     }

    
    if(previewLoading){
       { svelteHTML.createElement("div", { "class":`warn check`,});
         { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }
         { svelteHTML.createElement("span", {});   }
       }
    } else if (willConflict){
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-head`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":14,}});}
           { svelteHTML.createElement("span", {});
              { svelteHTML.createElement("span", { "class":`mono`,});source; }   { svelteHTML.createElement("span", { "class":`mono`,});target; }
               plural(preview?.conflicted_files.length ?? 0, 'file');    
                            
             { svelteHTML.createElement("span", { "class":`mono`,});target; } 
           }
         }
        if(preview && preview.conflicted_files.length > 0){
           { svelteHTML.createElement("ul", { "class":`conflict-files`,});
               for(let f of __sveltets_2_ensureArray((preview.conflicted_files.slice(0, 12)))){f;
               { svelteHTML.createElement("li", { "class":`mono`,});f; }
            }
            if(preview.conflicted_files.length > 12){
               { svelteHTML.createElement("li", { "class":`more`,}); preview.conflicted_files.length - 12;  }
            }
           }
        }
       }
    } else if (dirty){
      
       { svelteHTML.createElement("div", { "class":`warn`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":14,}});}
         { svelteHTML.createElement("span", {});
            plural(status?.changes.length ?? 0, 'uncommitted change');
             { svelteHTML.createElement("strong", {});  }      { svelteHTML.createElement("strong", {});  }
         }
       }
    }

    if(error){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: error, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`err`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
         { svelteHTML.createElement("span", {});
          error;
          if(errorHint){ { svelteHTML.createElement("br", {});} { svelteHTML.createElement("strong", {});errorHint; }}
         }
       }}
    }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MergeApprovalModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MergeApprovalModal__SvelteComponent_ = ReturnType<typeof MergeApprovalModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default MergeApprovalModal__SvelteComponent_;