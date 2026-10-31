///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import { plural } from '../../lib/plural';
// One repo: toolbar header + tabs (Graph / Pull Requests / Review). Staging
// and history both live on the graph now (WIP row + detail panel), so there
// are no separate Changes/History tabs.
import type { GitOpInProgress, MergeResult, Repo, RepoStatusResp } from '../../lib/api/types';
import { onTabKey } from '../../lib/tabKeys';
import { router } from '../../lib/router.svelte';
import { git } from '../../lib/stores/git.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import GitToolbar from './GitToolbar.svelte';
import GraphView from './GraphView.svelte';
import GraphSearchBar from './GraphSearchBar.svelte';
import BlamePanel from './BlamePanel.svelte';
import FileHistoryPanel from './FileHistoryPanel.svelte';
import RemotesPanel from './RemotesPanel.svelte';
import RecoveryTools from './RecoveryTools.svelte';
import { gitBridge } from './gitBridge.svelte';
import LazyMount from '../../lib/components/LazyMount.svelte';
import { lazyComponent } from '../../lib/lazy-component.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import { toasts } from '../../lib/toast.svelte';
;

  interface Props {
    repo: Repo;
    tab: string; // graph | prs | review (legacy changes/history render the graph)
    /** Embedded in the agent-mode right panel: switch tabs via `onTab` (local
     *  state) instead of routing, and hide the "← Repos" back button. */
    embedded?: boolean;
    onTab?: (tab: string) => void;
    /** Git page (tab-store owned navigation): open another repo as a tab. The
     *  route-based fallback below is a no-op there once the route is `#/git`. */
    onopenrepo?: (repoId: string) => void;
    /** Git page: open its Add Repository sheet in the given mode. */
    onaddrepo?: (mode: 'register' | 'clone' | 'browse') => void;
    /** Git page: the verbs (Fetch…Pop) and ⋯ live in the page's PageHeader, so
     *  this row is only the in-pane view switcher. */
    hostActions?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Views only some visits open stay out of the Git chunk (perf R7): the PR
  // list (+ CreatePr), the local review, the merge dialog and the conflict
  // resolver load on first use, then mount synchronously (lazyComponent).
  // Graph and WIP — what every visit shows — stay eager.
  const PrListLazy = lazyComponent(() => import('./PrList.svelte'));
  const LocalReviewLazy = lazyComponent(() => import('./LocalReviewPanel.svelte'));
  const MergeApprovalLazy = lazyComponent(() => import('./MergeApprovalModal.svelte'));
  const ConflictResolverLazy = lazyComponent(() => import('./ConflictResolverView.svelte'));

  let { repo, tab, embedded = false, onTab, onopenrepo, onaddrepo, hostActions = false }: Props = $props();

  // Legacy deep links / persisted state may still say 'changes' or 'history';
  // both render the graph now, and the Graph tab must read as active for them.
  // Legacy ids are the graph: changes/history, and focus (Focus is a
  // page-level view of the Git page now, not a repo sub-tab).
  const effTab = $derived(tab === 'changes' || tab === 'history' || tab === 'focus' ? 'graph' : tab);

  // Status is owned by the git store (single source of truth) so the auto-fetch
  // loop, the tab strip and this toolbar all stay in sync from one fetch.
  const status = $derived(git.statusById[repo.id] ?? null);

  // ── Merge / conflict-resolution state ──────────────────────────────────────
  // The pending merge approval (set when a branch is dropped onto another).
  let mergeReq = $state<{ source: string; target: string } | null>(null);
  // True while the conflict resolver view is showing instead of the tabs.
  let resolving = $state(false);
  // Seed data carried from a conflicting merge into the resolver.
  let conflictSeed = $state<{ files: string[]; source: string | null }>({ files: [], source: null });
  // True when ANY resolvable operation is in progress (merge / rebase /
  // cherry-pick / revert, or conflicted files with no state file, e.g. a
  // conflicting stash pop) — surfaces the "Resolve conflicts" banner.
  let merging = $state(false);
  // Which operation it is; null for state-file-less conflicts (stash pop).
  let mergeOp = $state<GitOpInProgress | null>(null);

  /** Human label for the in-progress operation, banner + resolver wording. */
  const opLabel = $derived(
    mergeOp === 'rebase'
      ? 'rebase'
      : mergeOp === 'cherry_pick'
        ? 'cherry-pick'
        : mergeOp === 'revert'
          ? 'revert'
          : mergeOp === 'merge'
            ? 'merge'
            : null,
  );

  // A failed FIRST status load (folder moved or deleted, daemon down) used to
  // leave the toolbar and graph skeletons pulsing forever. The store records the
  // failure (`statusErrorById`); it only replaces the view while there is no
  // status at all — once any status exists, a later failure keeps the stale one.
  const statusError = $derived(status ? null : (git.statusErrorById[repo.id] ?? null));
  let statusLoading = $state(false);
  function loadStatus(id: string): void {
    statusLoading = true;
    void git.refreshStatus(id).finally(() => {
      if (id === repo.id) statusLoading = false;
    });
  }

  $effect(() => {
    const id = repo.id;
    resolving = false;
    merging = false;
    mergeOp = null;
    mergeReq = null;
    // Status lives in the store; (re)load it for this repo. The auto-fetch loop
    // keeps it fresh thereafter, and the tab strip shares the same value.
    loadStatus(id);
  });

  // Unmerged paths as the status reports them. A merge in progress ALWAYS shows
  // up here (kind="conflicted"), whoever started it — the merge modal, a pull
  // whose merge conflicted, an agent session, or git on the command line.
  const conflictedPaths = $derived(
    (status?.changes ?? []).filter((c) => c.kind === 'conflicted').map((c) => c.path),
  );
  // Primitive key so the effect below re-runs when the conflict SET changes —
  // not on every status poll (each poll hands back a fresh array reference).
  const conflictKey = $derived(conflictedPaths.join(' '));
  // Re-checked whenever that set changes, so the banner + "Resolve conflicts"
  // tab appear as soon as the repo enters a merge — not only on mount. (This
  // used to be a mount-only effect keyed on repo.id: a mid-session conflict left
  // the incoming files sitting in WIP with no way into the resolver until you
  // navigated away and back.)
  $effect(() => {
    const id = repo.id;
    const seen = conflictKey;
    void git
      .getMergeStatus(id)
      .then((m) => {
        // A late reply for a repo this view no longer shows must not paint
        // its conflicts here.
        if (id !== repo.id) return;
        merging = m.merging;
        mergeOp = m.op ?? null;
        if (m.merging) {
          conflictSeed = {
            files: m.conflicted_files.length > 0 ? m.conflicted_files : conflictedPaths,
            source: m.source ?? conflictSeed.source,
          };
        } else if (!seen) {
          conflictSeed = { files: [], source: null };
        }
      })
      .catch(() => {
        if (id !== repo.id) return;
        // Fall back to what the status already told us rather than hiding a
        // merge the user is standing in.
        merging = conflictedPaths.length > 0;
        mergeOp = status?.op_in_progress ?? null;
      });
  });

  function setStatus(s: RepoStatusResp): void {
    git.setStatus(repo.id, s);
  }

  function requestMerge(source: string, target: string): void {
    mergeReq = { source, target };
  }

  function onMerged(result: MergeResult): void {
    mergeReq = null;
    setStatus(result.repo_status);
    merging = false;
    // Refresh refs/commits is handled inside GraphView's own effect on status
    // change; force a light status reconcile + PR refresh via the store.
    if (git.primary?.id === repo.id) void git.refreshPrimary();
    // Re-read refs + log in place (keeps the graph's selection and scroll).
    gitBridge.refreshGraph(repo.id);
  }

  function onConflicts(result: MergeResult): void {
    // Capture the source label before clearing the request.
    const source = mergeReq?.source ?? null;
    mergeReq = null;
    setStatus(result.repo_status);
    merging = true;
    conflictSeed = { files: result.conflicted_files, source };
    resolving = true;
  }

  function openResolver(): void {
    resolving = true;
  }

  function leaveResolver(): void {
    resolving = false;
    merging = false;
    setStatusFromDaemon();
    if (git.primary?.id === repo.id) void git.refreshPrimary();
    // The graph remounts when the resolver closes; its cached snapshot is
    // revalidated against `/refs`, and this covers a graph that stayed mounted.
    gitBridge.refreshGraph(repo.id);
  }

  function setStatusFromDaemon(): void {
    void git.refreshStatus(repo.id);
  }

  // Remote CRUD lives behind a header button + modal (the branch bar is a
  // different owner's surface, and remotes are a repo-level setting, not a
  // per-branch one).
  let remotesOpen = $state(false);

  /** ⋯ in the toolbar row: the repo-level tools that don't earn a permanent
   *  button (remotes are a setting; recovery is rare and deliberate). */
   function openMoreMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Remotes…', icon: 'globe', action: () => (remotesOpen = true) },
      { label: 'Recovery tools…', icon: 'undo', action: () => gitBridge.openRecovery(repo.id) },
      { separator: true },
      {
        label: 'Copy repository path',
        icon: 'copy',
        action: () =>
          void copyTextOrThrow(repo.path)
            .then(() => toasts.success('Copied', repo.path))
            .catch(() => toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.')),
      },
    ]);
  }

  // History / blame open as a right-side drawer over the graph. The request
  // comes through `gitBridge` from whichever diff header asked for it, so a
  // stale request for ANOTHER repo never renders here.
  const fileTool = $derived(
    gitBridge.fileTool?.repoId === repo.id ? gitBridge.fileTool : null,
  );

  /** Host of a remote URL for the unsupported-forge message — handles
   *  https://, ssh:// and scp-like (git@host:path) forms (mirrors the daemon's
   *  detect.rs split). */
  function remoteHost(url: string | null): string {
    if (!url) return 'this remote';
    const u = url.trim();
    for (const scheme of ['https://', 'http://', 'ssh://', 'git://']) {
      if (u.startsWith(scheme)) {
        const rest = u.slice(scheme.length).replace(/^[^@/]+@/, '');
        return rest.split('/')[0]?.split(':')[0] || 'this remote';
      }
    }
    const scp = u.match(/^(?:[^@/]+@)?([^:/]+):/);
    return scp?.[1] ?? 'this remote';
  }

  // Per-repo views only. Focus (my PRs + my Jira work) is cross-repo: it is a
  // page-level view of the Git page (GitPage's header), not a sub-tab here.
  const tabs = [
    { id: 'graph', label: 'Graph' },
    { id: 'prs', label: 'Pull requests' },
    { id: 'review', label: 'Review' },
  ];

  function selectTab(id: string): void {
    resolving = false;
    if (embedded) onTab?.(id);
    else router.go(`git/${repo.id}/${id}`);
  }


  // Repo switcher: jump between repositories without going back to the list.
  // Prefer the workspace-independent global list (Git page); fall back to the
  // per-workspace list when embedded in the agent right panel (global not loaded).
  const repoPool = $derived(git.allRepos.length ? git.allRepos : git.repos);
  function openRepoSwitcher(e: MouseEvent): void {
    const others = repoPool.filter((r) => r.id !== repo.id);
    // On the Git page the tab store owns navigation, so `router.go('git')` is a
    // no-op there (the route already IS `#/git`) — "Add repository…" / "All
    // repositories…" used to do nothing. Use the page's own callbacks when given.
    const tail = onaddrepo
      ? [
          { label: 'Clone a repository…', icon: 'download', pinned: true, action: () => onaddrepo('clone') },
          { label: 'Add a local repository…', icon: 'folder', pinned: true, action: () => onaddrepo('register') },
        ]
      : [
          { label: 'Add repository…', icon: 'plus', action: () => router.go('git') },
          { label: 'All repositories…', icon: 'folder', action: () => router.go('git') },
        ];
    ctxMenu.show(
      e,
      [
        ...others.map((r) => ({
          label: r.name,
          icon: 'branch',
          action: () => (onopenrepo ? onopenrepo(r.id) : router.go(`git/${r.id}/${tab}`)),
        })),
        ...(others.length > 0 ? [{ separator: true }] : []),
        ...tail,
      ],
      others.length > 8 ? { filter: true, filterPlaceholder: 'Filter repositories…', maxVisible: 12 } : undefined,
    );
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`repoview`,});
  
   { svelteHTML.createElement("div", {      "class":`rv-head`,"role":`group`,"aria-label":`${repo.name} toolbar`,});hostActions;
    if(!embedded){
       { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => router.go('git'),}); { svelteHTML.createElement("span", {   "class":`rv-back-arrow`,"aria-hidden":`true`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});} }  }
    }
    if(!onopenrepo){
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {          "class":`rv-name rv-switch`,"title":`${repo.name} · ${repo.path} — switch repository${repoPool.length > 1 ? ` (${repoPool.length} registered)` : ''}`,"onclick":openRepoSwitcher,"oncontextmenu":openRepoSwitcher,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":14,}});}
         { svelteHTML.createElement("span", { "class":`rv-name-text`,});repo.name; }
        if(repoPool.length > 1){ { svelteHTML.createElement("span", { "class":`rv-count`,});repoPool.length; }}
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
       }}
    }
     { svelteHTML.createElement("div", {     "class":`rv-tabs segmented`,"role":`tablist`,"aria-label":`${repo.name} views`,});
         for(let t of __sveltets_2_ensureArray(tabs)){t.id;
         { svelteHTML.createElement("button", {              "class":`rv-tab`,"role":`tab`,"aria-selected":effTab === t.id && !resolving,"tabindex":effTab === t.id && !resolving ? 0 : -1,"onclick":() => selectTab(t.id),"onkeydown":onTabKey,});effTab === t.id && !resolving;
          t.label;
          if(t.id === 'graph' && status && status.changes.length > 0){
             { svelteHTML.createElement("span", {   "class":`count`,"title":`${plural(status.changes.length, 'uncommitted change')} (WIP)`,});status.changes.length; }
          }
          if(t.id === 'graph' && conflictedPaths.length > 0){
             { svelteHTML.createElement("span", {   "class":`count conflict-count`,"title":plural(conflictedPaths.length, 'conflicted file'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}conflictedPaths.length; }
          }
         }
      }
      
      if(resolving){
         { svelteHTML.createElement("button", {         "class":`rv-tab conflict-tab active`,"role":`tab`,"aria-selected":`true`,"tabindex":0,"onkeydown":onTabKey,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":12,}});}
           
          if(conflictSeed.files.length > 0){
             { svelteHTML.createElement("span", { "class":`count conflict-count`,});conflictSeed.files.length; }
          }
         }
      }
     }
    if(!hostActions){
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(status){
         { const $$_rablooTtiG2C = __sveltets_2_ensureComponent(GitToolbar); new $$_rablooTtiG2C({ target: __sveltets_2_any(), props: {       "repoId":repo.id,status,"onstatus":setStatus,"onrefresh":() => gitBridge.refreshGraph(repo.id),}});}
      } else if (!statusError){
         { svelteHTML.createElement("div", { "class":`toolbar-skeleton`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":1,"height":26,"label":`repository status`,}});} }
      }
       { svelteHTML.createElement("button", {           "class":`icon-btn rv-more`,"title":`More repository actions`,"aria-label":`More repository actions`,"aria-haspopup":`menu`,"onclick":openMoreMenu,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":16,}});} }
    }
   }

  
  if(merging && !resolving){
     { svelteHTML.createElement("div", { "class":`merge-banner`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":13,}});}
       { svelteHTML.createElement("span", {});
        if(opLabel){ opLabel;   if(conflictSeed.source){' ';  { svelteHTML.createElement("span", { "class":`mono`,});conflictSeed.source; } } }else{        }
        if(conflictSeed.files.length > 0){
          conflictSeed.files.length === 1 ? '1 file needs' : `${conflictSeed.files.length} files need`; }
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":openResolver,});  }
     }
  }

   { svelteHTML.createElement("div", { "class":`rv-body`,});
    if(resolving){
       { const $$_tnuoMyzaL2C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL2C({ target: __sveltets_2_any(), props: {       "lazy":ConflictResolverLazy,"what":`the conflict resolver`,"props":{
          repoId: repo.id,
          initialFiles: conflictSeed.files,
          initialSource: conflictSeed.source,
          initialOp: mergeOp,
          onleave: leaveResolver,
        },}});}
    } else if (effTab === 'prs'){
      if(repo.forge === null && !repo.remote_url){
        
         { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {             "icon":`pr`,"title":`No remote yet`,"body":`Pull requests need a remote on GitHub, Bitbucket Cloud or GitLab. Add one, then publish your branch.`,"actionLabel":`Add a remote…`,"actionIcon":`globe`,"onaction":() => (remotesOpen = true),}});}
      } else if (repo.forge === 'unrecognized'){
        
         { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {       "icon":`pr`,"title":`Pull requests aren’t available for ${remoteHost(repo.remote_url)}`,"body":`Otto supports GitHub, Bitbucket Cloud, and GitLab. This repository’s remote isn’t one of them, so there’s no pull request view here.`,}});}
      }else{
         { const $$_tnuoMyzaL2C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL2C({ target: __sveltets_2_any(), props: {      "lazy":PrListLazy,"what":`pull requests`,"props":{ repoId: repo.id },}});}
      }
    } else if (effTab === 'review'){
       { svelteHTML.createElement("div", { "class":`rv-tab-scroll`,});
         { const $$_tnuoMyzaL3C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL3C({ target: __sveltets_2_any(), props: {        "lazy":LocalReviewLazy,"what":`the local review`,"variant":`panel`,"props":{ repoId: repo.id },}});}
       }
    } else if (status){
      
       { svelteHTML.createElement("div", { "class":`rv-graph`,});
         { const $$_raBhcraeShparG3C = __sveltets_2_ensureComponent(GraphSearchBar); new $$_raBhcraeShparG3C({ target: __sveltets_2_any(), props: {  "repoId":repo.id,}});}
         { svelteHTML.createElement("div", { "class":`rv-graph-body`,});
           { const $$_weiVhparG4C = __sveltets_2_ensureComponent(GraphView); new $$_weiVhparG4C({ target: __sveltets_2_any(), props: {              "repoId":repo.id,"repoPath":repo.path,"workspaceId":repo.workspace_id,status,"onstatus":setStatus,"onmergerequest":requestMerge,"onresolveconflicts":openResolver,}});}
          if(fileTool){
             { svelteHTML.createElement("aside", { "class":`rv-drawer`,});
              if(fileTool.kind === 'history'){
                 { const $$_lenaPyrotsiHeliF5C = __sveltets_2_ensureComponent(FileHistoryPanel); new $$_lenaPyrotsiHeliF5C({ target: __sveltets_2_any(), props: {       "repoId":repo.id,"path":fileTool.path,"onclose":() => gitBridge.closeFileTool(),}});}
              }else{
                 { const $$_lenaPemalB5C = __sveltets_2_ensureComponent(BlamePanel); new $$_lenaPemalB5C({ target: __sveltets_2_any(), props: {         "repoId":repo.id,"path":fileTool.path,"rev":fileTool.rev,"onclose":() => gitBridge.closeFileTool(),}});}
              }
             }
          }
         }
       }
    } else if (statusError){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {            "what":repo.name,"variant":`page`,"error":`${statusError} If the folder moved or was deleted, remove the repository from the Git page and add it again.`,"empty":true,"loading":statusLoading,"onretry":() => loadStatus(repo.id),}});}
    }else{
       { svelteHTML.createElement("div", { "style":`padding: 16px`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":5,"height":36,}});} }
    }
   }
 }

if(remotesOpen){
   { const $$_lenaPsetomeR0C = __sveltets_2_ensureComponent(RemotesPanel); new $$_lenaPsetomeR0C({ target: __sveltets_2_any(), props: {     "repoId":repo.id,"onclose":() => {
      remotesOpen = false;
      // The repo record's remote/forge are derived from origin — re-read them
      // so an added or changed remote lights up the PR view without a reload.
      void git.loadAllRepos(true);
    },}});}
}

if(mergeReq){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":MergeApprovalLazy,"what":`the merge dialog`,"quiet":true,"props":{
      repoId: repo.id,
      source: mergeReq.source,
      target: mergeReq.target,
      onclose: () => (mergeReq = null),
      onmerged: onMerged,
      onconflicts: onConflicts,
    },}});}
}



if(gitBridge.recovery?.repoId === repo.id){
  repo.id; {
     { const $$_slooTyrevoceR0C = __sveltets_2_ensureComponent(RecoveryTools); new $$_slooTyrevoceR0C({ target: __sveltets_2_any(), props: {          "repoId":repo.id,"initialMode":gitBridge.recovery.mode,"initialOnto":gitBridge.recovery.onto,"onclose":() => { gitBridge.recovery = null; gitBridge.refreshGraph(repo.id); },"onresolve":() => { gitBridge.recovery = null; resolving = true; },}});}
  }
}
};
return { props: {} as any as Props, exports: {} as any as { openMoreMenu: typeof openMoreMenu }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RepoView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RepoView__SvelteComponent_ = ReturnType<typeof RepoView__SvelteComponent_>;
/*Ωignore_endΩ*/export default RepoView__SvelteComponent_;