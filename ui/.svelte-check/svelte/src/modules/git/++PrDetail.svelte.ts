///<reference types="svelte" />
;
import { scrollBehavior } from '../../lib/motion';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
// PR detail: meta, editable markdown description, diff with inline comment
// threads, general comments, approve/merge/decline, "open as session".
// Four tabs in the header's tab row: Summary | Files | Commits | Review (AI agents).
import { onDestroy, untrack } from 'svelte';
import { onTabKey } from '../../lib/tabKeys';
import { api, isAbortError } from '../../lib/api/client';
import type { DiffResp, NewPrCommentReq, PrComment, PrCommit, PrDetail } from '../../lib/api/types';
import { guardUnsaved } from '../../lib/leaveGuard';
import { router } from '../../lib/router.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { git } from '../../lib/stores/git.svelte';
import { confirmOutward } from '../../lib/confirmOutward';
import { focusFirstField, refocusEditTrigger } from '../../lib/inlineEdit';
import { renderMarkdown } from '../../lib/md';
import { openExternal } from '../../lib/external';
import DiffViewer from './DiffViewer.svelte';
import { notePrHead, prDiffFileLoader, prDiffHead, prDiffSummary } from './diff-load';
import CommentThread from './CommentThread.svelte';
import ReviewPanel from './ReviewPanel.svelte';
import PrMergeModal from './PrMergeModal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import { rel } from '../../lib/stores/now.svelte';
import { invalidatePr, prCommitsCache, prDetailCache, prKey } from './pr-cache';
;

  interface Props {
    repoId: string;
    number: number;
  };;

  type Tab = 'summary' | 'files' | 'commits' | 'review';;function $$render() {
  const prTabs/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", {         "class":`segmented prd-tabs`,"role":`tablist`,"aria-label":`Pull request views`,"tabindex":-1,"onkeydown":onTabKey,});
       for(let t of __sveltets_2_ensureArray(PR_TABS)){t.id;
       { svelteHTML.createElement("button", {            "class":`tab-btn`,"role":`tab`,"aria-selected":activeTab === t.id,"tabindex":activeTab === t.id ? 0 : -1,"onclick":() => selectTab(t.id),});activeTab === t.id;
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":t.icon,"size":12,}});} t.label;
       }
    }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, number }: Props = $props();
  let disposed = false;
  onDestroy(() => { disposed = true; });

  const TABS: Tab[] = ['summary', 'files', 'commits', 'review'];
  const PR_TABS: { id: Tab; label: string; icon: IconName }[] = [
    { id: 'summary', label: 'Summary', icon: 'comment' },
    { id: 'files', label: 'Files', icon: 'file' },
    { id: 'commits', label: 'Commits', icon: 'commit' },
    { id: 'review', label: 'Review', icon: 'zap' },
  ];
  let activeTab: Tab = $state('summary');

  // Remember the last-used tab per PR so returning to a PR with a running
  // review lands back on Review (not Summary). Restore on PR change.
  $effect(() => {
    const saved = localStorage.getItem(`otto_pr_tab_${repoId}_${number}`);
    activeTab = saved && TABS.includes(saved as Tab) ? (saved as Tab) : 'summary';
  });

  function selectTab(t: Tab): void {
    activeTab = t;
    localStorage.setItem(`otto_pr_tab_${repoId}_${number}`, t);
  }

  // Server payloads are replaced wholesale, never mutated: `$state.raw` keeps
  // Svelte from proxying every hunk/line (a deep proxy over a 100k-line diff
  // cost ~100 ms per walk vs ~2 ms raw).
  let pr: PrDetail | null = $state.raw(null);
  let diff: DiffResp | null = $state.raw(null);
  let commits: PrCommit[] | null = $state.raw(null);
  // The Files tab paints the summary (file list + counts) and DiffViewer
  // pulls each file's hunks on demand through the shared PR diff cache (the
  // Review tab's snippets reuse the same fetches).
  const loadPrFile = $derived(prDiffFileLoader(repoId, number));
  let diffAbort: AbortController | null = null;
  onDestroy(() => diffAbort?.abort());
  let loading = $state(true);
  let diffLoading = $state(false);
  let commitsLoading = $state(false);
  let editMode = $state(false);
  let editTitle = $state('');
  let editDesc = $state('');
  let busy = $state('');
  let newComment = $state('');
  // Merge runs through PrMergeModal — it owns the strategy, the readiness
  // rows and the delete-source-branch choice.
  let mergeOpen = $state(false);
  let showRequestChanges = $state(false);
  let requestChangesBody = $state('');
  $effect(() => guardUnsaved(
    () => (editMode && !!pr && (editTitle !== pr.title || editDesc !== pr.description_md))
      || newComment.trim() !== '' || requestChangesBody.trim() !== '',
    { what: `pull request #${number}` },
  ));
  // Provider to spawn the "open as session" review agent on (registry-sourced,
  // defaults to the configured default agent — never a hardcoded 'claude').
  let reviewProvider = $state(defaultAgentProvider());

  const inlineComments = $derived.by(() => (pr?.comments ?? []).filter((c) => c.path !== null));
  const generalComments = $derived.by(() => (pr?.comments ?? []).filter((c) => c.path === null));

  // A different PR (back/forward, a notification deep link) reuses this
  // component: drop the previous PR's data first, so a slow or failed load
  // never shows PR A's title/diff (or offers its Approve…) under PR B's number.
  $effect(() => {
    const rid = repoId;
    const num = number;
    untrack(() => {
      diffAbort?.abort();
      diffAbort = null;
      diffLoading = false;
      pr = null;
      diff = null;
      diffHead = '';
      commits = null;
      prError = null;
      diffError = null;
      commitsError = null;
      // Stale-while-revalidate: paint the last-seen PR at once and refresh it
      // quietly (the daemon's ETag cache makes that a cheap 304). Commits are
      // reused only while fresh — they have no other revalidation path.
      const hit = prDetailCache.get(prKey(rid, num));
      if (hit) pr = hit.value;
      const ch = prCommitsCache.get(prKey(rid, num));
      if (ch?.fresh) commits = ch.value;
      void load(rid, num, !!hit);
    });
  });

  // Load failures render INLINE with Retry (a toast is for failed actions).
  let prError = $state<string | null>(null);

  // Lazy-load diff when switching to Files tab. `diffError` stops the effect
  // from re-firing after an error — diff stays null on failure, so without the
  // latch this loop hammered the daemon with retries forever.
  let diffError = $state<string | null>(null);
  $effect(() => {
    if (activeTab === 'files' && diff === null && !diffLoading && !diffError) {
      void loadDiff(repoId, number);
    }
  });

  // Lazy-load commits when switching to Commits tab (same failure latch; the
  // latch used to leave the tab on "Loading commits…" forever).
  let commitsError = $state<string | null>(null);
  $effect(() => {
    if (activeTab === 'commits' && commits === null && !commitsLoading && !commitsError) {
      void loadCommits(repoId, number);
    }
  });

  async function load(rid: string, num: number, quiet = false): Promise<void> {
    if (!quiet) loading = true;
    try {
      const next = await api.get<PrDetail>(`/repos/${rid}/prs/${num}`);
      prDetailCache.set(prKey(rid, num), next);
      if (disposed || rid !== repoId || num !== number) return; // switched PRs mid-flight
      pr = next;
      prError = null;
      // A push moved the head: the loaded diff is the pre-push one. Re-read it
      // under the new head (quietly — the old diff stays up until it lands).
      const head = next.head_sha ?? '';
      if (head && diff && diffHead && diffHead !== head) void loadDiff(rid, num);
      else {
        notePrHead(rid, num, head);
        if (head && diff && !diffHead) diffHead = head;
      }
    } catch (e) {
      if (!disposed && rid === repoId && num === number) prError = loadErrorText(e);
    } finally {
      if (!disposed) loading = false;
    }
  }

  /** The PR head the shown diff belongs to ('' = unknown). */
  let diffHead = '';

  async function loadDiff(rid: string, num: number): Promise<void> {
    diffAbort?.abort();
    const ctl = new AbortController();
    diffAbort = ctl;
    diffLoading = true;
    try {
      const next = await prDiffSummary(rid, num, ctl.signal, pr?.head_sha);
      if (disposed || ctl.signal.aborted || rid !== repoId || num !== number) return;
      diff = next;
      diffHead = prDiffHead(rid, num);
      diffError = null;
    } catch (e) {
      if (isAbortError(e) || ctl.signal.aborted) return;
      if (!disposed && rid === repoId && num === number) diffError = loadErrorText(e);
    } finally {
      if (!disposed && diffAbort === ctl) {
        diffLoading = false;
        diffAbort = null;
      }
    }
  }

  async function loadCommits(rid: string, num: number): Promise<void> {
    commitsLoading = true;
    try {
      const next = await api.get<PrCommit[]>(`/repos/${rid}/prs/${num}/commits`);
      prCommitsCache.set(prKey(rid, num), next);
      if (disposed || rid !== repoId || num !== number) return;
      commits = next;
      commitsError = null;
    } catch (e) {
      if (!disposed && rid === repoId && num === number) commitsError = loadErrorText(e);
    } finally {
      if (!disposed) commitsLoading = false;
    }
  }

  async function requestChanges(): Promise<void> {
    if (busy !== '') return;
    const ok = await confirmOutward({
      verb: 'Request changes',
      title: `Request changes on PR #${number}?`,
      where: `${repoLabel} · PR #${number}${pr ? ` “${pr.title}”` : ''}`,
      what: requestChangesBody.trim() || 'A “changes requested” review with no message.',
      who: `${pr?.author ? `${pr.author} (the author)` : 'The author'} and the PR’s reviewers are notified.`,
    });
    if (!ok || disposed || busy !== '') return;
    busy = 'request-changes';
    try {
      invalidatePr(repoId, number);
      await api.post(`/repos/${repoId}/prs/${number}/request-changes`, {
        body: requestChangesBody.trim() || null,
      });
      if (disposed) return;
      toasts.success('Changes requested', `PR #${number}`);
      showRequestChanges = false;
      requestChangesBody = '';
      await load(repoId, number);
    } catch (e) {
      toastError('Couldn’t request changes', e);
    } finally {
      if (!disposed) busy = '';
    }
  }

  // The shared, self-ticking relative time (lib/stores/now) — no local formatter.
  const formatRelativeDate = (iso: string): string => rel(iso);

  const PR_STATE_LABEL: Record<string, string> = { open: 'Open', merged: 'Merged', declined: 'Declined' };
  const PROVIDER_LABEL: Record<string, string> = { github: 'GitHub', bitbucket: 'Bitbucket', gitlab: 'GitLab' };
  const prRepo = $derived(git.allRepos.find((r) => r.id === repoId) ?? git.repos.find((r) => r.id === repoId) ?? null);
  const providerName = $derived(prRepo?.provider ? (PROVIDER_LABEL[prRepo.provider] ?? prRepo.provider) : 'the provider');

  function startEdit(): void {
    if (!pr) return;
    editTitle = pr.title;
    editDesc = pr.description_md;
    editMode = true;
  }
  /** Close the editor (Cancel or a save); focus returns to its Edit button. */
  function endEdit(): void { editMode = false; void refocusEditTrigger('pr-description'); }

  async function saveEdit(): Promise<void> {
    if (busy !== '') return;
    const ok = await confirmOutward({
      verb: 'Update PR',
      title: `Update PR #${number} on ${providerName}?`,
      where: `${repoLabel} · PR #${number}`,
      what: `Title: ${editTitle.trim() || '(empty)'}${editDesc !== pr?.description_md ? '\nThe description is replaced with your text.' : ''}`,
      who: `${pr?.author ? `${pr.author} (the author)` : 'The author'} and the PR’s reviewers see the change.`,
    });
    if (!ok || disposed || busy !== '') return;
    busy = 'edit';
    try {
      invalidatePr(repoId, number);
      await api.patch(`/repos/${repoId}/prs/${number}`, { title: editTitle, description: editDesc });
      if (disposed) return;
      endEdit();
      await load(repoId, number);
      toasts.success('Pull request updated');
    } catch (e) {
      toastError('Couldn’t update the PR', e);
    } finally {
      if (!disposed) busy = '';
    }
  }

  async function postComment(
    body: string,
    path?: string,
    line?: number,
    inReplyTo?: string,
    anchor?: { side: 'old' | 'new'; oldLine: number | null },
  ): Promise<void> {
    const rid = repoId;
    const num = number;
    const req: NewPrCommentReq = {
      body,
      path: path ?? null,
      line: line ?? null,
      in_reply_to: inReplyTo ?? null,
    };
    if (path && line != null) {
      req.side = anchor?.side ?? 'new';
      req.old_line = anchor?.oldLine ?? null;
      // Anchor to the diff the reviewer is looking at; also spares the daemon
      // a PR round-trip for the head sha.
      if (diffHead) req.commit_id = diffHead;
    }
    invalidatePr(rid, num);
    const posted = await api.post<PrComment>(`/repos/${rid}/prs/${num}/comments`, req);
    if (disposed || rid !== repoId || num !== number) return;
    // Show it now (the full PR reload is 1–2 s of forge calls), then
    // reconcile quietly in the background.
    // Only a well-formed comment is merged; anything else waits for the reload.
    if (pr && typeof posted?.id === 'string' && posted.id !== '') {
      pr = { ...pr, comments: withComment(pr.comments, { ...posted, replies: posted.replies ?? [] }, inReplyTo) };
    }
    void load(rid, num, true);
  }

  /** `comments` with `c` appended — as a reply under the thread whose head
   *  (or one of whose replies) is `parent`, else as a new top-level entry. */
  function withComment(comments: PrComment[], c: PrComment, parent?: string): PrComment[] {
    if (parent) {
      const i = comments.findIndex(
        (h) => h.id === parent || h.thread_id === parent || (h.replies ?? []).some((r) => r.id === parent),
      );
      if (i >= 0) {
        const head = comments[i];
        const next = comments.slice();
        next[i] = { ...head, replies: [...(head.replies ?? []), c] };
        return next;
      }
    }
    return [...comments, c];
  }

  /** Resolve or reopen a review thread on the provider, then refresh statuses. */
  async function resolveThread(threadId: string, resolved: boolean): Promise<void> {
    try {
      invalidatePr(repoId, number);
      await api.post(`/repos/${repoId}/prs/${number}/comments/${encodeURIComponent(threadId)}/resolve`, { resolved });
      if (disposed) return;
      toasts.success(resolved ? 'Thread resolved' : 'Thread reopened');
      await load(repoId, number);
    } catch (e) {
      toastError(resolved ? 'Couldn’t resolve the thread' : 'Couldn’t reopen the thread', e);
    }
  }

  // On a phone the on-screen keyboard can cover the comment box (it's at the
  // bottom of the Summary tab). Scroll it into view once focus lands; deferred so
  // it runs after the keyboard starts animating up.
  function scrollIntoViewOnFocus(e: FocusEvent): void {
    const el = e.currentTarget as HTMLElement;
    setTimeout(() => el.scrollIntoView({ block: 'center', behavior: scrollBehavior() }), 250);
  }

  async function addGeneralComment(): Promise<void> {
    if (busy !== '' || newComment.trim() === '') return;
    busy = 'comment';
    try {
      await postComment(newComment.trim());
      if (!disposed) newComment = '';
    } catch (e) {
      toastError('Couldn’t post the comment', e);
    } finally {
      if (!disposed) busy = '';
    }
  }

  // Approve and Decline are posted to the provider under the user's account and
  // notify the author + reviewers, so both confirm where / what / who first.
  const repoLabel = $derived(prRepo?.name ?? 'this repository');

  // What declining means where the PR lives: Bitbucket Cloud "declines" (and
  // a declined PR can't be reopened); GitHub and GitLab close it, reopenable.
  const declineWhat = $derived(
    prRepo?.provider === 'bitbucket'
      ? `Bitbucket declines the PR: it is closed without merging and can’t be reopened.`
      : `${providerName === 'the provider' ? 'The provider' : providerName} closes the PR without merging. It can be reopened there.`,
  );

  async function action(kind: 'approve' | 'decline'): Promise<void> {
    const approve = kind === 'approve';
    const ok = await confirmOutward({
      verb: approve ? 'Approve PR' : 'Decline PR',
      title: approve ? `Approve PR #${number}?` : `Decline PR #${number}?`,
      where: `${repoLabel} on ${providerName} · PR #${number}${pr ? ` “${pr.title}”` : ''}`,
      what: approve
        ? 'Your approval, posted under your git account.'
        : declineWhat,
      who: `${pr?.author ? `${pr.author} (the author)` : 'The author'} and the PR’s reviewers are notified.`,
      danger: !approve,
    });
    if (!ok || disposed || busy !== '') return;
    busy = kind;
    try {
      invalidatePr(repoId, number);
      await api.post(`/repos/${repoId}/prs/${number}/${kind}`);
      if (disposed) return;
      toasts.success(`PR ${kind === 'approve' ? 'approved' : kind + 'd'}`, `#${number}`);
      await load(repoId, number);
    } catch (e) {
      toastError(approve ? 'Couldn’t approve the PR' : 'Couldn’t decline the PR', e);
    } finally {
      if (!disposed) busy = '';
    }
  }

  async function openAsSession(): Promise<void> {
    if (!pr) return;
    busy = 'session';
    try {
      await ws.createSession({
        kind: 'agent',
        provider: reviewProvider,
        title: `review PR #${pr.number}`,
        meta: {
          pr_context: {
            repo_id: repoId,
            number: pr.number,
            title: pr.title,
            url: pr.url,
            source_branch: pr.source_branch,
            target_branch: pr.target_branch,
          },
        },
      });
      // createSession → addSession → navigateToSession handles routing.
    } catch (e) {
      toastError('Couldn’t open a session', e);
    } finally {
      if (!disposed) busy = '';
    }
  }
