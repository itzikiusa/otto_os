///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
// Toolbar row: Fetch / Pull / Push / Branch / Stash / Pop + current branch chip.
// In the page header Fetch folds into the Pull split menu (≤5 header
// controls) and the verbs are also ⌘K commands (group "Git").
import { git } from '../../lib/stores/git.svelte';
import { api } from '../../lib/api/client';
import type { PullMode, PullModeResp, RepoStatusResp } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { runPull } from './pullFlow';
import { runPush } from './pushFlow';
import { gitBridge } from './gitBridge.svelte';
import { registry } from '../../lib/commands.svelte';
;

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Called after an op that may have MOVED refs/history (fetch/pull/push/
     *  branch) so the parent can re-mount the graph — otherwise the commit graph
     *  shows stale data until the repo tab is reopened. */
    onrefresh?: () => void;
    /** Rendered as direct children of the PageHeader's actions row (so they
     *  overflow into its ⋯ by `data-overflow`) instead of in a `.toolbar` wrapper. */
    inHeader?: boolean;
  };function $$render() {
  const items/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {

  { svelteHTML.createElement("span", {    "class":`branch-chip`,"title":`${status.branch}${status.upstream ? ` · tracks ${status.upstream}` : ''}${status.ahead > 0 ? ` · ${status.ahead} ahead` : ''}${status.behind > 0 ? ` · ${status.behind} behind` : ''}`,});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});}
   { svelteHTML.createElement("span", { "class":`mono branch-name`,});status.branch; }
  
  if(status.ahead > 0){ { svelteHTML.createElement("span", { "class":`ab up`,}); { svelteHTML.createElement("span", { "aria-hidden":`true`,}); status.ahead; } { svelteHTML.createElement("span", { "class":`sr-only`,});status.ahead;  } }}
  if(status.behind > 0){ { svelteHTML.createElement("span", { "class":`ab down`,}); { svelteHTML.createElement("span", { "aria-hidden":`true`,}); status.behind; } { svelteHTML.createElement("span", { "class":`sr-only`,});status.behind;  } }}
 }

if(!inHeader){ { svelteHTML.createElement("span", { "class":`divider`,}); }}


if(!inHeader){
 { svelteHTML.createElement("button", {             "class":`btn small ghost tbtn`,...__sveltets_2_empty({"data-overflow":`3`}),...__sveltets_2_empty({"data-icon":`fetch`}),...__sveltets_2_empty({"data-label":`Fetch`}),"disabled":busy !== '',"onclick":doFetch,"title":`Fetch from remote`,});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`fetch`,"size":12,}});}
  busy === 'fetch' ? 'Fetching…' : 'Fetch';
 }
}


 { svelteHTML.createElement("span", { "class":`split`,});
   { svelteHTML.createElement("button", {             "class":`btn small ghost tbtn`,...__sveltets_2_empty({"data-icon":`arrowDown`}),...__sveltets_2_empty({"data-label":pullLabel}),"disabled":busy !== '' || detached,"onclick":() => void doPull(),"title":detached
      ? 'HEAD is detached — check out a branch to pull'
      : "Pull from upstream using the repo’s configured mode",});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}
    busy === 'pull' ? 'Pulling…' : pullLabel;
   }
   { svelteHTML.createElement("button", {                 "class":`btn small ghost tbtn caret`,...__sveltets_2_empty({"data-icon":`chevronDown`}),...__sveltets_2_empty({"data-label":inHeader ? 'Fetch and pull options…' : 'Pull options…'}),"disabled":busy !== '' || (detached && !inHeader),"aria-haspopup":`menu`,"onclick":pullMenu,"title":inHeader ? 'Fetch, or pull with a different mode' : 'Pull with a different mode',"aria-label":inHeader ? 'Fetch and pull options' : 'Pull options',}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
 }


  { svelteHTML.createElement("button", {                "class":`btn small tbtn`,...__sveltets_2_empty({"data-overflow":`4`}),...__sveltets_2_empty({"data-icon":`arrowUp`}),...__sveltets_2_empty({"data-label":status.upstream ? 'Push' : 'Publish'}),"disabled":busy !== '' || nothingToPush || detached,"onclick":() => void doPush(),"title":detached
    ? 'HEAD is detached — check out a branch to push'
    : status.upstream
    ? status.ahead > 0
      ? `Push ${plural(status.ahead, 'commit')} to ${status.upstream}`
      : `Nothing to push — ${status.branch} has no commits that ${status.upstream} doesn’t`
    : `Publish ${status.branch} to origin`,});!pushPrimary;pushPrimary;
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}
  busy === 'push' ? 'Pushing…' : status.upstream ? 'Push' : 'Publish';
 }

