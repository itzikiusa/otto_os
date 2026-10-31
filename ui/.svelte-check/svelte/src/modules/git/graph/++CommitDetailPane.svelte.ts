///<reference types="svelte" />
;// The git graph's right pane (arch-1-09: split out of GraphView.svelte): the
// WIP staging panel when the working-tree row is selected, otherwise the
// selected commit's header (sha, refs, subject, author, the "on <branch>"
// hint) over its summary-first diff. On a phone it is the accordion's last
// section; GraphView owns the selection (graphSelection.svelte.ts).

import type { CommitInfo, RepoStatusResp } from '../../../lib/api/types';
import { rel } from '../../../lib/stores/now.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import WipPanel from '../WipPanel.svelte';
import GraphCommitDiff from '../graph-commit-diff.svelte';
import RefChip from './RefChip.svelte';
import type { RefChip as Chip } from '../ref-menus';
;

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    onresolve?: () => void;
    /** Desktop: the pane takes the rest of the width when open. */
    open: boolean;
    isMobile: boolean;
    wipSelected: boolean;
    selectedSha: string | null;
    selectedCommit: CommitInfo | null;
    /** The selected commit's ref chips (GraphView's `chipsFor`). */
    chips: Chip[];
    stashLabel: (sha: string) => string;
    /** The branch the highlighted spine resolves to, if any. */
    selectedBranch: string | null;
    diffSha: string | null;
    diffHeld: boolean;
    onresume: () => void;
    onfiletools: (e: MouseEvent, path: string) => void;
    fileCount?: number | null;
    onclose: () => void;
    oncommitted: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let {
    repoId,
    status,
    onstatus,
    onresolve,
    open,
    isMobile,
    wipSelected,
    selectedSha,
    selectedCommit,
    chips,
    stashLabel,
    selectedBranch,
    diffSha,
    diffHeld,
    onresume,
    onfiletools,
    fileCount = $bindable(null),
    onclose,
    oncommitted,
  }: Props = $props()/*Ωignore_startΩ*/;fileCount;/*Ωignore_endΩ*/;

  /** The shared relative-time label (content.md §7), compact for dense rows. */
  function fmtDate(iso: string): string {
    return rel(iso, { compact: true });
  }
  function fmtDateTitle(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleString();
  }
;
async () => {

  { svelteHTML.createElement("div", {     "class":`detail-panel`,});open && !isMobile;isMobile && open;isMobile && !open;
  if(wipSelected){
     { const $$_lenaPpiW1C = __sveltets_2_ensureComponent(WipPanel); new $$_lenaPpiW1C({ target: __sveltets_2_any(), props: {      repoId,status,onstatus,oncommitted,onclose,onresolve,}});}
  } else if (selectedSha === null){
     { svelteHTML.createElement("div", { "class":`detail-empty`,});
       { svelteHTML.createElement("span", { "class":`detail-empty-label`,});  }
       { svelteHTML.createElement("span", { "class":`dim detail-empty-hint`,});   }
     }
  } else if (selectedCommit !== null){
     { svelteHTML.createElement("div", { "class":`detail-header`,});
       { svelteHTML.createElement("div", { "class":`detail-header-main`,});
        
         { svelteHTML.createElement("div", { "class":`detail-title-row`,});
           { svelteHTML.createElement("span", { "class":`mono detail-sha`,});selectedCommit.short_sha; }
           { svelteHTML.createElement("span", { "class":`detail-refs`,});
              for(let chip of __sveltets_2_ensureArray(chips)){
               { const $$_pihCfeR5C = __sveltets_2_ensureComponent(RefChip); new $$_pihCfeR5C({ target: __sveltets_2_any(), props: {      chip,"label":chip.kind === 'stash' ? stashLabel(selectedCommit.sha) : chip.label,"title":chip.worktree && !chip.current
                  ? `Worktree · ${chip.label}`
                  : chip.current
                    ? `Checked out · ${chip.label}`
                    : chip.label,}});}
            }
           }
           { svelteHTML.createElement("button", {       "class":`icon-btn detail-close`,"onclick":onclose,"title":`Close commit detail`,"aria-label":`Close commit detail`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
           }
         }
         { svelteHTML.createElement("div", { "class":`detail-subject`,});selectedCommit.subject; }
         { svelteHTML.createElement("div", { "class":`detail-meta`,});
           { svelteHTML.createElement("span", { "class":`detail-author`,});selectedCommit.author; }
           { svelteHTML.createElement("span", { "class":`dim detail-dot`,});  }
           { svelteHTML.createElement("span", {   "class":`dim detail-date`,"title":fmtDateTitle(selectedCommit.date),});fmtDate(selectedCommit.date); }
          if(selectedBranch && !chips.some((c) => c.label === selectedBranch)){
             { svelteHTML.createElement("span", {   "class":`on-branch-hint`,"title":`On branch ${selectedBranch}`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});} { svelteHTML.createElement("span", { "class":`chip-label`,});selectedBranch; }
             }
          }
         }
       }
     }

    
      { const $$_ffiDtimmoChparG1C = __sveltets_2_ensureComponent(GraphCommitDiff); const $$_ffiDtimmoChparG1 = new $$_ffiDtimmoChparG1C({ target: __sveltets_2_any(), props: {       repoId,"sha":diffSha,"held":diffHeld,onresume,onfiletools,fileCount,}});/*Ωignore_startΩ*/() => fileCount = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_ffiDtimmoChparG1.$$bindings = 'fileCount';}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('fileCount'), slots: {}, events: {} }}
const CommitDetailPane__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CommitDetailPane__SvelteComponent_ = ReturnType<typeof CommitDetailPane__SvelteComponent_>;
/*Ωignore_endΩ*/export default CommitDetailPane__SvelteComponent_;