;
async () => {



 { svelteHTML.createElement("div", { "class":`prd-page`,});

  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {        "title":pr ? pr.title || `Pull request #${pr.number}` : 'Pull request',"crumbs":[{ label: prRepo ? `${prRepo.name} · Pull requests` : 'Pull requests', onclick: () => router.go(`git/${repoId}/prs`) }],"tabs":pr ? prTabs : undefined,"tabsPlacement":`below`,titleContent:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("span", { "class":`prd-title`,});pr ? pr.title || `Pull request #${pr.number}` : 'Pull request'; }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
    if(pr){
       { svelteHTML.createElement("span", { "class":`prd-num dim`,}); pr.number; }
       { svelteHTML.createElement("span", { "class":`chip ${pr.state === 'open' ? 'ok' : pr.state === 'merged' ? 'accent' : 'bad'}`,});PR_STATE_LABEL[pr.state] ?? pr.state; }
    }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(pr){
       { svelteHTML.createElement("select", {         "class":`prd-provider-select`,"bind:value":reviewProvider,"disabled":busy !== '',"title":`Agent to open the review session on`,"aria-label":`Review agent`,});/*Ωignore_startΩ*/() => reviewProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let p of __sveltets_2_ensureArray(agentProviders())){p;
           { svelteHTML.createElement("option", { "value":p,});p; }
        }
       }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy !== '',"onclick":openAsSession,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}   
       }
       { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-icon":`external`}),"onclick":() => openExternal(pr?.url),"title":`Open this pull request on ${providerName}`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}   providerName;
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {titleContent, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
 { svelteHTML.createElement("div", { "class":`prd`,});
  if(!pr && (loading || prError)){
     { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {             "what":`this pull request`,"variant":`page`,"rows":5,loading,"error":prError,"empty":true,"onretry":() => void load(repoId, number),}});}
  } else if (pr){

     { svelteHTML.createElement("div", { "class":`prd-title-block`,});
      if(editMode){
          {const $$action_0 = __sveltets_2_ensureAction(focusFirstField(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {          "dir":`auto`,"class":`input prd-title-input`,"aria-label":`Pull request title`,"bind:value":editTitle,"disabled":busy === 'edit',});/*Ωignore_startΩ*/() => editTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
      }
       { svelteHTML.createElement("div", { "class":`prd-meta`,});
         { svelteHTML.createElement("span", { "class":`dim`,});pr.author; }
         { svelteHTML.createElement("span", { "class":`mono dim`,});pr.source_branch;  { svelteHTML.createElement("span", { "class":`dir-arrow`,});  } pr.target_branch; }
        if(pr.mergeable === false){ { svelteHTML.createElement("span", { "class":`chip bad`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}  }}
        if(pr.approved_by.length > 0){
           { svelteHTML.createElement("span", {   "class":`chip ok`,"title":pr.approved_by.join(', '),});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} plural(pr.approved_by.length, 'approval');
           }
        }
       }
     }

    
    
     { svelteHTML.createElement("div", { "hidden":activeTab !== 'summary',});
       { svelteHTML.createElement("section", { "class":`prd-desc card`,});
        if(editMode){
           { svelteHTML.createElement("textarea", {           "dir":`auto`,"class":`input`,"rows":8,"bind:value":editDesc,"aria-label":`Pull request description`,"disabled":busy === 'edit',});/*Ωignore_startΩ*/() => editDesc = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           { svelteHTML.createElement("div", { "class":`row prd-compose-foot`,});
             { svelteHTML.createElement("span", { "class":`hint dim`,});  number;  repoLabel; providerName;        }
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy === 'edit',"onclick":endEdit,});  }
             { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":busy === 'edit' || editTitle.trim() === '',"onclick":saveEdit,});
              busy === 'edit' ? 'Saving…' : 'Save to ' + providerName;
             }
           }
        }else{
           { svelteHTML.createElement("div", { "class":`md-body`,});
            
             renderMarkdown(pr.description_md || '_No description._');
           }
           { svelteHTML.createElement("button", {         "class":`btn small ghost edit-btn`,...__sveltets_2_empty({"data-edit-for":`pr-description`}),"aria-label":`Edit the title and description`,"onclick":startEdit,"title":`Edit the title and description on ${providerName}`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}  }
        }
       }

      if(pr.reviewers.length > 0){
         { svelteHTML.createElement("section", { "class":`prd-reviewers card`,});
           { svelteHTML.createElement("div", { "class":`prd-reviewers-title`,});  }
          
              for(let reviewer of __sveltets_2_ensureArray(pr.reviewers)){let i = 1;i;
             { svelteHTML.createElement("div", { "class":`reviewer-row`,});
              if(reviewer.avatar_url){
                 { svelteHTML.createElement("img", {      "class":`reviewer-img`,"src":reviewer.avatar_url,"alt":reviewer.name,});}
              }else{
                 { svelteHTML.createElement("span", { "class":`reviewer-avatar`,});reviewer.name.charAt(0).toUpperCase(); }
              }
               { svelteHTML.createElement("span", { "class":`reviewer-name`,});reviewer.name; }
              if(reviewer.reviewed_at){
                 { svelteHTML.createElement("span", { "class":`reviewer-time dim`,});formatRelativeDate(reviewer.reviewed_at); }
              }
               { svelteHTML.createElement("span", { "class":`reviewer-spacer`,}); }
              if(reviewer.approved){
                 { svelteHTML.createElement("span", { "class":`chip ok`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
              }
             }
          }
         }
      } else if (pr.approved_by.length > 0){
         { svelteHTML.createElement("section", { "class":`prd-reviewers card`,});
           { svelteHTML.createElement("div", { "class":`prd-reviewers-title`,});  }
              for(let name of __sveltets_2_ensureArray(pr.approved_by)){let i = 1;i;
             { svelteHTML.createElement("div", { "class":`reviewer-row`,});
               { svelteHTML.createElement("span", { "class":`reviewer-avatar`,});name.charAt(0).toUpperCase(); }
               { svelteHTML.createElement("span", { "class":`reviewer-name`,});name; }
               { svelteHTML.createElement("span", { "class":`reviewer-spacer`,}); }
               { svelteHTML.createElement("span", { "class":`chip ok`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
             }
          }
         }
      }

      if(pr.state === 'open'){
         { svelteHTML.createElement("section", { "class":`prd-actions card`,});
           { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy !== '',"onclick":() => action('approve'),});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
            busy === 'approve' ? 'Approving…' : 'Approve…';
           }
           { svelteHTML.createElement("button", {         "class":`btn`,"disabled":busy !== '',"aria-expanded":showRequestChanges,"onclick":() => (showRequestChanges = !showRequestChanges),});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
             
           }
           { svelteHTML.createElement("div", { "class":`row merge-group`,});
             { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy !== '',"onclick":() => (mergeOpen = true),});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":12,}});}
              
             }
           }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
          
           { svelteHTML.createElement("button", {           "class":`btn danger`,"disabled":busy !== '',...__sveltets_2_empty({"data-testid":`prd-decline`}),"title":`Close PR #${number} without merging (asks first)`,"onclick":() => action('decline'),});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
            busy === 'decline' ? 'Declining…' : 'Decline…';
           }
         }
        if(showRequestChanges){
           { svelteHTML.createElement("section", { "class":`prd-request-changes card`,});
             { svelteHTML.createElement("div", {   "class":`section-title`,"style":`margin-bottom: 8px`,});  }
             { svelteHTML.createElement("textarea", {              "dir":`auto`,"class":`input`,"rows":3,"bind:value":requestChangesBody,"disabled":busy === 'request-changes',"aria-label":`What needs to change`,"placeholder":`The retry loop needs a cap before this can merge.`,});/*Ωignore_startΩ*/() => requestChangesBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
             { svelteHTML.createElement("div", { "class":`row prd-compose-foot`,});
               { svelteHTML.createElement("span", { "class":`hint dim`,});   number;  repoLabel;        }
               { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":busy === 'request-changes',"onclick":() => (showRequestChanges = false),});  }
               { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":busy === 'request-changes',"onclick":requestChanges,});
                busy === 'request-changes' ? 'Requesting…' : 'Request changes';
               }
             }
           }
        }
      }

       { svelteHTML.createElement("section", { "class":`prd-comments`,});
         { svelteHTML.createElement("div", { "class":`section-title`,});
           generalComments.length;
         }
         { svelteHTML.createElement("p", { "class":`hint dim prd-posts-to`,});      number;  repoLabel;        }
           for(let c of __sveltets_2_ensureArray(generalComments)){c.id;
           { svelteHTML.createElement("div", {   "class":`card`,"style":`padding: 4px 14px 8px; margin-bottom: 8px`,});
             { const $$_daerhTtnemmoC6C = __sveltets_2_ensureComponent(CommentThread); new $$_daerhTtnemmoC6C({ target: __sveltets_2_any(), props: {      "comment":c,"onreply":(parentId, body) => postComment(body, undefined, undefined, parentId),"onresolve":resolveThread,}});}
           }
        }
           { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-s)`,});   }
        

         { svelteHTML.createElement("div", { "class":`new-comment card`,});
           { svelteHTML.createElement("textarea", {               "dir":`auto`,"class":`input`,"rows":3,"bind:value":newComment,"disabled":busy === 'comment',"aria-label":`New comment`,"placeholder":`Leave a comment…`,"onfocus":scrollIntoViewOnFocus,});/*Ωignore_startΩ*/() => newComment = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           { svelteHTML.createElement("div", { "class":`row prd-compose-foot`,});
             { svelteHTML.createElement("span", { "class":`hint dim`,});   number;  repoLabel;        }
             { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":busy === 'comment' || newComment.trim() === '',"onclick":addGeneralComment,});
              busy === 'comment' ? 'Posting…' : 'Post comment';
             }
           }
         }
       }
     }

    
    if(activeTab === 'files' || diff){
       { svelteHTML.createElement("section", {   "class":`prd-diff`,"hidden":activeTab !== 'files',});
        if(diffError || !diff){
           { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {          "what":`the diff`,"loading":diffLoading || !diffError,"error":diffError,"empty":true,"onretry":() => void loadDiff(repoId, number),}});}
        }else{
           { const $$_reweiVffiD4C = __sveltets_2_ensureComponent(DiffViewer); new $$_reweiVffiD4C({ target: __sveltets_2_any(), props: {                  diff,"prMode":true,"showNav":true,"comments":inlineComments,"onAddComment":(path, line, body, anchor) => postComment(body, path, line, undefined, anchor),"onReplyComment":(parentId, body) => postComment(body, undefined, undefined, parentId),"onResolveComment":resolveThread,"loadFile":loadPrFile,"stateKey":`${repoId}#${number}`,}});}
        }
       }
    }

    
    if(activeTab === 'commits'){
       { svelteHTML.createElement("section", { "class":`prd-commits`,});
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL4 = new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`commits`,"loading":commitsLoading || (commits === null && !commitsError),"error":commitsError,"empty":!commits || commits.length === 0,"onretry":() => void loadCommits(repoId, number),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-s); padding: 12px 0`,});   }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL4.$$prop_def;/*Ωignore_endΩ*/
          
             for(let c of __sveltets_2_ensureArray(commits ?? [])){c.sha;
             { svelteHTML.createElement("div", { "class":`commit-row`,});
               { svelteHTML.createElement("span", { "class":`commit-sha mono`,});c.short_sha; }
               { svelteHTML.createElement("span", { "class":`commit-subject`,});c.subject; }
               { svelteHTML.createElement("span", { "class":`commit-author dim`,});c.author; }
               { svelteHTML.createElement("span", { "class":`commit-date dim`,});formatRelativeDate(c.date); }
             }
          }
         LoadState}
       }
    }

    
    if(activeTab === 'review'){
       { svelteHTML.createElement("section", { "class":`prd-review`,});
         { const $$_lenaPweiveR4C = __sveltets_2_ensureComponent(ReviewPanel); new $$_lenaPweiveR4C({ target: __sveltets_2_any(), props: {    "repoId":repoId,"prNumber":number,}});}
       }
    }
  }
 }
 PageBody}
 }

if(mergeOpen && pr){
   { const $$_ladoMegreMrP0C = __sveltets_2_ensureComponent(PrMergeModal); new $$_ladoMegreMrP0C({ target: __sveltets_2_any(), props: {          repoId,number,pr,"onclose":() => {
      mergeOpen = false;
      // The modal's "Open review" link persists the tab and routes here — we
      // are already on this route, so honour the stored choice on close.
      const saved = localStorage.getItem(`otto_pr_tab_${repoId}_${number}`);
      if (saved && TABS.includes(saved as Tab)) activeTab = saved as Tab;
    },"onmerged":() => {
      invalidatePr(repoId, number);
      mergeOpen = false;
      void load(repoId, number);
    },"onstale":() => {
      invalidatePr(repoId, number);
      void load(repoId, number, true);
    },}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PrDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PrDetail__SvelteComponent_ = ReturnType<typeof PrDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default PrDetail__SvelteComponent_;