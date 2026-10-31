///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
// Git module page. Workspace-INDEPENDENT: shows GitKraken-style top-level repo
// tabs (one per open repo) above the active repo's RepoView. With no tab open
// it shows a full-width landing — the repo browser/list + "Add Repository"
// flow + EmptyState. Open repos persist across restarts (global localStorage,
// via the git store). PR detail (#/git/:id/pr/:n) is still routed; a plain
// #/git/:id/:tab deep-link opens that repo as a tab.
import { untrack } from 'svelte';
import { onTabKey } from '../../lib/tabKeys';
import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import type { GitAccount, GitProviderKind, Repo, RemoteRepoSummary } from '../../lib/api/types';
import { router } from '../../lib/router.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { git } from '../../lib/stores/git.svelte';
import { toasts } from '../../lib/toast.svelte';
import GitTabs from './GitTabs.svelte';
import { focusPath, parseGitRoute } from './gitRoute';
import RepoView from './RepoView.svelte';
import GitToolbar from './GitToolbar.svelte';
import { gitBridge } from './gitBridge.svelte';
import { registry } from '../../lib/commands.svelte';
import { graphCache } from './graph-cache';
import FocusView from './FocusView.svelte';
import LazyMount from '../../lib/components/LazyMount.svelte';
import { lazyComponent } from '../../lib/lazy-component.svelte';
import Modal from '../../lib/components/Modal.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // The PR detail (+ its review panel, comment threads and merge dialog) is a
  // deep-linked route most Git visits never open: its own chunk (perf R7).
  const PrDetailLazy = lazyComponent(() => import('./PrDetail.svelte'));

  // Route awareness is now limited to PR detail + deep-links into a repo tab.
  const routeRepoId = $derived(router.parts[1] ?? null);
  const isPr = $derived(router.parts[2] === 'pr' && router.parts[3] !== undefined);

  // The active repo is driven by the tab store, not the route.
  const activeRepo = $derived(git.allRepos.find((r) => r.id === git.activeRepoId) ?? null);
  const activeStatus = $derived(activeRepo ? (git.statusById[activeRepo.id] ?? null) : null);
  let repoView = $state<{ openMoreMenu: (e: MouseEvent) => void } | undefined>();
  const prRepo = $derived(git.allRepos.find((r) => r.id === routeRepoId) ?? null);

  // Focus (my PRs + my Jira work across every repo) is a PAGE-level view, not
  // a repo sub-tab, and it lives in the URL (gitRoute.ts): the Focus entry
  // pinned before the repo tabs goes to #/git/focus; a repo tab, the Git
  // crumb, a repo link inside Focus or Back leave it. The legacy
  // #/git/<repo>/focus opens that repo, then Focus.
  const route = $derived(parseGitRoute(router.parts));
  const focusView = $derived(route.focus);
  function setFocus(on: boolean): void {
    router.go(focusPath(on));
  }

  // add repo sheet
  let addOpen = $state(false);
  let addMode: 'register' | 'clone' | 'browse' = $state('register');

  // ⌘K: adding a repository (the header no longer carries its own button —
  // the + tab and the empty state do). The open repo's verbs come from
  // GitToolbar.
  $effect(() =>
    registry.register('git', [
      { id: 'git.add-local', title: 'Add a local repository…', group: 'Git', keywords: 'register folder repo', run: () => { addMode = 'register'; addOpen = true; } },
      { id: 'git.clone', title: 'Clone a repository…', group: 'Git', keywords: 'remote url github bitbucket gitlab', run: () => { addMode = 'clone'; addOpen = true; } },
    ]),
  );
  let addPath = $state('');
  let addUrl = $state('');
  let addName = $state('');
  let addAccount = $state('');
  let accounts: GitAccount[] = $state([]);
  let busy = $state(false);
  let pickerOpen = $state(false);


  // Clone destination dir — REMEMBERED across sessions so the user never has to
  // re-pick it (persisted to localStorage; defaults to the last-used location).
  const CLONE_DIR_KEY = 'otto_git_clone_dir';
  let cloneDir = $state(localStorage.getItem(CLONE_DIR_KEY) ?? '');
  let cloneDirPickerOpen = $state(false);
  function rememberCloneDir(): void {
    const v = cloneDir.trim();
    if (v) localStorage.setItem(CLONE_DIR_KEY, v);
  }

  // "Browse remote" mode: search repos under a git account's namespace.
  let browseAccount = $state('');
  let browseQuery = $state('');
  let remoteRepos: RemoteRepoSummary[] = $state([]);
  let remoteLoading = $state(false);
  let remoteError = $state('');
  let remoteRevision = 0;
  let remoteResultsAccount = '';
  $effect(() => {
    if (!addOpen) {
      remoteRevision++;
      if (searchTimer) clearTimeout(searchTimer);
      remoteRepos = [];
      remoteLoading = false;
    } else if (untrack(() => addMode === 'browse')) {
      untrack(() => void runRemoteSearch());
    }
    return () => {
      remoteRevision++;
      if (searchTimer) clearTimeout(searchTimer);
    };
  });
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  const accountsWithNs = $derived(accounts.filter((a) => a.namespace));

  /** Display name for the forge chip (the enum is lowercase: "bitbucket"). */
  const PROVIDER_LABEL: Record<string, string> = {
    github: 'GitHub',
    bitbucket: 'Bitbucket',
    gitlab: 'GitLab',
  };

  // Landing-hub repo filter: matches name, local path, or remote URL.
  let repoFilter = $state('');
  const filteredRepos = $derived.by(() => {
    const q = repoFilter.trim().toLowerCase();
    if (q === '') return git.allRepos;
    return git.allRepos.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.path.toLowerCase().includes(q) ||
        (r.remote_url ?? '').toLowerCase().includes(q),
    );
  });

  $effect(() => {
    // default the browse account to the first one that has a namespace
    if (browseAccount === '' && accountsWithNs.length > 0) browseAccount = accountsWithNs[0].id;
  });

  // ── Load the GLOBAL repo list + restore open tabs (once, on mount) ─────────
  // Workspace-independent: the page does NOT re-load when the active workspace
  // changes. `restoreOpenTabs` prunes ids that no longer exist. Guarded so it
  // runs a single time even if the effect re-fires.
  let restored = false;
  // A failed list load is `git.allReposError` (the store keeps the last known
  // list), so the landing shows Retry instead of "No repositories yet" — an
  // empty state one click from registering a duplicate.
  async function retryRepos(): Promise<void> {
    await git.loadAllRepos(true);
    if (git.allReposLoaded) await git.initializeOpenTabs();
  }
  $effect(() => {
    if (restored) return;
    restored = true;
    void (async () => {
      await git.initializeOpenTabs();
      // A deep-link into a repo (#/git/:id or #/git/:id/:tab, non-PR) opens that
      // repo as a tab so the route still lands somewhere useful.
      const r = route;
      if (r.repo && !r.pr && git.allRepos.some((x) => x.id === r.repo)) {
        git.openRepoTab(r.repo, r.tab ?? undefined);
        if (r.legacyFocus) router.replace(focusPath(true));
      }
    })();
  });

  // PrDetail's "back to PRs" routes to #/git/:id/prs; honour that as a tab open.
  // CRITICAL: `openRepoTab` WRITES subTab/activeRepoId/openRepoIds (fresh refs via
  // spread) AND reads them — and this effect tracks those reads — so calling it
  // bare here is a read→write→re-run INFINITE LOOP that froze the whole app on the
  // back-from-PR navigation. `untrack` the mutation so the effect only re-runs when
  // the route (its real deps, read above the untrack) changes.
  $effect(() => {
    const r = route;
    const known = r.repo != null && git.allRepos.some((x) => x.id === r.repo);
    if (r.repo && !r.pr && known && (r.tab || r.legacyFocus)) {
      untrack(() => {
        git.openRepoTab(r.repo!, r.tab ?? undefined);
        if (r.legacyFocus) router.replace(focusPath(true));
      });
    }
  });

  function openRepo(repoId: string): void {
    git.openRepoTab(repoId);
    // Drop any lingering deep-link route so the tab UI fully owns navigation.
    if (router.parts[0] === 'git' && router.parts.length > 1) router.go('git');
  }

  function scheduleRemoteSearch(): void {
    remoteRevision++;
    remoteRepos = [];
    remoteError = '';
    remoteLoading = true;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void runRemoteSearch(), 350);
  }

  async function runRemoteSearch(): Promise<void> {
    if (searchTimer) clearTimeout(searchTimer);
    const revision = ++remoteRevision;
    const accountId = browseAccount;
    remoteRepos = [];
    if (accountId === '') { remoteLoading = false; return; }
    remoteLoading = true;
    remoteError = '';
    try {
      const q = browseQuery.trim() === '' ? '' : `?q=${encodeURIComponent(browseQuery.trim())}`;
      const next = await api.get<RemoteRepoSummary[]>(`/git/accounts/${accountId}/remote-repos${q}`);
      if (revision !== remoteRevision) return;
      remoteRepos = next;
      remoteResultsAccount = accountId;
    } catch (e) {
      if (revision !== remoteRevision) return;
      remoteError = loadErrorText(e);
      remoteRepos = [];
    } finally {
      if (revision === remoteRevision) remoteLoading = false;
    }
  }

  async function cloneRemote(repo: RemoteRepoSummary): Promise<void> {
    if (!ws.currentId || busy || remoteLoading || remoteResultsAccount !== browseAccount) return;
    busy = true;
    try {
      const created = await api.post<Repo>(`/workspaces/${ws.currentId}/repos`, {
        clone_url: repo.clone_url,
        name: repo.name || null,
        git_account_id: remoteResultsAccount || null,
        clone_dir: cloneDir.trim() || null,
      });
      rememberCloneDir();
      toasts.success('Clone started', created.name);
      addOpen = false;
      await git.loadAllRepos(true);
    } catch (e) {
      toastError('Couldn’t clone', e);
    } finally {
      busy = false;
    }
  }

  // Accounts back both the add/clone flow and the per-card account picker on
  // the landing hub, so they load on mount — not only when the sheet opens.
  // Re-read on every `addOpen` toggle so an account added there shows up in the
  // cards without a reload.
  $effect(() => {
    void addOpen;
    void api
      .get<GitAccount[]>('/git/accounts')
      .then((a) => (accounts = a))
      .catch(() => (accounts = []));
  });

  /** Accounts that could serve a repo on `provider` (a token is provider-bound). */
  function accountsFor(provider: GitProviderKind | null): GitAccount[] {
    return provider === null ? [] : accounts.filter((a) => a.provider === provider);
  }

  /** (Re)bind a repo to a hosting account. Registration resolves the account
   *  once, so a repo added before its account existed needs this — otherwise
   *  every provider call fails with "repo has no git account". */
  async function setRepoAccount(r: Repo, accountId: string): Promise<void> {
    try {
      const updated = await api.patch<Repo>(`/repos/${r.id}`, {
        git_account_id: accountId === '' ? null : accountId,
      });
      await git.loadAllRepos(true);
      toasts.success(
        updated.git_account_id ? 'Account linked' : 'Account unlinked',
        r.name,
      );
    } catch (e) {
      toastError('Couldn’t link account', e);
      // Snap the <select> back to the stored binding — the change never landed.
      await git.loadAllRepos(true);
    }
  }

  async function addRepo(): Promise<void> {
    if (!ws.currentId) return;
    busy = true;
    try {
      const body =
        addMode === 'register'
          ? { path: addPath.trim(), name: addName.trim() || null }
          : {
              clone_url: addUrl.trim(),
              name: addName.trim() || null,
              git_account_id: addAccount === '' ? null : addAccount,
              clone_dir: cloneDir.trim() || null,
            };
      const repo = await api.post<Repo>(`/workspaces/${ws.currentId}/repos`, body);
      if (addMode === 'clone') rememberCloneDir();
      toasts.success(addMode === 'clone' ? 'Clone started' : 'Repo registered', repo.name);
      addOpen = false;
      addPath = addUrl = addName = '';
      await git.loadAllRepos(true);
      // Newly registered (not async-cloning) repos open straight into a tab.
      if (addMode === 'register') openRepo(repo.id);
    } catch (e) {
      toastError('Couldn’t add it', e);
    } finally {
      busy = false;
    }
  }

  async function removeRepo(r: Repo): Promise<void> {
    if (!(await confirmer.ask(`Remove “${r.name}” from Otto? Its tab closes and its saved review findings are deleted. The folder and its git history on disk are not touched, so you can add it again later.`, { title: 'Remove repository', confirmLabel: 'Remove', danger: true }))) return;
    try {
      await api.del(`/repos/${r.id}`);
      git.closeRepoTab(r.id);
      graphCache.drop(r.id);
      await git.loadAllRepos(true);
      toasts.info('Repository removed', r.name);
    } catch (e) {
      toastError('Couldn’t remove the repository', e);
    }
  }
