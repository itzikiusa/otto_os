<script lang="ts">
  // One pull request inside the PR workspace (the right-hand pane of a repo's
  // Pull requests tab; the tab strip above it lives in PrWorkspace). Same
  // layout for GitHub, Bitbucket and GitLab — every action goes through the
  // provider-neutral /repos/{id}/prs/* routes.
  //
  //   header   repo #N · state · Merge · Check out ▾ · ⋯
  //            title, author · opened/updated · checkout command
  //            target ← source                       N files +a −d
  //   tabs     Summary | Timeline | Code | Review (AI agents)
  //
  // Data: the detail paints from the SWR cache; commits + the diff summary
  // load once (header stats, Timeline, the Code tab's commit picker). While a
  // PR is open it is re-read every couple of minutes, but nothing on screen is
  // swapped under the reader: a change only raises an "Updated" pill.
  import { onDestroy, untrack } from 'svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { api, isAbortError } from '../../lib/api/client';
  import type {
    DiffResp,
    MergeStrategy,
    NewPrCommentReq,
    PrChecksResp,
    PrComment,
    PrCommit,
    PrDetail,
    PrSummary,
  } from '../../lib/api/types';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { git } from '../../lib/stores/git.svelte';
  import { confirmOutward } from '../../lib/confirmOutward';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { focusFirstField } from '../../lib/inlineEdit';
  import { renderMarkdown } from '../../lib/md';
  import { openExternal } from '../../lib/external';
  import { plural } from '../../lib/plural';
  import { scrollBehavior } from '../../lib/motion';
  import { loadErrorText } from '../../lib/loadError';
  import { rel } from '../../lib/stores/now.svelte';
  import { pollWhileVisible } from '../../lib/poll';
  import { defaultAgentProvider } from '../../lib/providers';
  import DiffViewer from './DiffViewer.svelte';
  import PrReviewButtons from './PrReviewButtons.svelte';
  import { myGitHandles } from './myGitHandles.svelte';
  import { myReview, withMyReview } from './pr-review-state';
  import { prAgentRequest, type PrAgentKind } from './pr-agent';
  import { fileStat } from './diff-model';
  import { notePrHead, prDiffFileLoader, prDiffHead, prDiffSummary, repoDiffFileLoader } from './diff-load';
  import CommentThread from './CommentThread.svelte';
  import ReviewPanel from './ReviewPanel.svelte';
  import PrMergeModal from './PrMergeModal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import { invalidatePr, prCommitsCache, prDetailCache, prKey } from './pr-cache';
  import PrTimelineView from './PrTimelineView.svelte';
  import PrSideColumn from './PrSideColumn.svelte';
  import PrAvatar from './PrAvatar.svelte';

  interface Props {
    repoId: string;
    number: number;
    /** Fresher PR data for the list pane (title/state/updated/head). */
    onloaded?: (pr: PrSummary) => void;
  }
  let { repoId, number, onloaded }: Props = $props();
  let disposed = false;
  onDestroy(() => { disposed = true; });

  type Tab = 'summary' | 'timeline' | 'code' | 'review';
  const TABS: { id: Tab; label: string }[] = [
    { id: 'summary', label: 'Summary' },
    { id: 'timeline', label: 'Timeline' },
    { id: 'code', label: 'Code' },
    { id: 'review', label: 'AI review' },
  ];
  const tabKey = (): string => `otto_pr_tab_${repoId}_${number}`;
  function savedTab(): Tab {
    try {
      const s = localStorage.getItem(tabKey());
      // 'files' / 'commits' are the pre-redesign tab ids.
      if (s === 'files' || s === 'commits') return 'code';
      return TABS.some((t) => t.id === s) ? (s as Tab) : 'summary';
    } catch {
      return 'summary';
    }
  }
  let activeTab: Tab = $state(untrack(savedTab));
  function selectTab(t: Tab): void {
    activeTab = t;
    try { localStorage.setItem(tabKey(), t); } catch { /* not persisted */ }
  }

  // Server payloads are replaced wholesale: `$state.raw` (no deep proxy over
  // a 100k-line diff).
  let pr = $state.raw<PrDetail | null>(null);
  let diff = $state.raw<DiffResp | null>(null);
  let commits = $state.raw<PrCommit[] | null>(null);
  let checks = $state.raw<PrChecksResp | null>(null);
  const loadPrFile = $derived(prDiffFileLoader(repoId, number));
  let diffAbort: AbortController | null = null;
  onDestroy(() => diffAbort?.abort());
  let loading = $state(true);
  let prError = $state<string | null>(null);
  let diffLoading = $state(false);
  let diffError = $state<string | null>(null);
  let commitsError = $state<string | null>(null);
  /** The PR head the shown diff belongs to ('' = unknown). */
  let diffHead = '';

  let editMode = $state(false);
  let editTitle = $state('');
  let editDesc = $state('');
  let busy = $state('');
  let newComment = $state('');
  let mergeOpen = $state(false);
  let mergeStrategy = $state<MergeStrategy | undefined>(undefined);
  let showRequestChanges = $state(false);
  let requestChangesBody = $state('');
  $effect(() => guardUnsaved(
    () => (editMode && !!pr && (editTitle !== pr.title || editDesc !== pr.description_md))
      || newComment.trim() !== '' || requestChangesBody.trim() !== '',
    { what: `pull request #${number}` },
  ));

  const inlineComments = $derived.by(() => (pr?.comments ?? []).filter((c) => c.path !== null));
  const generalComments = $derived.by(() => (pr?.comments ?? []).filter((c) => c.path === null));

  // ── Loading ──────────────────────────────────────────────────────────────
  $effect(() => {
    const rid = repoId;
    const num = number;
    untrack(() => {
      const hit = prDetailCache.get(prKey(rid, num));
      if (hit) pr = hit.value;
      const ch = prCommitsCache.get(prKey(rid, num));
      if (ch?.fresh) commits = ch.value;
      void load(rid, num, !!hit);
      void loadDiff(rid, num);
      if (!ch?.fresh) void loadCommits(rid, num);
    });
  });

  async function load(rid: string, num: number, quiet = false): Promise<void> {
    if (!quiet) loading = true;
    try {
      const next = await api.get<PrDetail>(`/repos/${rid}/prs/${num}`);
      prDetailCache.set(prKey(rid, num), next);
      if (disposed || rid !== repoId || num !== number) return;
      pr = next;
      prError = null;
      pending = null;
      onloaded?.(next);
      // A push moved the head: re-read the diff under it (the old one stays
      // up until the new one lands).
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
    try {
      const next = await api.get<PrCommit[]>(`/repos/${rid}/prs/${num}/commits`);
      prCommitsCache.set(prKey(rid, num), next);
      if (disposed || rid !== repoId || num !== number) return;
      commits = next;
      commitsError = null;
    } catch (e) {
      if (!disposed && rid === repoId && num === number) commitsError = loadErrorText(e);
    }
  }

  // CI checks: Summary's side column. Loaded once per open; failure is quiet
  // (the row just doesn't render) — checks are context, not the page.
  let checksTried = false;
  $effect(() => {
    if (activeTab !== 'summary' || checksTried || !pr) return;
    checksTried = true;
    const rid = repoId;
    const num = number;
    void api
      .get<PrChecksResp>(`/repos/${rid}/prs/${num}/checks`)
      .then((c) => { if (!disposed && rid === repoId && num === number) checks = c; })
      .catch(() => {});
  });

  // ── Background check for changes (never swaps content under the reader) ──
  const POLL_MS = 2 * 60_000;
  /** A fresher copy of the PR than the one on screen (head or activity moved). */
  let pending = $state.raw<PrDetail | null>(null);
  const pendingNewCommits = $derived(!!pending && !!pr && (pending.head_sha ?? '') !== (pr.head_sha ?? ''));
  $effect(() => {
    const rid = repoId;
    const num = number;
    const poller = pollWhileVisible(async (signal) => {
      if (disposed || !pr) return;
      prDetailCache.delete(prKey(rid, num));
      const next = await api.get<PrDetail>(`/repos/${rid}/prs/${num}`, signal);
      if (disposed || rid !== repoId || num !== number || !pr) return;
      prDetailCache.set(prKey(rid, num), next);
      onloaded?.(next);
      const moved = (next.head_sha ?? '') !== (pr.head_sha ?? '')
        || next.updated_at !== pr.updated_at
        || next.state !== pr.state
        || next.comments.length !== pr.comments.length;
      pending = moved ? next : null;
    }, { ms: POLL_MS });
    return () => poller.stop();
  });
  function applyPending(): void {
    pending = null;
    invalidatePr(repoId, number);
    commits = null;
    void load(repoId, number);
    void loadCommits(repoId, number);
  }
  function refreshAll(): void {
    invalidatePr(repoId, number);
    checksTried = false;
    checks = null;
    applyPending();
    toasts.info('Refreshing', `PR #${number}`);
  }

  // ── Labels / provider wording ────────────────────────────────────────────
  const PR_STATE_LABEL: Record<string, string> = { open: 'Open', merged: 'Merged', declined: 'Declined' };
  const PROVIDER_LABEL: Record<string, string> = { github: 'GitHub', bitbucket: 'Bitbucket', gitlab: 'GitLab' };
  const prRepo = $derived(git.allRepos.find((r) => r.id === repoId) ?? git.repos.find((r) => r.id === repoId) ?? null);
  const providerName = $derived(prRepo?.provider ? (PROVIDER_LABEL[prRepo.provider] ?? prRepo.provider) : 'the provider');
  const repoLabel = $derived(prRepo?.name ?? 'this repository');
  /** "owner/repo" from the remote URL (falls back to the local name). */
  const repoSlug = $derived.by(() => {
    const u = prRepo?.remote_url ?? '';
    const m = u.match(/[:/]([^/:]+\/[^/]+?)(?:\.git)?\/?$/);
    return m?.[1] ?? repoLabel;
  });
  const declineWhat = $derived(
    prRepo?.provider === 'bitbucket'
      ? `Bitbucket declines the PR: it is closed without merging and can’t be reopened.`
      : `${providerName === 'the provider' ? 'The provider' : providerName} closes the PR without merging. It can be reopened there.`,
  );
  const stateGlyph = (s: string): IconName => (s === 'merged' ? 'merge' : s === 'declined' ? 'x' : 'pr');
  const checkoutCmd = $derived(pr ? `git fetch origin ${pr.source_branch} && git checkout ${pr.source_branch}` : '');

  const stats = $derived.by(() => {
    if (!diff) return null;
    let add = diff.total_added ?? 0;
    let del = diff.total_deleted ?? 0;
    if (diff.total_added == null || diff.total_deleted == null) {
      add = 0;
      del = 0;
      for (const f of diff.files) { const st = fileStat(f); add += st.add; del += st.del; }
    }
    return { files: diff.files.length, add, del };
  });

  // ── Writes (each one confirms where / what / who first) ──────────────────
  function startEdit(): void {
    if (!pr) return;
    editTitle = pr.title;
    editDesc = pr.description_md;
    editMode = true;
    selectTab('summary');
  }

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
      editMode = false;
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
    const req: NewPrCommentReq = { body, path: path ?? null, line: line ?? null, in_reply_to: inReplyTo ?? null };
    if (path && line != null) {
      req.side = anchor?.side ?? 'new';
      req.old_line = anchor?.oldLine ?? null;
      if (diffHead) req.commit_id = diffHead;
    }
    invalidatePr(rid, num);
    const posted = await api.post<PrComment>(`/repos/${rid}/prs/${num}/comments`, req);
    if (disposed || rid !== repoId || num !== number) return;
    if (pr && typeof posted?.id === 'string' && posted.id !== '') {
      pr = { ...pr, comments: withComment(pr.comments, { ...posted, replies: posted.replies ?? [] }, inReplyTo) };
    }
    void load(rid, num, true);
  }

  function withComment(comments: PrComment[], c: PrComment, parent?: string): PrComment[] {
    if (parent) {
      const i = comments.findIndex(
        (h) => h.id === parent || h.thread_id === parent || (h.replies ?? []).some((r) => r.id === parent),
      );
      if (i >= 0) {
        const next = comments.slice();
        next[i] = { ...comments[i], replies: [...(comments[i].replies ?? []), c] };
        return next;
      }
    }
    return [...comments, c];
  }

  async function resolveThread(threadId: string, resolved: boolean): Promise<void> {
    try {
      invalidatePr(repoId, number);
      await api.post(`/repos/${repoId}/prs/${number}/comments/${encodeURIComponent(threadId)}/resolve`, { resolved });
      if (disposed) return;
      toasts.success(resolved ? 'Thread resolved' : 'Thread reopened');
      await load(repoId, number, true);
    } catch (e) {
      toastError(resolved ? 'Couldn’t resolve the thread' : 'Couldn’t reopen the thread', e);
    }
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

  // ── Your review: shown in the header, patched the moment a write lands ──
  myGitHandles.load();
  const isMe = (name: string): boolean => myGitHandles.isMe(name);
  const myName = $derived([...myGitHandles.handles][0] ?? 'You');
  const mine = $derived(pr ? myReview(pr, isMe) : null);
  function patchNow(next: PrDetail): void {
    pr = next;
    prDetailCache.set(prKey(repoId, number), next);
    onloaded?.(next);
  }
  function openRequestChanges(): void {
    selectTab('summary');
    showRequestChanges = true;
  }

  async function action(kind: 'approve' | 'decline'): Promise<void> {
    const approve = kind === 'approve';
    const ok = await confirmOutward({
      verb: approve ? 'Approve PR' : 'Decline PR',
      title: approve ? `Approve PR #${number}?` : `Decline PR #${number}?`,
      where: `${repoLabel} on ${providerName} · PR #${number}${pr ? ` “${pr.title}”` : ''}`,
      what: approve ? 'Your approval, posted under your git account.' : declineWhat,
      who: `${pr?.author ? `${pr.author} (the author)` : 'The author'} and the PR’s reviewers are notified.`,
      danger: !approve,
    });
    if (!ok || disposed || busy !== '') return;
    busy = kind;
    try {
      invalidatePr(repoId, number);
      await api.post(`/repos/${repoId}/prs/${number}/${kind}`);
      if (disposed) return;
      toasts.success(`PR ${approve ? 'approved' : 'declined'}`, `#${number}`);
      if (pr) patchNow(approve ? withMyReview(pr, 'approved', isMe, myName) : { ...pr, state: 'declined' });
      await load(repoId, number, true);
    } catch (e) {
      toastError(approve ? 'Couldn’t approve the PR' : 'Couldn’t decline the PR', e);
    } finally {
      if (!disposed) busy = '';
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
      await api.post(`/repos/${repoId}/prs/${number}/request-changes`, { body: requestChangesBody.trim() || null });
      if (disposed) return;
      toasts.success('Changes requested', `PR #${number}`);
      showRequestChanges = false;
      requestChangesBody = '';
      if (pr) patchNow(withMyReview(pr, 'changes', isMe, myName));
      await load(repoId, number, true);
    } catch (e) {
      toastError('Couldn’t request changes', e);
    } finally {
      if (!disposed) busy = '';
    }
  }

  function openMerge(strategy?: MergeStrategy): void {
    mergeStrategy = strategy;
    mergeOpen = true;
  }

  // ── Local: check out the branch (fetch, then switch; local edits stashed) ─
  async function checkoutBranch(): Promise<void> {
    if (!pr || busy !== '') return;
    const branch = pr.source_branch;
    busy = 'checkout';
    try {
      await api.post(`/repos/${repoId}/fetch`);
      await git.checkout(repoId, { branch, auto_stash: true });
      if (!disposed) toasts.success(`Checked out ${branch}`, 'Any local changes were stashed first and restored.');
    } catch (e) {
      toastError(`Couldn’t check out ${branch}`, e);
    } finally {
      if (!disposed) busy = '';
    }
  }

  function checkoutMenu(e: MouseEvent): void {
    if (!pr) return;
    ctxMenu.show(e, [
      { label: `Check out ${pr.source_branch}`, icon: 'branch', disabled: busy !== '', action: () => void checkoutBranch() },
      { label: 'Copy checkout command', icon: 'copy', action: () => void copyWithToast(checkoutCmd, 'Checkout command') },
      { label: 'Copy branch name', icon: 'copy', action: () => void copyWithToast(pr?.source_branch ?? '', 'Branch name') },
    ]);
  }

  // ── Agent sessions that already know which PR is meant ───────────────────
  async function openAgent(kind: PrAgentKind): Promise<void> {
    if (!pr || busy !== '') return;
    busy = 'session';
    try {
      const req = prAgentRequest(kind, pr, { repoId, repoSlug, cwd: prRepo?.path ?? null, provider: defaultAgentProvider() });
      await ws.openSessionWithPrompt(req, { route: true });
    } catch (e) {
      toastError('Couldn’t open a session', e);
    } finally {
      if (!disposed) busy = '';
    }
  }

  function moreMenu(e: MouseEvent): void {
    if (!pr) return;
    const open = pr.state === 'open';
    const items: MenuItem[] = [
      { label: 'Refresh', icon: 'refresh', action: refreshAll },
      { separator: true },
      { label: 'Ask a question', icon: 'comment', hint: 'agent', action: () => void openAgent('ask') },
      { label: 'Explain this PR', icon: 'sparkle', hint: 'agent', action: () => void openAgent('explain') },
      { label: 'Fix findings in a session', icon: 'zap', hint: 'agent', disabled: !open, action: () => void openAgent('fix') },
      { separator: true },
      { label: 'Merge', icon: 'merge', disabled: !open, action: () => openMerge('merge') },
      { label: 'Squash and merge', icon: 'merge', disabled: !open, action: () => openMerge('squash') },
      { label: 'Rebase and merge', icon: 'merge', disabled: !open, action: () => openMerge('rebase') },
      { separator: true },
      { label: 'Approve…', icon: 'check', disabled: !open, action: () => void action('approve') },
      { label: 'Request changes…', icon: 'warning', disabled: !open, action: openRequestChanges },
      { label: 'Edit title and description', icon: 'edit', action: startEdit },
      { separator: true },
      { label: `Open on ${providerName}`, icon: 'external', action: () => openExternal(pr?.url) },
      { label: 'Copy link', icon: 'link', action: () => void copyWithToast(pr?.url ?? '', 'Link') },
      { label: 'Copy PR number', icon: 'copy', action: () => void copyWithToast(`#${pr?.number}`, 'PR number') },
    ];
    if (open) {
      items.push({ separator: true }, { label: prRepo?.provider === 'bitbucket' ? 'Decline pull request…' : 'Close pull request…', icon: 'x', danger: true, action: () => void action('decline') });
    }
    ctxMenu.show(e, items);
  }

  // ── Code tab: one commit or all of them ──────────────────────────────────
  let commitSha = $state<string | null>(null);
  let commitDiff = $state.raw<DiffResp | null>(null);
  let commitError = $state<string | null>(null);
  let commitLoading = $state(false);
  const selectedCommit = $derived(commitSha ? (commits ?? []).find((c) => c.sha === commitSha) ?? null : null);
  const commitLoader = $derived(commitSha ? repoDiffFileLoader(repoId, `commit:${commitSha}`) : undefined);

  async function pickCommit(sha: string | null): Promise<void> {
    commitSha = sha;
    commitDiff = null;
    commitError = null;
    if (!sha) return;
    commitLoading = true;
    const url = `/repos/${repoId}/diff?target=${encodeURIComponent(`commit:${sha}`)}&summary=true`;
    try {
      let d: DiffResp;
      try {
        d = await api.get<DiffResp>(url);
      } catch {
        // The commit may not be local yet: fetch once, then retry.
        await api.post(`/repos/${repoId}/fetch`);
        d = await api.get<DiffResp>(url);
      }
      if (disposed || commitSha !== sha) return;
      commitDiff = d;
    } catch (e) {
      if (!disposed && commitSha === sha) commitError = `${loadErrorText(e)} The commit isn’t in your local clone (a fork’s commit can’t be fetched from origin).`;
    } finally {
      if (!disposed && commitSha === sha) commitLoading = false;
    }
  }

  function commitMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'All commits', checked: commitSha === null, pinned: true, action: () => void pickCommit(null) },
      { separator: true, pinned: true },
      ...(commits ?? []).map((c) => ({
        label: c.subject,
        hint: c.short_sha,
        checked: commitSha === c.sha,
        title: `${c.short_sha} · ${c.author} · ${rel(c.date)}`,
        action: () => void pickCommit(c.sha),
      })),
    ];
    if (commits === null) items.push({ label: commitsError ? 'Couldn’t load commits' : 'Loading commits…', disabled: true });
    ctxMenu.show(e, items, { filter: (commits?.length ?? 0) > 8, filterPlaceholder: 'Filter commits…' });
  }

  function scrollIntoViewOnFocus(e: FocusEvent): void {
    const el = e.currentTarget as HTMLElement;
    setTimeout(() => el.scrollIntoView({ block: 'center', behavior: scrollBehavior() }), 250);
  }

