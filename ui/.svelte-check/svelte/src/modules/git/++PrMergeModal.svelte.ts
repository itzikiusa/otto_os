///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
// Merge confirmation for a hosted PR. Nothing merges until the user clicks
// Merge here — and the modal first shows everything that argues against it:
// per-check CI, approvals, mergeability, open blocker findings, and the two
// facts only the local checkout knows (unpushed commits, branch freshness).
// Blocking reasons disable Merge until "Merge anyway" is ticked.
import { onDestroy } from 'svelte';
import { api, ApiError } from '../../lib/api/client';
import type { MergeStrategy, PrChecksResp, PrDetail, PrReadiness } from '../../lib/api/types';
import { router } from '../../lib/router.svelte';
import { toasts } from '../../lib/toast.svelte';
import { openExternal } from '../../lib/external';
import Modal from '../../lib/components/Modal.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import { pinnedHead } from './pr-cache';
;

  interface Props {
    repoId: string;
    number: number;
    pr: PrDetail;
    onclose: () => void;
    onmerged: () => void;
    /** The PR moved under the modal (a pinned merge 409'd): the parent should
     *  re-read its own (now stale) copy of the PR. */
    onstale?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, number, pr, onclose, onmerged, onstale }: Props = $props();

  let disposed = false;
  onDestroy(() => { disposed = true; });

  let strategy: MergeStrategy = $state('merge');
  let deleteSource = $state(false);
  let mergeAnyway = $state(false);
  let merging = $state(false);
  let error = $state<string | null>(null);

  // Both probes are best-effort, but a failed probe is NOT "all clear": the
  // row reads "Unavailable" with a Retry, and Merge needs "Merge anyway" until
  // CI / readiness could actually be verified.
  let checks = $state<PrChecksResp | null>(null);
  let checksLoading = $state(true);
  let checksError = $state(false);
  let readiness = $state<PrReadiness | null>(null);
  let readinessLoading = $state(true);
  let readinessError = $state(false);
  // Bumped by Retry to re-run the probes.
  let probeTick = $state(0);

  // The head the merge is PINNED to, read by the modal itself alongside the
  // probes (S15-305). `pr` is the parent's copy — possibly a cache hit from
  // minutes ago — so pinning `pr.head_sha` 409'd forever after a push while
  // the CI row showed the live head's checks.
  let liveHead = $state<string | null>(null);
  let headLoading = $state(true);
  $effect(() => {
    const id = repoId;
    const n = number;
    void probeTick;
    headLoading = true;
    void api
      .get<PrDetail>(`/repos/${id}/prs/${n}`)
      .then((r) => (liveHead = r.head_sha ?? null))
      // Unreadable: fall back to the parent's head — the daemon still refuses
      // a merge whose head moved, so this can only fail closed.
      .catch(() => (liveHead = null))
      .finally(() => (headLoading = false));
  });
  const pinSha = $derived(pinnedHead(liveHead, pr.head_sha ?? null));

  $effect(() => {
    const id = repoId;
    const n = number;
    void probeTick;
    checksLoading = true;
    checksError = false;
    void api
      .get<PrChecksResp>(`/repos/${id}/prs/${n}/checks`)
      .then((r) => (checks = r))
      .catch(() => {
        checks = null;
        checksError = true;
      })
      .finally(() => (checksLoading = false));
  });

  $effect(() => {
    const id = repoId;
    const n = number;
    void probeTick;
    readinessLoading = true;
    readinessError = false;
    void api
      .get<PrReadiness>(`/repos/${id}/prs/${n}/readiness`)
      .then((r) => (readiness = r))
      .catch(() => {
        readiness = null;
        readinessError = true;
      })
      .finally(() => (readinessLoading = false));
  });

  const blockers = $derived(readiness?.review?.unresolved_blocker_count ?? 0);
  const mergeable = $derived(readiness?.mergeable ?? pr.mergeable ?? null);

  // What argues against merging. Empty ⇒ Merge is enabled outright.
  const reasons = $derived(
    [
      checks?.ci.state === 'failure' ? 'CI failing' : null,
      mergeable === false ? 'Not mergeable' : null,
      blockers > 0 ? plural(blockers, 'blocker finding') : null,
      checksError || readinessError ? 'Unable to verify CI / readiness' : null,
    ].filter((r): r is string => r !== null),
  );
  const loading = $derived(checksLoading || readinessLoading || headLoading);
  const canMerge = $derived(!merging && !loading && (reasons.length === 0 || mergeAnyway));

  const FRESHNESS: Record<string, string> = {
    fresh: 'Up to date with the target branch',
    behind: 'Behind the target branch',
    unknown: 'Unknown (branch not checked out locally)',
  };

  function checkIcon(state: string): { icon: IconName; cls: string } {
    if (state === 'success') return { icon: 'check', cls: 'ok' };
    if (state === 'failure') return { icon: 'x', cls: 'bad' };
    if (state === 'skipped' || state === 'neutral') return { icon: 'dot', cls: 'dim' };
    // Anything else is still pending / in progress — waiting on CI.
    return { icon: 'clock', cls: 'warn' };
  }

  /** Persist the Review tab and route to the PR — PrDetail honours the stored
   *  tab when this modal closes, so the blockers link lands on the findings. */
  function openReview(): void {
    try {
      localStorage.setItem(`otto_pr_tab_${repoId}_${number}`, 'review');
    } catch {
      /* private mode — the route still moves */
    }
    router.go(`git/${repoId}/pr/${number}`);
    onclose();
  }

  async function doMerge(): Promise<void> {
    if (!canMerge) return;
    const mergedNumber = number;
    merging = true;
    error = null;
    try {
      // Pin the merge to the head these checks/readiness rows describe: a push
      // while the modal is open must 409, not merge unreviewed commits.
      await api.post(`/repos/${repoId}/prs/${number}/merge`, {
        strategy,
        delete_source_branch: deleteSource,
        ...(pinSha ? { expected_head_sha: pinSha } : {}),
      });
      toasts.success('PR merged', `#${mergedNumber}`);
      if (!disposed) onmerged();
    } catch (e) {
      // Keep the provider's own words — "Required status check failing" and
      // friends are the actionable text.
      const moved = e instanceof ApiError && e.status === 409 && /head|changed|sha/i.test(e.message);
      const detail = moved
        ? 'The pull request changed since these checks ran (new commits were pushed). The checks below were re-read for the new head — review them, then merge again.'
        : e instanceof ApiError || e instanceof Error
          ? e.message
          : String(e);
      if (disposed) toasts.error(`Couldn’t merge PR #${mergedNumber}`, detail);
      else error = detail;
      // Recover instead of dead-ending: re-read the head + probes here, and let
      // the parent refresh its stale copy. Merge stays a fresh, explicit click.
      if (moved && !disposed) {
        mergeAnyway = false;
        probeTick++;
        onstale?.();
      }
    } finally {
      merging = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Merge pull request`,"width":520,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", { "class":`foot`,});
      if(reasons.length > 0){
         { svelteHTML.createElement("span", {  "class":`reasons`,});mergeAnyway;reasons.join(' · '); }
      }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {     "class":`btn ghost`,"onclick":onclose,"disabled":merging,});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":doMerge,"disabled":!canMerge,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":12,}});}
        merging ? 'Merging…' : 'Merge pull request';
       }
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`body`,});
     { svelteHTML.createElement("div", { "class":`prline`,});
       { svelteHTML.createElement("span", { "class":`dim`,}); number; }
       { svelteHTML.createElement("span", { "class":`ptitle`,});pr.title; }
     }
     { svelteHTML.createElement("div", { "class":`flow mono dim`,});
      pr.source_branch;  { svelteHTML.createElement("span", { "class":`dir-arrow`,});  } pr.target_branch;
      if(pinSha){ { svelteHTML.createElement("span", { "title":`The merge is pinned to this head commit`,});  pinSha.slice(0, 7); }}
     }

    
     { svelteHTML.createElement("div", { "class":`rows`,});
       { svelteHTML.createElement("div", { "class":`row-item`,});
         { svelteHTML.createElement("span", { "class":`rlabel`,});  }
         { svelteHTML.createElement("span", { "class":`rvalue`,});
          if(checksLoading){
             { svelteHTML.createElement("span", {       "class":`spinner`,"role":`img`,"aria-label":`Loading checks`,"title":`Loading checks`,}); }
          } else if (checksError || checks === null){
             { svelteHTML.createElement("span", { "class":`warn`,});  }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => probeTick++,});  }
          } else if (checks.checks.length === 0){
             { svelteHTML.createElement("span", { "class":`dim`,});   }
          }else{
             { svelteHTML.createElement("span", { "class":`checks`,});
                  for(let c of __sveltets_2_ensureArray(checks.checks)){let i = 1;i;
                const ic = checkIcon(c.state);
                if(c.url){
                   { svelteHTML.createElement("button", {       "class":`check link ${ic.cls}`,"title":`${c.name} — ${c.state}`,"onclick":() => void openExternal(c.url),});
                     { svelteHTML.createElement("span", { "class":`glyph`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":ic.icon,"size":12,}});} }c.name;
                   }
                }else{
                   { svelteHTML.createElement("span", {   "class":`check ${ic.cls}`,"title":`${c.name} — ${c.state}`,});
                     { svelteHTML.createElement("span", { "class":`glyph`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":ic.icon,"size":12,}});} }c.name;
                   }
                }
              }
             }
          }
         }
       }

       { svelteHTML.createElement("div", { "class":`row-item`,});
         { svelteHTML.createElement("span", { "class":`rlabel`,});  }
         { svelteHTML.createElement("span", { "class":`rvalue`,});
          if(readinessLoading){ { svelteHTML.createElement("span", {       "class":`spinner`,"role":`img`,"aria-label":`Loading approvals`,"title":`Loading approvals`,}); }
          } else if (readinessError || readiness === null){
             { svelteHTML.createElement("span", { "class":`warn`,});  }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => probeTick++,});  }
          }else{readiness.approvals;}
         }
       }

       { svelteHTML.createElement("div", { "class":`row-item`,});
         { svelteHTML.createElement("span", { "class":`rlabel`,});  }
         { svelteHTML.createElement("span", { "class":`rvalue`,});
          if(mergeable === true){ { svelteHTML.createElement("span", { "class":`ok`,});  }
          } else if (mergeable === false){ { svelteHTML.createElement("span", { "class":`bad`,});      }
          }else{ { svelteHTML.createElement("span", { "class":`dim`,});  }}
         }
       }

       { svelteHTML.createElement("div", { "class":`row-item`,});
         { svelteHTML.createElement("span", { "class":`rlabel`,});  }
         { svelteHTML.createElement("span", { "class":`rvalue`,});
          if(readinessLoading){ { svelteHTML.createElement("span", {       "class":`spinner`,"role":`img`,"aria-label":`Loading blockers`,"title":`Loading blockers`,}); }
          } else if (readiness?.review){
             { svelteHTML.createElement("span", { });blockers > 0;blockers; }
             { svelteHTML.createElement("span", { "class":`dim`,}); readiness.review.unresolved_total;  }
             { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":openReview,});  }
          } else if (readinessError){
             { svelteHTML.createElement("span", { "class":`warn`,});  }
          }else{
             { svelteHTML.createElement("span", { "class":`dim`,});   }
          }
         }
       }

      if(readiness && readiness.unpushed !== null){
         { svelteHTML.createElement("div", { "class":`row-item`,});
           { svelteHTML.createElement("span", { "class":`rlabel`,});   }
           { svelteHTML.createElement("span", {  "class":`rvalue`,});readiness.unpushed > 0;readiness.unpushed; }
         }
      }

      if(readiness){
         { svelteHTML.createElement("div", { "class":`row-item`,});
           { svelteHTML.createElement("span", { "class":`rlabel`,});  }
           { svelteHTML.createElement("span", {  "class":`rvalue`,});readiness.branch_freshness === 'behind';
            FRESHNESS[readiness.branch_freshness] ?? readiness.branch_freshness;
           }
         }
      }
     }

    
     { svelteHTML.createElement("div", { "class":`opts`,});
       { svelteHTML.createElement("label", { "class":`opt`,});
         { svelteHTML.createElement("span", { "class":`rlabel`,});  }
         { svelteHTML.createElement("select", {     "class":`input`,"bind:value":strategy,"style":`width: 140px`,});/*Ωignore_startΩ*/() => strategy = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`merge`,});  }
           { svelteHTML.createElement("option", { "value":`squash`,});  }
           { svelteHTML.createElement("option", { "value":`rebase`,});  }
         }
       }
       { svelteHTML.createElement("label", { "class":`check-opt`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":deleteSource,});/*Ωignore_startΩ*/() => deleteSource = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", {});     }
       }
      if(reasons.length > 0){
         { svelteHTML.createElement("label", { "class":`check-opt anyway`,});
           { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":mergeAnyway,});/*Ωignore_startΩ*/() => mergeAnyway = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {});  }
         }
      }
     }

    if(error){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Couldn’t merge. ${error}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`err`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});} { svelteHTML.createElement("span", {});  error; } }}
    }
     { svelteHTML.createElement("p", { "class":`who dim`,});
        { svelteHTML.createElement("span", { "class":`mono`,});pr.source_branch; }   { svelteHTML.createElement("span", { "class":`mono`,});pr.target_branch; }   
                    
     }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PrMergeModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PrMergeModal__SvelteComponent_ = ReturnType<typeof PrMergeModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default PrMergeModal__SvelteComponent_;