;
async () => {

if(prRepo && isPr){
  
  `${prRepo.id}:${router.parts[3]}`; {
     { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {      "lazy":PrDetailLazy,"what":`the pull request`,"props":{ repoId: prRepo.id, number: Number(router.parts[3]) },}});}
  }
}else{
   { svelteHTML.createElement("div", { "class":`gitpage`,});
    
     { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {       "title":focusView ? 'Focus' : 'Git',"crumbs":focusView ? [{ label: 'Git', onclick: () => setFocus(false) }] : [],"class":`git-header`,tabs:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_sbaTtiG0C = __sveltets_2_ensureComponent(GitTabs); new $$_sbaTtiG0C({ target: __sveltets_2_any(), props: {        "embedded":true,"focus":focusView,"ontogglefocus":() => setFocus(!focusView),"onopen":openRepo,"onadd":(mode) => { addMode = mode; addOpen = true; },}});}
      };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
        if(activeRepo && !focusView){
          const repoId = activeRepo.id;
          
          if(activeStatus){
             { const $$_rablooTtiG0C = __sveltets_2_ensureComponent(GitToolbar); new $$_rablooTtiG0C({ target: __sveltets_2_any(), props: {         "inHeader":true,repoId,"status":activeStatus,"onstatus":(s) => git.setStatus(repoId, s),"onrefresh":() => gitBridge.refreshGraph(repoId),}});}
          }
           { svelteHTML.createElement("button", {                "class":`icon-btn`,...__sveltets_2_empty({"data-keep":true}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More repository actions`}),"title":`More repository actions`,"aria-label":`More repository actions`,"aria-haspopup":`menu`,"onclick":(e) => repoView?.openMoreMenu(e),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":16,}});} }
        }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
      
      
     PageHeader}

    if(focusView){
       { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
         { svelteHTML.createElement("div", { "class":`landing-focus`,});
           { const $$_weiVsucoF3C = __sveltets_2_ensureComponent(FocusView); new $$_weiVsucoF3C({ target: __sveltets_2_any(), props: {}});}
         }
       PageBody}
    } else if (activeRepo){
      activeRepo.id; {
         { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
         { svelteHTML.createElement("div", { "class":`gitpage-body`,});
           { const $$_weiVopeR3C = __sveltets_2_ensureComponent(RepoView); const $$_weiVopeR3 = new $$_weiVopeR3C({ target: __sveltets_2_any(), props: {              "repo":activeRepo,"tab":git.subTabFor(activeRepo.id),"embedded":true,"hostActions":true,"onTab":(t) => git.setSubTab(activeRepo.id, t),"onopenrepo":openRepo,"onaddrepo":(mode) => { addMode = mode; addOpen = true; },}});repoView = $$_weiVopeR3;}
         }
         PageBody}
      }
    }else{
      
       { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"what":`repositories`,"variant":`page`,"loading":!git.allReposLoaded && (git.loading || !git.allReposError),"error":git.allReposError,"empty":git.allRepos.length === 0,"rows":3,"onretry":() => void retryRepos(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
             { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`branch`,"title":`No repositories yet`,"body":`Register an existing local repo or clone one from GitHub, Bitbucket or GitLab.`,"actionLabel":`Add repository`,"actionIcon":`plus`,"onaction":() => (addOpen = true),}});}
          };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
          
           { svelteHTML.createElement("div", { "class":`landing-inner`,});
             { svelteHTML.createElement("div", { "class":`landing-hint`,});
               { svelteHTML.createElement("span", { "class":`section-title landing-count`,});  git.allRepos.length; }
               { svelteHTML.createElement("span", { "class":`dim`,});          }
             }
             { svelteHTML.createElement("div", { "class":`repo-search`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
               { svelteHTML.createElement("input", {                "dir":`ltr`,"class":`repo-search-input`,"type":`search`,"bind:value":repoFilter,"placeholder":`Filter repositories by name, path or remote…`,"aria-label":`Filter repositories`,"spellcheck":`false`,"onkeydown":(e) => {
                  if (e.key === 'Escape' && repoFilter !== '') {
                    e.stopPropagation();
                    repoFilter = '';
                  }
                },});/*Ωignore_startΩ*/() => repoFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
              if(repoFilter.trim() !== ''){
                 { svelteHTML.createElement("span", { "class":`repo-search-count dim`,});
                  filteredRepos.length;  git.allRepos.length;
                 }
              }
             }
            if(filteredRepos.length === 0){
               { svelteHTML.createElement("div", { "class":`repo-search-empty dim`,});
                   repoFilter.trim();
               }
            }
             { svelteHTML.createElement("div", { "class":`repo-grid`,});
                 for(let r of __sveltets_2_ensureArray(filteredRepos)){r.id;
               { svelteHTML.createElement("div", { "class":`repo-card card`,});
                 { svelteHTML.createElement("button", {   "class":`repo-main`,"onclick":() => openRepo(r.id),});
                   { svelteHTML.createElement("div", { "class":`repo-name`,});
                     { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":14,}});}
                     { svelteHTML.createElement("span", {   "class":`repo-name-text`,"title":r.name,});r.name; }
                    if(r.provider){ { svelteHTML.createElement("span", { "class":`chip repo-provider`,});PROVIDER_LABEL[r.provider] ?? r.provider; }}
                   }
                   { svelteHTML.createElement("div", {   "class":`repo-path mono`,"title":r.path,});r.path; }
                  if(r.remote_url){ { svelteHTML.createElement("div", {   "class":`repo-remote mono dim`,"title":r.remote_url,});r.remote_url; }}
                 }
                 { svelteHTML.createElement("div", { "class":`repo-actions`,});
                   { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => git.openRepoTab(r.id, 'prs'),"title":`Open ${r.name} on its pull requests`,});
                     { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`pr`,"size":12,}});}  
                   }
                  if(r.provider){
                    
                     { svelteHTML.createElement("select", {            "class":`repo-account`,"title":r.git_account_id
                        ? 'Git account used for pull requests'
                        : `No ${PROVIDER_LABEL[r.provider] ?? r.provider} account linked — pull requests will fail`,"aria-label":`Git account for ${r.name}`,"value":r.git_account_id ?? '',"onchange":(e) => setRepoAccount(r, e.currentTarget.value),});!r.git_account_id;
                       { svelteHTML.createElement("option", {"value":"",});  }
                         for(let a of __sveltets_2_ensureArray(accountsFor(r.provider))){a.id;
                         { svelteHTML.createElement("option", { "value":a.id,});a.label; }
                      }
                     }
                  }
                   { svelteHTML.createElement("span", { "class":`grow`,}); }
                   { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Remove ${r.name} from Otto`,"aria-label":`Remove ${r.name} from Otto`,"onclick":() => removeRepo(r),});
                     { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
                   }
                 }
               }
            }
             }
           }
         LoadState}
       PageBody}
    }
   }
}