if(inHeader){
  
   { svelteHTML.createElement("button", {                  "class":`btn small ghost tbtn`,...__sveltets_2_empty({"data-keep":true}),...__sveltets_2_empty({"data-icon":`branch`}),...__sveltets_2_empty({"data-label":`Branch & stash`}),"disabled":busy !== '',"aria-haspopup":`menu`,"aria-label":`Branch and stash`,"title":`New branch, stash, pop stash`,"onclick":branchMenu,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`stash`,"size":12,}});}  { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
}else{
 { svelteHTML.createElement("span", { "class":`divider`,}); }


 { svelteHTML.createElement("button", {             "class":`btn small ghost tbtn`,...__sveltets_2_empty({"data-overflow":`-2`}),...__sveltets_2_empty({"data-icon":`plus`}),...__sveltets_2_empty({"data-label":`New branch`}),"disabled":busy !== '',"onclick":() => void doCreateBranch(),"title":`Create a new branch from ${status.branch} and switch to it`,});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
  busy === 'branch' ? 'Creating…' : 'Branch';
 }

 { svelteHTML.createElement("span", { "class":`divider`,}); }


  { svelteHTML.createElement("button", {              "class":`btn small ghost tbtn`,...__sveltets_2_empty({"data-overflow":`-3`}),...__sveltets_2_empty({"data-icon":`stash`}),...__sveltets_2_empty({"data-label":`Stash`}),"disabled":busy !== '' || status.changes.length === 0,"onclick":doStash,"title":status.changes.length === 0 ? 'Nothing to stash — the working tree is clean' : 'Stash working changes (including untracked files)',});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`stash`,"size":12,}});}
  busy === 'stash' ? 'Stashing…' : 'Stash';
 }


  { svelteHTML.createElement("button", {              "class":`btn small ghost tbtn`,...__sveltets_2_empty({"data-overflow":`-4`}),...__sveltets_2_empty({"data-icon":`archive`}),...__sveltets_2_empty({"data-label":`Pop stash`}),"disabled":busy !== '' || nothingToPop,"onclick":doPop,"title":nothingToPop ? 'Nothing to pop — there are no stashes' : 'Apply the latest stash and drop it',});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":12,}});}
  busy === 'pop' ? 'Popping…' : 'Pop';
 }
}
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, status, onstatus, onrefresh, inHeader = false }: Props = $props();

  let busy = $state('');

  // Nothing to push: the branch tracks an upstream and has no local commits on
  // top of it. (No upstream → the button is Publish, always available.)
  const nothingToPush = $derived(status.upstream != null && status.ahead === 0);
  // `git status` reports a detached HEAD as the branch "(detached)": there is
  // no branch to push or pull, so both would only fail with git's
  // "You are not currently on a branch".
  const detached = $derived(status.branch === '(detached)');
  // Push is the header's primary action only while there is something to push.
  const pushPrimary = $derived(status.upstream != null && status.ahead > 0 && !detached);
  // Known-empty stash list → Pop can only fail, so say so up front. Unknown
  // (the graph hasn't reported yet) keeps it enabled.
  const stashCount = $derived(gitBridge.stashCount[repoId]);
  const nothingToPop = $derived(stashCount === 0);

  async function doFetch(): Promise<void> {
    busy = 'fetch';
    try {
      const s = await git.fetchRepo(repoId);
      onstatus(s); // Store refsRev quietly refreshes the graph without a remount.
      // Say what the fetch found, not which remote we assume it hit.
      toasts.success(
        'Fetched',
        s.behind > 0
          ? `${plural(s.behind, 'new commit')} on ${s.upstream ?? 'the upstream'} — pull to bring them in`
          : s.upstream
            ? `${s.branch} is up to date with ${s.upstream}`
            : 'Remote branches and tags refreshed',
      );
    } catch (e) {
      toastError('Couldn’t fetch', e);
    } finally {
      busy = '';
    }
  }

  // The repo's EFFECTIVE pull mode (its `pull.rebase`/`pull.ff` config), loaded
  // once per repo so the button says what it will actually do. Until it is
  // known (or when the read fails) the button says plain "Pull": guessing
  // "merge" mislabelled a repo configured to rebase.
  let pullMode = $state<PullMode | null>(null);
  let pullModeFor = '';
  $effect(() => {
    const id = repoId;
    if (pullModeFor === id) return;
    pullModeFor = id;
    pullMode = null;
    void api
      .get<PullModeResp>(`/repos/${id}/pull-mode`)
      .then((r) => {
        if (pullModeFor === id) pullMode = r.mode;
      })
      .catch(() => {
        // Not sticky: the next time this repo is shown, read it again.
        if (pullModeFor === id) pullModeFor = '';
      });
  });

  const MODE_LABEL: Record<PullMode, string> = {
    merge: 'merge',
    rebase: 'rebase',
    ff_only: 'ff-only',
  };
  const pullLabel = $derived(pullMode ? `Pull (${MODE_LABEL[pullMode]})` : 'Pull');

  async function doPull(mode?: PullMode): Promise<void> {
    busy = 'pull';
    try {
      await runPull(repoId, onstatus, { mode });
      onrefresh?.();
    } finally {
      busy = '';
    }
  }

  function pullMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      ...(inHeader
        ? [
            { label: busy === 'fetch' ? 'Fetching…' : 'Fetch', icon: 'fetch' as const, title: 'Fetch from remote', action: () => void doFetch() },
            { separator: true as const },
          ]
        : []),
      { label: 'Pull (merge)', icon: 'arrowDown', disabled: detached, action: () => void doPull('merge') },
      { label: 'Pull (rebase)', icon: 'arrowDown', disabled: detached, action: () => void doPull('rebase') },
      { label: 'Pull (fast-forward only)', icon: 'arrowDown', disabled: detached, action: () => void doPull('ff_only') },
    ]);
  }

  // ⌘K: the open repo's everyday verbs (the header copy only — the side-panel
  // toolbar would register a second, identical set).
  $effect(() => {
    if (!inHeader) return;
    const branch = status.branch;
    const upstream = status.upstream;
    const isDetached = detached;
    const canPush = !nothingToPush && !isDetached;
    return registry.register('git-repo', [
      { id: 'git.fetch', title: 'Fetch', group: 'Git', keywords: 'remote refresh update', run: () => void doFetch() },
      ...(isDetached
        ? []
        : [{ id: 'git.pull', title: pullLabel, group: 'Git', keywords: 'upstream merge rebase', run: () => void doPull() }]),
      ...(canPush
        ? [{ id: 'git.push', title: upstream ? 'Push' : 'Publish branch', group: 'Git', keywords: `upload ${branch}`, run: () => void doPush() }]
        : []),
      { id: 'git.branch', title: 'New branch…', group: 'Git', keywords: `create checkout from ${branch}`, run: () => void doCreateBranch() },
    ]);
  });

  async function doPush(): Promise<void> {
    // No confirm: push is the most routine git action (the user asked for
    // fewer nag dialogs, and no git client asks before a plain push). The
    // button label already says Push vs Publish, and the toast reports it.
    // A REJECTED push asks Pull vs Force-with-lease (runPush) — force is
    // never sent without that explicit pick.
    busy = 'push';
    try {
      await runPush(repoId, { branch: status.branch, upstream: status.upstream }, onstatus);
      onrefresh?.();
    } finally {
      busy = '';
    }
  }

  // Same sheet the graph's "Create branch here…" uses. (This used to be an
  // absolutely-positioned inline popover, which the ≤1024px toolbar's
  // horizontal scroller clipped — the input was unreachable on a tablet.)
  async function doCreateBranch(): Promise<void> {
    const raw = await confirmer.promptText(`New branch from ${status.branch}`, {
      title: 'Create branch',
      confirmLabel: 'Create',
      placeholder: 'feature/my-branch',
    });
    const name = raw?.trim() ?? '';
    if (!name) return;
    busy = 'branch';
    try {
      const s = await git.checkout(repoId, { branch: name, create: true });
      onstatus(s);
      onrefresh?.();
      toasts.success('Branch created', name);
    } catch (e) {
      toastError('Couldn’t create the branch', e);
    } finally {
      busy = '';
    }
  }

  function branchMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'New branch…', icon: 'plus', title: `Create a new branch from ${status.branch} and switch to it`, action: () => void doCreateBranch() },
      { separator: true },
      {
        label: 'Stash changes',
        icon: 'stash',
        disabled: status.changes.length === 0,
        title: status.changes.length === 0 ? 'Nothing to stash — the working tree is clean' : 'Stash working changes (including untracked files)',
        action: () => void doStash(),
      },
      {
        label: 'Pop stash',
        icon: 'archive',
        disabled: nothingToPop,
        title: nothingToPop ? 'Nothing to pop — there are no stashes' : 'Apply the latest stash and drop it',
        action: () => void doPop(),
      },
    ]);
  }

  async function doStash(): Promise<void> {
    busy = 'stash';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/stash`, { op: 'save' });
      onstatus(s);
      // Re-mount the graph: its STASHES section + stash node only reload then.
      onrefresh?.();
      toasts.success('Stashed');
    } catch (e) {
      toastError('Couldn’t stash', e);
    } finally {
      busy = '';
    }
  }

  async function doPop(): Promise<void> {
    busy = 'pop';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/stash`, { op: 'pop' });
      onstatus(s);
      onrefresh?.();
      // A conflicting pop is a 200 with unmerged paths (git keeps the stash
      // entry) — guide the user into the resolver instead of claiming success.
      const conflicts = s.changes.filter((c) => c.kind === 'conflicted').length;
      if (conflicts > 0) {
        toasts.warn(
          'Stash popped with conflicts',
          `${plural(conflicts, 'file')} need resolution — open "Resolve conflicts". The stash entry was kept.`,
        );
      } else {
        toasts.success('Stash popped');
      }
    } catch (e) {
      toastError('Couldn’t pop', e);
    } finally {
      busy = '';
    }
  }
;
async () => {



if(inHeader){
  ;__sveltets_2_ensureSnippet(items());
}else{
   { svelteHTML.createElement("div", { "class":`toolbar`,});
    ;__sveltets_2_ensureSnippet(items());
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GitToolbar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GitToolbar__SvelteComponent_ = ReturnType<typeof GitToolbar__SvelteComponent_>;
/*Ωignore_endΩ*/export default GitToolbar__SvelteComponent_;