</script>


{#snippet commitPicker()}
  <button class="btn small ghost prd-commit-pick" aria-haspopup="menu" onclick={commitMenu}
    title={selectedCommit ? `${selectedCommit.short_sha} · ${selectedCommit.subject}` : 'Show the changes of one commit'}>
    <span class="prd-commit-label">{selectedCommit ? selectedCommit.subject : 'All commits'}</span>
    <Icon name="chevronDown" size={12} />
  </button>
{/snippet}

<div class="prd" data-pr={number}>
  {#if !pr && (loading || prError)}
    <div class="prd-pad">
      <LoadState what="this pull request" variant="page" rows={5} {loading} error={prError} empty onretry={() => void load(repoId, number)} />
    </div>
  {:else if pr}
    <header class="prd-head">
      <div class="prd-row prd-row-top">
        <button class="prd-repo link" onclick={() => openExternal(pr?.url)} title="Open on {providerName}">
          <span class="prd-slug">{repoSlug}</span>
          <span class="prd-num">#{pr.number}</span>
          <Icon name="external" size={12} />
        </button>
        <span class="prd-state st-{pr.draft && pr.state === 'open' ? 'draft' : pr.state}">
          <Icon name={stateGlyph(pr.state)} size={12} />
          {pr.draft && pr.state === 'open' ? 'Draft' : (PR_STATE_LABEL[pr.state] ?? pr.state)}
        </span>
        {#if pr.mergeable === false}<span class="prd-state st-declined"><Icon name="warning" size={12} /> Conflicts</span>{/if}
        <span class="grow"></span>
        {#if pending}
          <button class="btn small prd-pending" onclick={applyPending} title="Load the latest version of this pull request">
            <Icon name="refresh" size={12} /> {pendingNewCommits ? 'New commits' : 'Updated'} · Refresh
          </button>
        {/if}
        <div class="prd-actions">
          {#if pr.state === 'open'}
            <PrReviewButtons review={mine} {busy} onapprove={() => void action('approve')} onrequestchanges={openRequestChanges} />
            <button class="btn small primary" disabled={busy !== ''} onclick={() => openMerge()} title="Merge this pull request (asks first)">
              <Icon name="merge" size={12} /> Merge
            </button>
          {/if}
          <button class="btn small" aria-haspopup="menu" disabled={busy === 'checkout'} onclick={checkoutMenu} title="Check out this pull request locally">
            <Icon name="branch" size={12} /> {busy === 'checkout' ? 'Checking out…' : 'Check out'} <Icon name="chevronDown" size={12} />
          </button>
          <button class="btn small ghost icon-only" aria-haspopup="menu" aria-label="More pull request actions" title="More actions" onclick={moreMenu}>
            <Icon name="more" size={14} />
          </button>
        </div>
      </div>

      {#if editMode}
        <input dir="auto" class="input prd-title-input" aria-label="Pull request title" bind:value={editTitle} disabled={busy === 'edit'} use:focusFirstField />
      {:else}
        <h2 class="prd-title" dir="auto">{pr.title || `Pull request #${pr.number}`}</h2>
      {/if}

      <div class="prd-row prd-meta">
        <PrAvatar name={pr.author} />
        <span class="prd-author">{pr.author}</span>
        <span class="dim" title={new Date(pr.updated_at).toLocaleString()}>
          {pr.created_at ? `opened ${rel(pr.created_at)} · ` : ''}updated {rel(pr.updated_at)}
        </span>
        <button class="prd-cmd mono" onclick={() => void copyWithToast(checkoutCmd, 'Checkout command')} title="Copy: {checkoutCmd}">{checkoutCmd}</button>
      </div>

      <div class="prd-row prd-branches">
        <span class="mono dim prd-branch" title="{pr.source_branch} → {pr.target_branch}">
          {pr.target_branch} <span class="dir-arrow" aria-label="from">←</span> {pr.source_branch}
        </span>
        <span class="grow"></span>
        {#if stats}
          <span class="prd-stats" title="Files changed, lines added and deleted">
            <Icon name="file" size={12} /> {plural(stats.files, 'file')}
            <span class="add">+{stats.add.toLocaleString()}</span>
            <span class="del">−{stats.del.toLocaleString()}</span>
          </span>
        {/if}
      </div>

      <div class="segmented prd-tabs" role="tablist" aria-label="Pull request views" tabindex="-1" onkeydown={onTabKey}>
        {#each TABS as t (t.id)}
          <button role="tab" aria-label={t.label} aria-selected={activeTab === t.id} tabindex={activeTab === t.id ? 0 : -1}
            class:active={activeTab === t.id} onclick={() => selectTab(t.id)}>
            {t.label}
            {#if t.id === 'summary' && generalComments.length > 0}<span class="prd-tab-count" aria-hidden="true">{generalComments.length}</span>{/if}
          </button>
        {/each}
      </div>
    </header>

    <div class="prd-body" class:code={activeTab === 'code'}>
      <!-- Summary stays mounted (hidden) so composer drafts survive a tab switch. -->
      <div class="prd-summary" hidden={activeTab !== 'summary'}>
        <div class="prd-main">
          <section class="prd-desc card">
            {#if editMode}
              <textarea dir="auto" class="input" rows="10" bind:value={editDesc} aria-label="Pull request description" disabled={busy === 'edit'}></textarea>
              <div class="row prd-compose-foot">
                <span class="hint dim">Updates PR #{number} on {repoLabel} ({providerName}) · visible to the author and reviewers</span>
                <button class="btn small" disabled={busy === 'edit'} onclick={() => (editMode = false)}>Cancel</button>
                <button class="btn small primary" disabled={busy === 'edit' || editTitle.trim() === ''} onclick={saveEdit}>
                  {busy === 'edit' ? 'Saving…' : 'Save to ' + providerName}
                </button>
              </div>
            {:else}
              <div class="md-body">
                <!-- renderMarkdown escapes input before transforming -->
                {@html renderMarkdown(pr.description_md || '_No description._')}
              </div>
              <button class="btn small ghost edit-btn" aria-label="Edit the title and description" onclick={startEdit} title="Edit the title and description on {providerName}"><Icon name="edit" size={12} /> Edit</button>
            {/if}
          </section>

          {#if pr.state === 'open'}
            <section class="prd-review-bar">
              <span class="grow"></span>
              <button class="btn small ghost danger-text" data-testid="prd-decline" disabled={busy !== ''} title="Close PR #{number} without merging (asks first)" onclick={() => action('decline')}>
                {busy === 'decline' ? 'Declining…' : 'Decline…'}
              </button>
            </section>
            {#if showRequestChanges}
              <section class="prd-request-changes card">
                <textarea dir="auto" class="input" rows="3" bind:value={requestChangesBody} disabled={busy === 'request-changes'} use:focusFirstField
                  aria-label="What needs to change" placeholder="The retry loop needs a cap before this can merge."></textarea>
                <div class="row prd-compose-foot">
                  <span class="hint dim">Posts to PR #{number} on {repoLabel} · visible to the author and reviewers</span>
                  <button class="btn small ghost" disabled={busy === 'request-changes'} onclick={() => (showRequestChanges = false)}>Cancel</button>
                  <button class="btn small primary" disabled={busy === 'request-changes'} onclick={requestChanges}>
                    {busy === 'request-changes' ? 'Requesting…' : 'Request changes'}
                  </button>
                </div>
              </section>
            {/if}
          {/if}

          <section class="prd-comments">
            <h3 class="prd-h">Conversation <span class="dim">{generalComments.length}</span></h3>
            {#each generalComments as c (c.id)}
              <div class="card prd-thread">
                <CommentThread comment={c} onreply={(parentId, body) => postComment(body, undefined, undefined, parentId)} onresolve={resolveThread} />
              </div>
            {:else}
              <p class="dim prd-none">No comments yet.</p>
            {/each}
            <div class="new-comment card">
              <textarea dir="auto" class="input" rows="3" bind:value={newComment} disabled={busy === 'comment'} aria-label="New comment" placeholder="Leave a comment…" onfocus={scrollIntoViewOnFocus}></textarea>
              <div class="row prd-compose-foot">
                <span class="hint dim">Posts to PR #{number} on {repoLabel} · visible to the author and reviewers</span>
                <button class="btn small" disabled={busy === 'comment' || newComment.trim() === ''} onclick={addGeneralComment}>
                  {busy === 'comment' ? 'Posting…' : 'Post comment'}
                </button>
              </div>
            </div>
          </section>
        </div>

        <PrSideColumn {pr} {checks} {commits} {providerName} />
      </div>

      {#if activeTab === 'timeline'}
        <PrTimelineView {pr} {commits} {commitsError} onopencode={() => selectTab('code')} onpickcommit={(sha) => { selectTab('code'); void pickCommit(sha); }} />
      {/if}

      {#if activeTab === 'code' || diff}
        <section class="prd-code" hidden={activeTab !== 'code'}>
          {#if commitSha}
            {#if commitDiff}
              <DiffViewer
                diff={commitDiff}
                prMode={true}
                loadFile={commitLoader}
                toolbarLead={commitPicker}
                stateKey={`${repoId}#${number}@${commitSha}`}
                viewedStorageKey={`otto_pr_viewed:${repoId}#${number}`}
              />
            {:else}
              <div class="prd-code-bar">{@render commitPicker()}</div>
              <LoadState what="this commit’s changes" loading={commitLoading} error={commitError} empty onretry={() => void pickCommit(commitSha)} />
            {/if}
          {:else if diffError || !diff}
            <LoadState what="the diff" loading={diffLoading || !diffError} error={diffError} empty onretry={() => void loadDiff(repoId, number)} />
          {:else}
            <DiffViewer
              {diff}
              prMode={true}
              showNav={true}
              comments={inlineComments}
              onAddComment={(path, line, body, anchor) => postComment(body, path, line, undefined, anchor)}
              onReplyComment={(parentId, body) => postComment(body, undefined, undefined, parentId)}
              onResolveComment={resolveThread}
              loadFile={loadPrFile}
              toolbarLead={commitPicker}
              stateKey={`${repoId}#${number}`}
              viewedStorageKey={`otto_pr_viewed:${repoId}#${number}`}
            />
          {/if}
        </section>
      {/if}

      {#if activeTab === 'review'}
        <section class="prd-review">
          <ReviewPanel repoId={repoId} prNumber={number} />
        </section>
      {/if}
    </div>
  {/if}
</div>

{#if mergeOpen && pr}
  <PrMergeModal
    {repoId}
    {number}
    {pr}
    initialStrategy={mergeStrategy}
    onclose={() => {
      mergeOpen = false;
      activeTab = savedTab();
    }}
    onmerged={() => {
      invalidatePr(repoId, number);
      mergeOpen = false;
      void load(repoId, number);
    }}
    onstale={() => {
      invalidatePr(repoId, number);
      void load(repoId, number, true);
    }}
  />
{/if}

<style>
  .prd {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .prd-pad { padding: 16px; }
  .prd-head {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 16px 0;
    border-block-end: 1px solid var(--separator);
  }
  .prd-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .prd-repo {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .prd-slug { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .prd-num { color: var(--success); font-variant-numeric: tabular-nums; }
  .prd-state {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 600;
    white-space: nowrap;
  }
  .st-open { color: var(--success); background: var(--success-soft); }
  .st-merged { color: var(--accent-text); background: var(--accent-soft); }
  .st-declined { color: var(--danger); background: var(--danger-soft); }
  .st-draft { color: var(--text-dim); background: var(--surface-2); }
  .prd-actions { display: inline-flex; align-items: center; gap: 6px; flex: none; }
  .icon-only { padding-inline: 6px; }
  .prd-pending { color: var(--info); border-color: color-mix(in srgb, var(--info) 40%, var(--border)); }
  .prd-title {
    margin: 2px 0 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    line-height: 1.25;
    overflow-wrap: anywhere;
  }
  .prd-title-input { width: 100%; font-size: var(--fs-l); height: 32px; }
  .prd-meta { font-size: var(--fs-s); }
  .prd-author { font-weight: 550; white-space: nowrap; }
  .prd-meta .dim { white-space: nowrap; }
  .prd-cmd {
    min-width: 0;
    flex: 1 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: start;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: none;
    border: none;
    padding: 0;
    cursor: copy;
  }
  .prd-cmd:hover { color: var(--text); }
  .prd-branches { font-size: var(--fs-s); }
  .prd-branch { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .prd-stats { display: inline-flex; align-items: center; gap: 6px; white-space: nowrap; font-variant-numeric: tabular-nums; color: var(--text-dim); }
  .add { color: var(--success); }
  .del { color: var(--danger); }
  .prd-tabs { align-self: flex-start; margin-block: 6px 10px; }
  .prd-tab-count { margin-inline-start: 4px; font-size: var(--fs-xs); color: var(--text-dim); }

  .prd-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 14px 16px 24px;
  }
  .prd-body.code { padding: 0; display: flex; flex-direction: column; }
  .prd-code { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .prd-code[hidden] { display: none; }
  .prd-code-bar { padding: 8px 12px; border-block-end: 1px solid var(--separator); }
  .prd-commit-pick { max-width: 340px; gap: 4px; font-weight: 550; }
  .prd-commit-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* Summary: content + a details column (stacks under ~900px of pane). */
  .prd-summary {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 240px;
    gap: 20px;
    align-items: start;
  }
  .prd-summary[hidden] { display: none; }
  @container (max-width: 860px) {
    .prd-summary { grid-template-columns: minmax(0, 1fr); }
  }
  .prd-body { container-type: inline-size; }
  .prd-main { min-width: 0; display: flex; flex-direction: column; gap: 12px; }
  .prd-desc { position: relative; padding: 12px 16px; }
  .prd-desc textarea { width: 100%; }
  .edit-btn {
    position: absolute;
    top: 8px;
    inset-inline-end: 8px;
    opacity: 0;
    transition: opacity var(--dur-fast) ease-out;
  }
  .prd-desc:hover .edit-btn, .prd-desc:focus-within .edit-btn, .edit-btn:focus-visible { opacity: 1; }
  @media (hover: none) { .edit-btn { opacity: 1; } }
  .prd-review-bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .danger-text { color: var(--danger); }
  .prd-request-changes { padding: 12px 16px; }
  .prd-request-changes textarea, .new-comment textarea { width: 100%; }
  .prd-h {
    margin: 0 0 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .prd-thread { padding: 4px 14px 8px; margin-bottom: 8px; }
  .prd-none { font-size: var(--fs-s); margin: 0 0 8px; }
  .new-comment { padding: 12px; }
  .prd-compose-foot { justify-content: flex-end; align-items: center; flex-wrap: wrap; gap: 8px; margin-top: 8px; }
  .prd-compose-foot .hint { flex: 1 1 220px; min-width: 0; font-size: var(--fs-xs); }

  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent-text);
    cursor: pointer;
    font: inherit;
  }
  .link:hover { text-decoration: underline; }
  .prd-repo.link { color: var(--text-dim); }
  .dir-arrow { display: inline-block; }
  :global([dir='rtl']) .dir-arrow { transform: scaleX(-1); }

  @media (max-width: 1024px) {
    /* One scroller for the whole PR: the header scrolls away with the content
       (a fixed header left a phone in landscape 0 px for the diff), and the
       diff's toolbar pins to the top of THIS scroller — so the diff must not
       be a scroller of its own here (its phone default for bare hosts). */
    .prd { overflow-y: auto; overflow-x: hidden; overscroll-behavior: contain; }
    .prd-head, .prd-body, .prd-code { flex: none; min-height: auto; overflow: visible; }
    .prd-code :global(.diff) { overflow-y: visible; }
    .prd-head { padding: 10px 12px 0; }
    .prd-row-top { flex-wrap: wrap; }
    .prd-actions { flex-wrap: wrap; flex: 1 1 100%; }
    .prd-actions .btn { height: 34px; }
    .prd-meta { flex-wrap: wrap; }
    .prd-cmd { flex-basis: 100%; }
    .prd-title-input { height: 38px; font-size: 16px; } /* ui-guards: allow — 16px stops iOS zoom-on-focus */
    .prd-body { padding: 12px; }
    .prd-summary { grid-template-columns: minmax(0, 1fr); }
    .new-comment textarea, .prd-desc textarea, .prd-request-changes textarea { font-size: var(--fs-l); }
    .edit-btn { opacity: 1; }
  }
</style>