if(addOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"title":`Add repository`,"onclose":() => (addOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (addOpen = false),});  }
      if(addMode !== 'browse'){
         { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || (addMode === 'register' ? addPath.trim() === '' : addUrl.trim() === ''),"onclick":addRepo,});
          busy ? (addMode === 'clone' ? 'Cloning…' : 'Adding…') : addMode === 'clone' ? 'Clone' : 'Add repository';
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", {     "class":`segmented add-mode`,"role":`tablist`,"aria-label":`How to add the repository`,});
       { svelteHTML.createElement("button", {          "role":`tab`,"onkeydown":onTabKey,"tabindex":addMode === 'register' ? 0 : -1,"aria-selected":addMode === 'register',"onclick":() => (addMode = 'register'),});addMode === 'register';
         
       }
       { svelteHTML.createElement("button", {          "role":`tab`,"onkeydown":onTabKey,"tabindex":addMode === 'browse' ? 0 : -1,"aria-selected":addMode === 'browse',"onclick":() => { addMode = 'browse'; if (remoteRepos.length === 0) void runRemoteSearch(); },});addMode === 'browse';
         
       }
       { svelteHTML.createElement("button", {          "role":`tab`,"onkeydown":onTabKey,"tabindex":addMode === 'clone' ? 0 : -1,"aria-selected":addMode === 'clone',"onclick":() => (addMode = 'clone'),});addMode === 'clone';  }
     }

    if(addMode === 'register'){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ar-path`,});  }
         { svelteHTML.createElement("div", { "class":`path-row`,});
           { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`ar-path`,"class":`input mono`,"bind:value":addPath,"placeholder":`/Users/you/code/repo`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => addPath = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {     "class":`btn`,"type":`button`,"onclick":() => (pickerOpen = true),});  }
         }
       }
    } else if (addMode === 'browse'){
      if(accountsWithNs.length === 0){
         { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "icon":`key`,"title":`No namespace configured`,"body":`Add a Git account with an organization, workspace or group to browse its repositories.`,"actionLabel":`Open Git accounts`,"actionIcon":`key`,"onaction":() => { addOpen = false; router.go('settings/git-accounts'); },}});}
      }else{
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`ar-bacct`,});  }
           { svelteHTML.createElement("select", {       "id":`ar-bacct`,"class":`input`,"bind:value":browseAccount,"onchange":() => { remoteRepos = []; void runRemoteSearch(); },});/*Ωignore_startΩ*/() => browseAccount = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let a of __sveltets_2_ensureArray(accountsWithNs)){a.id;
               { svelteHTML.createElement("option", { "value":a.id,});a.label;  a.namespace; a.provider;  }
            }
           }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`ar-bq`,});  }
           { svelteHTML.createElement("input", {              "dir":`auto`,"id":`ar-bq`,"class":`input`,"bind:value":browseQuery,"oninput":scheduleRemoteSearch,"placeholder":`api-gateway`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => browseQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`remote-list`,});
          if(remoteLoading){
             { svelteHTML.createElement("div", { "class":`dim pad`,});  }
          } else if (remoteError){
             { svelteHTML.createElement("div", {   "class":`err pad`,"role":`alert`,});
               { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
               { svelteHTML.createElement("span", { "class":`grow`,});      { svelteHTML.createElement("span", { "class":`dim`,});remoteError; } }
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void runRemoteSearch(),});  }
             }
          } else if (remoteRepos.length === 0){
             { svelteHTML.createElement("div", { "class":`dim pad`,});browseQuery.trim() ? `No repositories match “${browseQuery.trim()}”.` : 'This account has no repositories.'; }
          }else{
               for(let r of __sveltets_2_ensureArray(remoteRepos)){r.full_name;
               { svelteHTML.createElement("div", { "class":`remote-row`,});
                 { svelteHTML.createElement("div", { "class":`grow min0`,});
                   { svelteHTML.createElement("div", { "class":`remote-name`,});
                    r.name;
                    if(r.private){ { svelteHTML.createElement("span", { "class":`chip`,});  }}
                   }
                  if(r.description){ { svelteHTML.createElement("div", { "class":`remote-desc dim ellipsis`,});r.description; }}
                   { svelteHTML.createElement("div", { "class":`remote-full mono dim ellipsis`,});r.full_name; }
                 }
                 { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":busy,"onclick":() => cloneRemote(r),"title":`Clone ${r.full_name}`,});  }
               }
            }
          }
         }
      }
    }else{
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ar-url`,});  }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`ar-url`,"class":`input mono`,"bind:value":addUrl,"placeholder":`git@github.com:org/repo.git`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => addUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ar-acct`,});   { svelteHTML.createElement("span", { "class":`dim`,});   } }
         { svelteHTML.createElement("select", {     "id":`ar-acct`,"class":`input`,"bind:value":addAccount,});/*Ωignore_startΩ*/() => addAccount = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});     }
             for(let a of __sveltets_2_ensureArray(accounts)){a.id;
             { svelteHTML.createElement("option", { "value":a.id,});a.label; a.provider;  }
          }
         }
       }
    }

    if(addMode !== 'register'){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ar-clonedir`,});  }
         { svelteHTML.createElement("div", { "class":`path-row`,});
           { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`ar-clonedir`,"class":`input mono`,"bind:value":cloneDir,"placeholder":`~/code`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => cloneDir = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {     "class":`btn`,"type":`button`,"onclick":() => (cloneDirPickerOpen = true),});  }
         }
         { svelteHTML.createElement("span", { "class":`hint`,});            }
       }
    }

    if(addMode !== 'browse'){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ar-name`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
         { svelteHTML.createElement("input", {        "dir":`auto`,"id":`ar-name`,"class":`input`,"bind:value":addName,});/*Ωignore_startΩ*/() => addName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
    }

    
   Modal}
}

if(pickerOpen){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {          "title":`Choose a git repository`,"start":addPath,"gitOnly":true,"onpick":(p) => {
      addPath = p;
      if (addName.trim() === '') addName = p.split('/').filter(Boolean).pop() ?? '';
      pickerOpen = false;
    },"onclose":() => (pickerOpen = false),}});}
}

if(cloneDirPickerOpen){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Choose a clone destination`,"start":cloneDir,"onpick":(p) => {
      cloneDir = p;
      rememberCloneDir();
      cloneDirPickerOpen = false;
    },"onclose":() => (cloneDirPickerOpen = false),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GitPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GitPage__SvelteComponent_ = ReturnType<typeof GitPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default GitPage__SvelteComponent_;