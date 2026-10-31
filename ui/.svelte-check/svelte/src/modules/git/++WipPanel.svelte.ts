///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import { plural } from '../../lib/plural';
// GitKraken-style WIP panel: shown in the graph's RIGHT detail pane when the
// WIP row is selected. Unstaged / Staged file trees (per-file + per-folder
// stage toggles, discard), a per-file working diff, and the commit composer.
// Replaces the old separate "Changes" tab — staging now lives on the graph.
import { onDestroy, tick, untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { api, isAbortError } from '../../lib/api/client';
import type {
    CommitConfig, CommitInfo, DiffResp, DiscardReq, DraftCommitMessageResp, FileChange, HeadRemotesResp, RepoStatusResp,
  } from '../../lib/api/types';
import { amendPublishedAt } from './refTracking';
import { toasts } from '../../lib/toast.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { git } from '../../lib/stores/git.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import DiffViewer from './DiffViewer.svelte';
import { repoDiffFileLoader } from './diff-load';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import { ListWindow } from '../../lib/listWindow.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { draftInFlight, readComposer, startDraft, takePendingDraft, watchComposer, writeComposer } from './commitComposer';
;

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Called after a successful commit so the graph can reload its log. */
    oncommitted: () => void;
    onclose: () => void;
    /** Open the conflict resolver (RepoView's tab) for a conflicted file. */
    onresolve?: () => void;
  };;

  // ── Keyboard focus across index moves ─────────────────────────────────
  // Staging moves a path to the other list, so Svelte destroys the row —
  // and the focused checkbox with it — and focus fell to <body>. A row
  // control passes its origin; after the new status renders, focus goes to
  // the checkbox now at the same index in the same section (else the last
  // row, else the section heading), and a polite status says what happened.
  type Origin = { section: 'unstaged' | 'staged'; key: string };;;

  // ── Folder tree (shared shape with the old Changes tab) ────────────────────
  type TFile = { type: 'file'; name: string; change: FileChange };;function $$render() {
 const fileRow/*Ωignore_positionΩ*/ = (file: TFile, depth: number, section: 'unstaged' | 'staged')/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {         "class":`wp-file`,"style":`padding-inline-start:${8 + depth * 14}px`,"oncontextmenu":(e) => fileMenu(e, file.change, section),});selectedPath === file.change.path;
     { svelteHTML.createElement("input", {             "type":`checkbox`,"checked":section === 'staged',...__sveltets_2_empty({"data-row-key":`f:${file.change.path}`}),"onchange":() =>
        stagePaths([file.change.path], section === 'unstaged', { section, key: `f:${file.change.path}` }),"title":section === 'staged' ? 'Unstage' : 'Stage',"aria-label":`${section === 'staged' ? 'Unstage' : 'Stage'} ${file.change.path}`,});}
     { svelteHTML.createElement("button", {           "class":`wp-name`,"onclick":() => {
        // A partially staged path is in both lists: open the side clicked.
        stagedView = section === 'staged';
        selectedPath = selectedPath === file.change.path ? null : file.change.path;
      },"title":file.change.path,"aria-expanded":selectedPath === file.change.path,"aria-controls":selectedPath === file.change.path ? diffId : undefined,});
       { svelteHTML.createElement("span", { "class":`kind k-${file.change.kind}`,});kindBadge[file.change.kind]; }
       { svelteHTML.createElement("span", { "class":`mono wp-fname`,});file.name; }
      if(partial.has(file.change.path)){
        
         { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {          "tone":`accent`,"variant":`outline`,"label":`Partial`,"title":`Partially staged`,"testid":`wip-partial`,}});}
      }
     }
     { svelteHTML.createElement("button", {         "class":`wp-discard`,"title":section === 'unstaged' ? 'Discard unstaged changes to this file' : 'Discard changes to this file',"aria-label":`Discard ${file.change.path}`,"onclick":() =>
        void discardPaths([file.change.path], file.change.path, section, { section, key: `f:${file.change.path}` }),});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
     }
   }}
};return __sveltets_2_any(0)}; const folderRow/*Ωignore_positionΩ*/ = (node: TFolder, depth: number, section: 'unstaged' | 'staged')/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
    const key = `${section}:${node.path}`;
    const shut = isCollapsed(key, node);
    
    
     {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {        "class":`wp-folder`,"style":`padding-inline-start:${8 + depth * 14}px`,"oncontextmenu":(e) => folderMenu(e, node, section),});
       { svelteHTML.createElement("input", {             "type":`checkbox`,"checked":section === 'staged',...__sveltets_2_empty({"data-row-key":`d:${node.path}`}),"onchange":() =>
          stagePaths(
            node.files.map((f) => f.path),
            section === 'unstaged',
            { section, key: `d:${node.path}` },
          ),"title":`${section === 'staged' ? 'Unstage' : 'Stage'} ${node.path}/ (${plural(node.files.length, 'file')})`,"aria-label":`${section === 'staged' ? 'Unstage' : 'Stage'} folder ${node.path}`,});}
       { svelteHTML.createElement("button", {         "class":`wp-fold-name`,"onclick":() => toggleFolder(key, node),"title":`${node.path}/ (${node.files.length})`,"aria-expanded":!shut,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":shut ? 'chevronRight' : 'chevronDown',"size":12,}});}
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`wp-fold-label`,});node.name; }
         { svelteHTML.createElement("span", { "class":`wp-fold-count`,});node.files.length; }
       }
      
       { svelteHTML.createElement("button", {         "class":`wp-discard`,"title":`Discard changes to every file under ${node.path}/`,"aria-label":`Discard folder ${node.path}`,"onclick":() =>
          void discardPaths(
            node.files.map((f) => f.path),
            `${node.path}/ (${plural(node.files.length, 'file')})`,
            section,
            { section, key: `d:${node.path}` },
          ),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
       }
     }}
};return __sveltets_2_any(0)}; const sectionRows/*Ωignore_positionΩ*/ = (rows: Row[], section: 'unstaged' | 'staged', empty: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const lw = winOf(section);
  const win = lw.range(rows.length);
  if(rows.length === 0){
     { svelteHTML.createElement("div", { "class":`dim wp-empty`,});empty; }
  }
   { svelteHTML.createElement("div", {    "class":`wp-rows`,...__sveltets_2_empty({"data-section":section}),[Symbol("@attach")]:lw.attach,});
    if(win.top > 0){ { svelteHTML.createElement("div", {     "class":`wp-spacer`,"style":`height:${win.top}px`,"aria-hidden":`true`,}); }}
       for(let r of __sveltets_2_ensureArray((rows.slice(win.start, win.end)))){r.key;
      if(r.node.type === 'folder'){
        ;__sveltets_2_ensureSnippet(folderRow(r.node, r.depth, section));
      }else{
        ;__sveltets_2_ensureSnippet(fileRow(r.node, r.depth, section));
      }
    }
    if(win.bottom > 0){ { svelteHTML.createElement("div", {     "class":`wp-spacer`,"style":`height:${win.bottom}px`,"aria-hidden":`true`,}); }}
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, status, onstatus, oncommitted, onclose, onresolve }: Props = $props();

  // Conflicted files get their OWN section with resolve actions — they must
  // not sit in the stage/unstage trees, where the checkbox would `git add`
  // a file that still contains conflict markers and silently mark it resolved.
  const conflicted = $derived(status.changes.filter((c) => c.kind === 'conflicted'));
  // A partially staged path (`MM`) is ONE row with both flags and belongs in
  // BOTH lists — filtering on `!staged` hid its unstaged half, so "Stage all"
  // skipped it and the commit silently left those edits out.
  const unstaged = $derived(status.changes.filter((c) => c.unstaged && c.kind !== 'conflicted'));
  const staged = $derived(status.changes.filter((c) => c.staged && c.kind !== 'conflicted'));
  // The daemon caps untracked rows (a non-ignored build dir can hold 200k):
  // say how many were left out instead of pretending the list is complete.
  const hiddenUntracked = $derived(
    status.untracked_truncated && status.untracked_total != null
      ? Math.max(0, status.untracked_total - status.changes.filter((c) => c.kind === 'untracked').length)
      : 0,
  );

  /** Partially staged: porcelain `MM` — the index and the worktree BOTH differ.
   *  One `FileChange` carries both flags (`parse.rs` emits a single row per
   *  path), so this is a flag test, not a two-tree intersection. */
  const partial = $derived(
    new Set(
      status.changes
        .filter((c) => c.staged && c.unstaged && c.kind !== 'conflicted')
        .map((c) => c.path),
    ),
  );

  /** Resolve one conflicted file by taking a whole side (`git checkout
   *  --ours/--theirs` + stage). Confirmed — it discards the other side. */
  async function takeSide(path: string, side: 'ours' | 'theirs'): Promise<void> {
    const ok = await confirmer.ask(
      `Resolve ${path} by keeping ${side === 'ours' ? 'YOUR version (ours)' : 'THEIR version (theirs)'}? If that side deleted the file, it will be deleted. The other side's changes are discarded.`,
      { title: `Take ${side}`, confirmLabel: `Take ${side}` },
    );
    if (!ok) return;
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/conflict/resolve`, {
        path,
        content: '',
        side,
      });
      onstatus(s);
      toasts.success('Resolved', `${path} — kept ${side}`);
    } catch (e) {
      toastError('Couldn’t resolve the conflict', e);
    }
  }

  // ── Commit composer (Summary + Description, joined "subject\n\nbody") ──────
  let subject = $state('');
  let body = $state('');
  let amend = $state(false);
  /** Sign this commit. Seeded from the repo's `commit.gpgsign`, then per-commit
   *  — an explicit `sign` overrides the config without writing it. */
  let signOn = $state(false);
  let signCfg = $state<CommitConfig | null>(null);
  let committing = $state(false);
  let drafting = $state(false);
  /** Same watch-the-agent affordance as the PR dialog: drafting runs as a REAL
   *  `commit-draft` session (a background source — not listed under Agents),
   *  so it can be embedded here and watched while the spinner runs. The id
   *  arrives with the POST at the END of the turn; until then the freshest
   *  commit-draft session created since the draft began is the live one. */
  let draftSessionId = $state<string | null>(null);
  /** When the agent's message landed — drives the byline above the fields. */
  let draftedAt = $state<number | null>(null);
  let draftStartedAt = $state<string | null>(null);
  let draftElapsed = $state(0);
  let showDraftTerm = $state(false);
  const liveDraftId: string | null = $derived.by(() => {
    if (draftSessionId) return draftSessionId;
    if (!drafting || !draftStartedAt) return null;
    const c = ws.sessions.filter(
      (s) => (s.meta as { source?: string } | null)?.source === 'commit-draft' && !s.archived && s.created_at >= draftStartedAt!,
    );
    return c.length > 0 ? c[c.length - 1].id : null;
  });

  // The composer outlives the panel (commitComposer.ts): restore this repo's
  // summary / description, adopt a draft that finished while we were away,
  // and resume the spinner on one still running. Declared before the
  // `wipRequest` effect so a "Commit with message…" prefill still wins.
  let composerRepo = '';
  /** False once unmounted: a draft landing after that must not reach the
   *  confirmer (its ask() supersedes — cancels — whatever dialog is open) or
   *  toast; the commitComposer parking slot delivers it to the next panel. */
  let alive = true;
  onDestroy(() => (alive = false));
  $effect(() => {
    const repo = repoId;
    const unwatch = untrack(() => {
      composerRepo = repo;
      const saved = readComposer(repo);
      subject = saved.subject;
      body = saved.body;
      draftedAt = saved.draftedAt;
      const stop = watchComposer(repo);
      const parked = takePendingDraft(repo);
      if (parked) void applyDraft(parked);
      const running = draftInFlight(repo);
      if (running && !drafting) void followDraft(running, repo);
      return stop;
    });
    return unwatch;
  });
  $effect(() => {
    const text = { subject, body, draftedAt };
    untrack(() => {
      if (composerRepo) writeComposer(composerRepo, text);
    });
  });

  // The draft endpoint reads the staged diff (falling back to the full working
  // diff) — neither includes untracked files, so require something draftable.
  const draftable = $derived(status.changes.some((c) => c.staged || c.kind !== 'untracked'));

  // ── Per-file diff (working tree, server-side scoped) ────────────────────────
  let selectedPath = $state<string | null>(null);
  // Replaced wholesale, never mutated — `$state.raw` skips deep-proxying hunks.
  let diff = $state.raw<DiffResp | null>(null);
  let diffLoading = $state(false);
  /** Non-null when the diff LOAD failed — rendered as an error, not as the
   *  "no textual diff" empty state (an error masquerading as emptiness). */
  let diffError = $state<string | null>(null);
  /** Bumped by the diff error's Retry; the select effect reads it. */
  let diffRetry = $state(0);

  /** Which side to show when a path is staged AND unstaged (default unstaged). */
  let stagedView = $state(false);
  /**
   * Hunk actions rebuild their patch from the server's own `git diff` /
   * `git diff --cached`, so the diff on screen must be exactly one of those or
   * the hunk indices don't line up. `working` (staged + unstaged vs HEAD, plus
   * synthesised untracked diffs) is neither — untracked files keep it and
   * simply get no hunk buttons.
   */
  const selTarget: 'working' | 'worktree' | 'staged' = $derived.by(() => {
    const p = selectedPath;
    if (p === null) return 'working';
    const ch = status.changes.find((c) => c.path === p);
    // Untracked has no index/worktree pair to diff, conflicted is the
    // resolver's business — both keep `working` and get no hunk buttons.
    if (!ch || ch.kind === 'untracked' || ch.kind === 'conflicted') return 'working';
    if (ch.staged && ch.unstaged) return stagedView ? 'staged' : 'worktree';
    if (ch.unstaged) return 'worktree';
    if (ch.staged) return 'staged';
    return 'working';
  });

  $effect(() => {
    const path = selectedPath;
    const target = selTarget;
    void diffRetry;
    if (path === null) {
      diff = null;
      diffError = null;
      diffLoading = false;
      return;
    }
    // Drop the previous file's diff at once: its hunk buttons must never act
    // while the header already names the next file.
    diff = null;
    diffLoading = true;
    diffError = null;
    // A newer selection aborts this fetch (effect cleanup) and the stale
    // guard drops a response that still lands — file A's slow diff used to
    // overwrite the diff of file B selected after it.
    const ctl = new AbortController();
    const stale = () => ctl.signal.aborted;
    void api
      .get<DiffResp>(`/repos/${repoId}/diff?target=${target}&path=${encodeURIComponent(path)}`, ctl.signal)
      .then((d) => {
        if (!stale()) diff = d;
      })
      .catch((e) => {
        if (stale() || isAbortError(e)) return;
        diff = { files: [] };
        diffError = loadErrorText(e);
      })
      .finally(() => {
        if (!stale()) diffLoading = false;
      });
    return () => ctl.abort();
  });
  // A live on-disk change (`repo_status_changed`) re-reads the open file's
  // diff in place: the change list can stay identical (the file was already
  // modified) while its content moved on. No blanking — the old diff stays
  // until the new one lands. Only when the change touched THIS file (the
  // event's `paths`; unknown → yes), and only while the window is visible —
  // a visible-but-unfocused window (the diff beside an editor) refreshes
  // live too; a hidden one marks the diff stale and catches up once when it
  // shows again instead of re-reading per save.
  let diffLiveStale = false;
  const diffLiveAllowed = (): boolean => typeof document === 'undefined' || !document.hidden;
  function refetchOpenDiff(path: string, target: typeof selTarget): () => void {
    const ctl = new AbortController();
    void api
      .get<DiffResp>(`/repos/${repoId}/diff?target=${target}&path=${encodeURIComponent(path)}`, ctl.signal)
      .then((d) => {
        if (!ctl.signal.aborted && selectedPath === path && selTarget === target) {
          diff = d;
          diffError = null;
        }
      })
      .catch(() => {
        /* keep the diff on screen; the next change or a reselect retries */
      });
    return () => ctl.abort();
  }
  $effect(() => {
    const rev = git.liveRev[repoId] ?? 0;
    if (rev === 0) return;
    const path = untrack(() => selectedPath);
    const target = untrack(() => selTarget);
    if (path === null || !untrack(() => git.liveTouches(repoId, path))) return;
    if (!diffLiveAllowed()) {
      diffLiveStale = true;
      return;
    }
    diffLiveStale = false;
    return refetchOpenDiff(path, target);
  });
  $effect(() => {
    // Picking another file loads it fresh, so a pending catch-up is moot.
    void selectedPath;
    diffLiveStale = false;
  });
  $effect(() => {
    let cancel: (() => void) | null = null;
    const catchUp = (): void => {
      if (!diffLiveStale || !diffLiveAllowed()) return;
      diffLiveStale = false;
      const path = selectedPath;
      if (path === null) return;
      cancel?.();
      cancel = refetchOpenDiff(path, selTarget);
    };
    window.addEventListener('focus', catchUp);
    document.addEventListener('visibilitychange', catchUp);
    return () => {
      window.removeEventListener('focus', catchUp);
      document.removeEventListener('visibilitychange', catchUp);
      cancel?.();
    };
  });
  /** Over-cap files ("Load anyway") re-fetch through the same target. */
  const loadWipFile = $derived(repoDiffFileLoader(repoId, selTarget));

  // Agent UI control (lib/uiCommands/git.ts): a pending WIP request for this
  // repo selects a file's diff and/or prefills the composer — so the user sees
  // what the agent is looking at, and the message it is about to commit.
  $effect(() => {
    const r = git.wipRequest;
    if (!r || r.repoId !== repoId) return;
    untrack(() => {
      const req = git.takeWipRequest(repoId);
      if (!req) return;
      if (req.path !== undefined) {
        stagedView = req.staged ?? false;
        selectedPath = req.path;
      }
      if (req.subject !== undefined) subject = req.subject;
      if (req.body !== undefined) body = req.body;
    });
  });

  // A staged/unstaged move can remove the selected file from the tree entirely
  // (e.g. discard); drop the diff so it doesn't show a stale file.
  $effect(() => {
    if (selectedPath !== null && !status.changes.some((c) => c.path === selectedPath)) {
      selectedPath = null;
    }
  });

  const kindBadge: Record<FileChange['kind'], string> = {
    modified: 'M',
    added: 'A',
    deleted: 'D',
    renamed: 'R',
    untracked: 'U',
    conflicted: '!',
  };

  let panelEl = $state<HTMLDivElement | null>(null);
  let announcement = $state('');
  const diffId = `wp-diff-${Math.random().toString(36).slice(2, 10)}`;
  const rowsOf = (section: 'unstaged' | 'staged'): Row[] => (section === 'unstaged' ? unstagedRows : stagedRows);
  const indexOf = (origin: Origin | undefined): number =>
    origin ? Math.max(0, rowsOf(origin.section).findIndex((r) => r.key === origin.key)) : -1;

  async function settleFocus(origin: Origin | undefined, before: number, message: string): Promise<void> {
    announcement = '';
    await tick();
    announcement = message;
    if (!origin) return;
    const rows = rowsOf(origin.section);
    const head = panelEl?.querySelector<HTMLElement>(`[data-wp-head="${origin.section}"]`);
    if (rows.length === 0) {
      head?.focus();
      return;
    }
    const key = rows[Math.min(before, rows.length - 1)].key;
    const list = panelEl?.querySelector(`.wp-rows[data-section="${origin.section}"]`);
    const boxes = Array.from(list?.querySelectorAll<HTMLInputElement>('input[type="checkbox"][data-row-key]') ?? []);
    (boxes.find((b) => b.dataset.rowKey === key) ?? boxes.at(-1) ?? head)?.focus();
  }

  const pathsLabel = (paths: string[]): string => (paths.length === 1 ? paths[0] : plural(paths.length, 'file'));

  async function stagePaths(paths: string[], stage: boolean, origin?: Origin): Promise<void> {
    if (paths.length === 0) return;
    const before = indexOf(origin);
    try {
      const s = await api.post<RepoStatusResp>(
        `/repos/${repoId}/${stage ? 'stage' : 'unstage'}`,
        { paths },
      );
      onstatus(s);
      await settleFocus(origin, before, `${stage ? 'Staged' : 'Unstaged'} ${pathsLabel(paths)}`);
    } catch (e) {
      toastError('Couldn’t finish the operation', e);
    }
  }

  /** Discard `paths`. From the Unstaged list (`section: 'unstaged'`) only the
   *  unstaged side goes (`keep_staged`): staged hunks of a partially staged
   *  file survive. From the Staged list the whole change is reverted to HEAD. */
  async function discardPaths(
    paths: string[],
    label: string,
    section: 'unstaged' | 'staged' = 'staged',
    origin?: Origin,
  ): Promise<void> {
    if (paths.length === 0) return;
    const before = indexOf(origin);
    const keepStaged = section === 'unstaged';
    const them = paths.length === 1 ? 'it' : 'them';
    const ok = await confirmer.ask(
      keepStaged
        ? `Discard unstaged changes to ${label}? This reverts ${them} to the staged version (or the last commit) — staged changes are kept, untracked files are deleted — and cannot be undone.`
        : `Discard changes to ${label}? This reverts ${them} to the last commit (new files are deleted) and cannot be undone.`,
      { title: keepStaged ? 'Discard unstaged changes' : 'Discard changes', confirmLabel: 'Discard', danger: true },
    );
    if (!ok) return;
    try {
      const req: DiscardReq = keepStaged ? { paths, keep_staged: true } : { paths };
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/discard`, req);
      onstatus(s);
      // Trust the fresh status, not the 200: a discard that left a file
      // changed must never toast "Discarded". (Unstaged discard: a path may
      // stay listed for its STAGED half — only an unstaged remainder counts.)
      const left = paths.filter((p) =>
        s.changes.some((c) => c.path === p && (!keepStaged || c.unstaged)),
      );
      await settleFocus(
        origin,
        before,
        left.length > 0 ? `Discard incomplete for ${pathsLabel(left)}` : `Discarded ${pathsLabel(paths)}`,
      );
      if (left.length > 0) {
        toasts.error(
          'Discard incomplete',
          `${left.length} file${left.length === 1 ? ' still has' : 's still have'} changes: ${left.slice(0, 3).join(', ')}${left.length > 3 ? '…' : ''}`,
        );
      } else {
        toasts.info(`Discarded ${plural(paths.length, 'file')}`);
      }
    } catch (e) {
      toastError('Couldn’t discard the changes', e);
    }
  }

  function fileMenu(e: MouseEvent, c: FileChange, section: 'unstaged' | 'staged'): void {
    e.preventDefault();
    const inStaged = section === 'staged';
    ctxMenu.show(e, [
      {
        label: inStaged ? 'Unstage' : 'Stage',
        action: () => void stagePaths([c.path], !inStaged),
      },
      { separator: true },
      {
        label: inStaged ? 'Discard…' : 'Discard unstaged changes…',
        icon: 'trash',
        danger: true,
        action: () => void discardPaths([c.path], c.path, section),
      },
    ]);
  }

  /** Right-click a folder row: the same two actions a file row offers, applied
   *  to every file under it (recursively — `files` carries the whole subtree). */
  function folderMenu(
    e: MouseEvent,
    node: TFolder,
    section: 'unstaged' | 'staged',
  ): void {
    e.preventDefault();
    const paths = node.files.map((f) => f.path);
    const n = `${plural(paths.length, 'file')}`;
    ctxMenu.show(e, [
      {
        label: `${section === 'staged' ? 'Unstage' : 'Stage'} ${node.name}/ (${n})`,
        action: () => void stagePaths(paths, section === 'unstaged'),
      },
      { separator: true },
      {
        label: `Discard ${node.name}/ (${n})…`,
        icon: 'trash',
        danger: true,
        action: () => void discardPaths(paths, `${node.path}/ (${n})`, section),
      },
    ]);
  }

  type TFolder = {
    type: 'folder';
    name: string;
    path: string;
    children: TNode[];
    files: FileChange[];
  };
  type TNode = TFile | TFolder;

  // One collator for every compare: `localeCompare` re-resolves the locale
  // per call, which dominated sorting a 20k-file tree.
  const collator = new Intl.Collator();
  function sortLevel(nodes: TNode[]): void {
    nodes.sort((a, b) => {
      if (a.type !== b.type) return a.type === 'folder' ? -1 : 1;
      return collator.compare(a.name, b.name);
    });
    for (const n of nodes) if (n.type === 'folder') sortLevel(n.children);
  }

  function buildTree(changes: FileChange[]): TNode[] {
    const rootChildren: TNode[] = [];
    const folders = new Map<string, TFolder>();
    for (const c of changes) {
      const parts = c.path.split('/');
      let children = rootChildren;
      let prefix = '';
      for (let i = 0; i < parts.length - 1; i++) {
        prefix = prefix ? `${prefix}/${parts[i]}` : parts[i];
        let folder = folders.get(prefix);
        if (!folder) {
          folder = { type: 'folder', name: parts[i], path: prefix, children: [], files: [] };
          folders.set(prefix, folder);
          children.push(folder);
        }
        folder.files.push(c);
        children = folder.children;
      }
      children.push({ type: 'file', name: parts[parts.length - 1], change: c });
    }
    sortLevel(rootChildren);
    return rootChildren;
  }

  const unstagedTree = $derived.by(() => buildTree(unstaged));
  const stagedTree = $derived.by(() => buildTree(staged));

  // Folders fold by "section:path" so the same folder can fold independently
  // in the Unstaged and Staged trees. A folder the user never toggled starts
  // collapsed when its subtree holds more than AUTO_COLLAPSE_FILES changes —
  // an un-ignored `node_modules/` or `dist/` lists tens of thousands of files
  // (20k rows ≈ 200k DOM nodes ≈ 1.5 s to mount); its count stays visible.
  const AUTO_COLLAPSE_FILES = 200;
  let folds = $state.raw<Map<string, boolean>>(new Map());
  function isCollapsed(key: string, node: TFolder): boolean {
    return folds.get(key) ?? node.files.length > AUTO_COLLAPSE_FILES;
  }
  function toggleFolder(key: string, node: TFolder): void {
    const next = new Map(folds);
    next.set(key, !isCollapsed(key, node));
    folds = next;
  }

  /** The tree as the flat list of VISIBLE rows (collapsed subtrees skipped),
   *  so each section mounts a bounded slice instead of recursing everything. */
  type Row = { node: TNode; depth: number; key: string };
  function visibleRows(nodes: TNode[], section: 'unstaged' | 'staged'): Row[] {
    const out: Row[] = [];
    const walk = (level: TNode[], depth: number) => {
      for (const n of level) {
        if (n.type === 'folder') {
          out.push({ node: n, depth, key: `d:${n.path}` });
          if (!isCollapsed(`${section}:${n.path}`, n)) walk(n.children, depth + 1);
        } else {
          out.push({ node: n, depth, key: `f:${n.change.path}` });
        }
      }
    };
    walk(nodes, 0);
    return out;
  }
  const unstagedRows = $derived(visibleRows(unstagedTree, 'unstaged'));
  const stagedRows = $derived(visibleRows(stagedTree, 'staged'));
  // Each section is WINDOWED against the shared `.wp-scroll` scroller: only
  // the rows in view (+ overscan) mount, between spacers sized to the rest, so
  // a 20k-file folder costs what a 50-file one does; ⌘F searches every row.
  const unstagedWin = new ListWindow('.wp-file, .wp-folder');
  const stagedWin = new ListWindow('.wp-file, .wp-folder');
  $effect(() => unstagedWin.findRows(() => null, () => unstagedRows, (r) => r.node.name));
  $effect(() => stagedWin.findRows(() => null, () => stagedRows, (r) => r.node.name));
  const winOf = (section: 'unstaged' | 'staged') => (section === 'unstaged' ? unstagedWin : stagedWin);

  // Section collapse (GitKraken keeps both open; folding is still handy).
  let unstagedOpen = $state(true);
  let stagedOpen = $state(true);

  // The staged list sits BELOW the unstaged one: when the unstaged rows (or a
  // section fold) change, its offset in the scroller moves without a scroll.
  $effect(() => {
    void unstagedRows.length;
    void stagedRows.length;
    void unstagedOpen;
    void stagedOpen;
    unstagedWin.refresh();
    stagedWin.refresh();
  });

  // Repo signing defaults. A failed read must not look like "this repo doesn't
  // sign": the toggle stays usable (per-commit override), but the row says the
  // default couldn't be read and offers Retry. A response for a repo the panel
  // has since left is dropped.
  let signCfgError = $state(false);
  let signCfgLoading = $state(false);
  let signCfgSeq = 0;
  function loadSignCfg(id: string): void {
    const seq = ++signCfgSeq;
    signCfgError = false;
    signCfgLoading = true;
    void api
      .get<CommitConfig>(`/repos/${id}/commit-config`)
      .then((c) => {
        if (seq !== signCfgSeq) return;
        signCfg = c;
        signOn = c.gpgsign;
      })
      .catch(() => {
        if (seq !== signCfgSeq) return;
        signCfgError = true;
        announce('Couldn’t read signing settings.');
      })
      .finally(() => {
        if (seq === signCfgSeq) signCfgLoading = false;
      });
  }
  $effect(() => {
    const id = repoId;
    untrack(() => loadSignCfg(id));
    return () => {
      signCfgSeq++;
    };
  });

  // Amend: HEAD's subject is shown as the PLACEHOLDER, never copied into the
  // field. An empty subject commits `--amend --no-edit`, which keeps the WHOLE
  // previous message — copying only the subject in (the log has no body) and
  // committing it replaced the message and silently dropped its description.
  let amendSubject = $state('');
  $effect(() => {
    if (!amend) return;
    const id = repoId;
    amendSubject = '';
    void api
      .get<CommitInfo[]>(`/repos/${id}/log?limit=1&skip=0`)
      .then((commits) => {
        if (amend && id === repoId) amendSubject = commits[0]?.subject ?? '';
      })
      .catch(() => {});
  });
  // Which remote refs already hold HEAD — asked only while Amend is ticked
  // (a `--contains` walk is too costly per status poll), and again when HEAD
  // may have moved (branch / ahead count change). null = unknown.
  let headRemotes = $state.raw<string[] | null>(null);
  $effect(() => {
    if (!amend) return;
    const id = repoId;
    void status.branch;
    void status.ahead;
    headRemotes = null;
    let live = true;
    void api
      .get<HeadRemotesResp>(`/repos/${id}/head/remotes`)
      .then((r) => {
        if (live && id === repoId) headRemotes = r.remotes;
      })
      .catch(() => {});
    return () => { live = false; };
  });
  /** HEAD is already published: amending rewrites a pushed commit, and the
   *  next push needs a force (with lease). Said BEFORE the user commits. */
  const amendPublished = $derived(amend ? amendPublishedAt(status.upstream, status.ahead, headRemotes) : null);

  const canCommit = $derived(
    !committing && !drafting && (subject.trim() !== '' || amend) && (staged.length > 0 || amend),
  );
  /** Why Commit is grey — a disabled button with no reason reads as broken
   *  (a draft works from UNSTAGED changes, so a good message can exist). */
  const commitHint = $derived(
    committing || drafting || canCommit
      ? ''
      : staged.length === 0 && !amend
        ? unstaged.length > 0
          ? 'Stage files to commit'
          : 'Nothing to commit'
        : subject.trim() === '' && !amend
          ? 'Write a summary to commit'
          : '',
  );
  /** ⌘/Ctrl+Enter in the summary or description commits — the standard
   *  composer shortcut; plain Enter keeps its meaning (newline / nothing). */
  function commitKey(e: KeyboardEvent): void {
    if (e.key !== 'Enter' || !(e.metaKey || e.ctrlKey) || e.isComposing) return;
    e.preventDefault();
    if (canCommit) void commit();
  }

  async function draftMessage(): Promise<void> {
    if (drafting || committing) return;
    const repo = repoId;
    const p = startDraft(repo, () =>
      api.post<DraftCommitMessageResp>(`/repos/${repo}/draft-commit-message`, {}),
    );
    await followDraft(p, repo);
  }

  /** Watch a draft request (started here, or before a remount) to the end. */
  async function followDraft(p: Promise<DraftCommitMessageResp>, repo: string): Promise<void> {
    drafting = true;
    draftSessionId = null;
    draftElapsed = 0;
    // Slight backdate so a session created in the same second still matches.
    draftStartedAt = new Date(Date.now() - 2000).toISOString();
    const tick = setInterval(() => (draftElapsed += 1), 1000);
    try {
      const d = await p;
      // Unmounted, or switched repos meanwhile: the reply was parked.
      if (!alive || repo !== repoId) return;
      await applyDraft(d);
    } catch (e) {
      if (alive && repo === repoId) toastError('Couldn’t draft the message', e);
    } finally {
      clearInterval(tick);
      drafting = false;
    }
  }

  /** Put an agent draft into the fields — asking first over typed text. */
  async function applyDraft(d: DraftCommitMessageResp): Promise<void> {
    if (!alive) return;
    const text = d.message.trim();
    // Never overwrite what the person already typed without asking.
    if (subject.trim() || body.trim()) {
      const ok = await confirmer.ask(
        'The agent’s message will replace the summary and description you have typed.',
        { title: 'Replace your title and description?', confirmLabel: 'Replace', cancelLabel: 'Keep mine', danger: false },
      );
      if (!alive) return;
      if (!ok) {
        draftSessionId = d.session_id ?? null;
        draftedAt = null;
        return;
      }
    }
    draftedAt = Date.now();
    const nl = text.indexOf('\n');
    if (nl === -1) {
      subject = text;
      body = '';
    } else {
      subject = text.slice(0, nl).trim();
      body = text.slice(nl + 1).replace(/^\s*\n/, '').trimEnd();
    }
    draftSessionId = d.session_id ?? null;
    toasts.info(
      'Draft ready',
      d.from_staged
        ? 'From staged changes — review the summary & description.'
        : 'From working changes (nothing staged) — review & edit.',
    );
  }

  async function commit(): Promise<void> {
    if (committing) return;
    committing = true;
    try {
      const message = subject.trim() + (body.trim() ? `\n\n${body.trim()}` : '');
      const r = await git.commit(repoId, { message, amend, sign: signOn });
      toasts.success('Committed', r.sha.slice(0, 8));
      subject = '';
      body = '';
      draftedAt = null;
      amend = false;
      onstatus(r.status);
      selectedPath = null;
      oncommitted();
    } catch (e) {
      toastError('Couldn’t commit the changes', e);
    } finally {
      committing = false;
    }
  }
;
async () => {







 { const $$_div0 = svelteHTML.createElement("div", {  "class":`wip-panel`,});panelEl = $$_div0;
   { svelteHTML.createElement("div", {     "class":`sr-only`,"role":`status`,"aria-live":`polite`,});announcement; }
   { svelteHTML.createElement("div", { "class":`wp-head`,});
     { svelteHTML.createElement("span", { "class":`wp-title mono`,});  }
     { svelteHTML.createElement("span", { "class":`wp-count`,});plural(status.changes.length, 'file');  }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {       "class":`icon-btn wp-close`,"onclick":onclose,"title":`Close WIP panel`,"aria-label":`Close WIP panel`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} }
   }

   { svelteHTML.createElement("div", {  "class":`wp-scroll`,});selectedPath !== null;
    
    if(conflicted.length > 0){
       { svelteHTML.createElement("div", { "class":`wp-section wp-conflicts`,});
         { svelteHTML.createElement("div", { "class":`wp-sec-head wp-conflict-head`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":12,}});}
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", { "class":`wp-sec-count`,});conflicted.length; }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
          if(onresolve){
             { svelteHTML.createElement("button", {       "type":`button`,"class":`btn ghost small wp-sec-action`,"onclick":(e) => {
                e.stopPropagation();
                onresolve?.();
              },});  }
          }
         }
         { svelteHTML.createElement("div", { "class":`wp-list`,});
             for(let c of __sveltets_2_ensureArray(conflicted)){c.path;
             { svelteHTML.createElement("div", { "class":`wp-file wp-conflict-row`,});
               { svelteHTML.createElement("span", { "class":`kind k-conflicted`,});  }
               { svelteHTML.createElement("button", {       "class":`wp-name`,"onclick":() => onresolve?.(),"title":`Resolve ${c.path} in the conflict resolver`,});
                 { svelteHTML.createElement("span", { "class":`mono wp-fname`,});c.path; }
               }
               { svelteHTML.createElement("button", {     "class":`wp-side`,"title":`Keep your version (ours)`,"onclick":() => void takeSide(c.path, 'ours'),});  }
               { svelteHTML.createElement("button", {     "class":`wp-side`,"title":`Keep their version (theirs)`,"onclick":() => void takeSide(c.path, 'theirs'),});  }
             }
          }
         }
       }
    }

    
     { svelteHTML.createElement("div", { "class":`wp-section`,});
       { svelteHTML.createElement("div", { "class":`wp-sec-head`,});
         { svelteHTML.createElement("button", {         "type":`button`,"class":`wp-sec-toggle`,...__sveltets_2_empty({"data-wp-head":`unstaged`}),"onclick":() => (unstagedOpen = !unstagedOpen),"aria-expanded":unstagedOpen,});
           { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":unstagedOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", { "class":`wp-sec-count`,});unstaged.length; }
         }
        if(unstaged.length > 0){
           { svelteHTML.createElement("button", {       "type":`button`,"class":`btn ghost small wp-sec-action`,"onclick":(e) => {
              e.stopPropagation();
              void stagePaths(unstaged.map((c) => c.path), true);
            },});  }
           { svelteHTML.createElement("button", {         "type":`button`,"class":`btn ghost small wp-sec-action danger`,"title":`Discard changes to all unstaged files`,"onclick":(e) => {
              e.stopPropagation();
              void discardPaths(unstaged.map((c) => c.path), 'all unstaged files', 'unstaged');
            },});  }
        }
       }
      if(unstagedOpen){
         { svelteHTML.createElement("div", { "class":`wp-list`,});
          ;__sveltets_2_ensureSnippet(sectionRows(unstagedRows, 'unstaged', 'Nothing unstaged.'));
         }
        if(hiddenUntracked > 0){
           { svelteHTML.createElement("p", {   "class":`wp-untracked-cap`,"role":`note`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}
             { svelteHTML.createElement("span", { });hiddenUntracked.toLocaleString();   hiddenUntracked === 1 ? 'file' : 'files';    
                      { svelteHTML.createElement("code", {});  }  }
           }
        }
      }
     }

    
     { svelteHTML.createElement("div", { "class":`wp-section`,});
       { svelteHTML.createElement("div", { "class":`wp-sec-head`,});
         { svelteHTML.createElement("button", {         "type":`button`,"class":`wp-sec-toggle`,...__sveltets_2_empty({"data-wp-head":`staged`}),"onclick":() => (stagedOpen = !stagedOpen),"aria-expanded":stagedOpen,});
           { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":stagedOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", { "class":`wp-sec-count`,});staged.length; }
         }
        if(staged.length > 0){
           { svelteHTML.createElement("button", {       "type":`button`,"class":`btn ghost small wp-sec-action`,"onclick":(e) => {
              e.stopPropagation();
              void stagePaths(staged.map((c) => c.path), false);
            },});  }
           { svelteHTML.createElement("button", {         "type":`button`,"class":`btn ghost small wp-sec-action danger`,"title":`Discard changes to all staged files`,"onclick":(e) => {
              e.stopPropagation();
              void discardPaths(staged.map((c) => c.path), 'all staged files');
            },});  }
        }
       }
      if(stagedOpen){
         { svelteHTML.createElement("div", { "class":`wp-list`,});
          ;__sveltets_2_ensureSnippet(sectionRows(stagedRows, 'staged', 'Nothing staged yet.'));
         }
      }
     }

   }

  
  if(selectedPath !== null){
     { svelteHTML.createElement("div", {   "class":`wp-diff`,"id":diffId,});
       { svelteHTML.createElement("div", { "class":`wp-diff-head`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
         { svelteHTML.createElement("span", {   "class":`mono wp-diff-path`,"title":selectedPath,});selectedPath; }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(partial.has(selectedPath)){
          
           { svelteHTML.createElement("div", {     "class":`segmented wp-target`,"role":`group`,"aria-label":`Diff side`,});
             { svelteHTML.createElement("button", {     "aria-pressed":!stagedView,"onclick":() => (stagedView = false),});!stagedView;  }
             { svelteHTML.createElement("button", {     "aria-pressed":stagedView,"onclick":() => (stagedView = true),});stagedView;  }
           }
        }
         { svelteHTML.createElement("button", {       "class":`icon-btn wp-close`,"onclick":() => (selectedPath = null),"title":`Close diff`,"aria-label":`Close diff`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} }
       }
       { svelteHTML.createElement("div", { "class":`wp-diff-body`,});
        if(diffLoading && !diff){
           { svelteHTML.createElement("div", { "style":`padding: 10px`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {    "rows":5,"height":20,}});} }
        } else if (diff && diff.files.length > 0){
           { const $$_reweiVffiD3C = __sveltets_2_ensureComponent(DiffViewer); new $$_reweiVffiD3C({ target: __sveltets_2_any(), props: {       diff,repoId,"loadFile":loadWipFile,"wip":selTarget === 'working'
              ? undefined
              : {
                  target: selTarget,
                  onapplied: (r) => {
                    onstatus(r.status);
                    diff = r.diff;
                    if (r.backup_stash)
                      toasts.info('Backup stash kept', r.backup_stash.slice(0, 8));
                  },
                },}});}
        } else if (diffError){
           { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`this file’s diff`,"error":diffError,"empty":true,"onretry":() => diffRetry++,}});}
        }else{
           { svelteHTML.createElement("div", { "class":`dim wp-empty`,});      }
        }
       }
     }
  }

  
   { svelteHTML.createElement("div", { "class":`wp-composer`,});
     { svelteHTML.createElement("div", { "class":`msg-box`,});
       { svelteHTML.createElement("input", {              "dir":`auto`,"class":`input subject-input`,"bind:value":subject,"placeholder":amend
          ? amendSubject
            ? `Keep “${amendSubject}” — or type a new message`
            : 'Keep the previous message — or type a new one'
          : 'Summary',"aria-label":`Commit summary`,"spellcheck":`false`,"onkeydown":commitKey,});/*Ωignore_startΩ*/() => subject = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {         "class":`btn small ghost draft-btn`,"disabled":drafting || committing || !draftable,"onclick":draftMessage,"title":draftable
          ? 'Draft a summary + description from your staged changes'
          : 'Stage a file first — drafting can’t see untracked files',});
        if(drafting){
           { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); } draftElapsed > 0 ? ` ${draftElapsed}s` : '';
        }else{
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});} }
       }
      if(liveDraftId){
         { svelteHTML.createElement("button", {       "class":`btn small ghost draft-btn`,"onclick":() => (showDraftTerm = !showDraftTerm),"title":showDraftTerm ? 'Hide the drafting agent' : 'Watch the drafting agent live',});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":showDraftTerm ? 'chevronUp' : 'terminal',"size":12,}});}
          showDraftTerm ? 'Hide agent' : 'Watch agent';
         }
      }
     }
    if(draftedAt && (subject.trim() || body.trim())){
       { svelteHTML.createElement("div", { "class":`draft-by`,}); { const $$_enilyBtnegA3C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA3C({ target: __sveltets_2_any(), props: {    "label":`Draft`,"at":draftedAt,}});} }
    }
    if(liveDraftId && showDraftTerm){
       { svelteHTML.createElement("div", { "class":`draft-term`,});
         { const $$_lanimreTyzaL3C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL3C({ target: __sveltets_2_any(), props: {    "sessionId":liveDraftId,"preferDom":true,"showToolbar":false,}});}
       }
    }
     { svelteHTML.createElement("textarea", {                "dir":`auto`,"class":`input body-input`,"rows":2,"bind:value":body,"placeholder":`Why this change, in a sentence or two`,"aria-label":`Commit description (optional)`,"spellcheck":`false`,"onkeydown":commitKey,});/*Ωignore_startΩ*/() => body = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
    if(amendPublished){
       { svelteHTML.createElement("div", {   "class":`amend-warn`,"role":`note`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
         { svelteHTML.createElement("span", {});     amendPublished;               }
       }
    }
     { svelteHTML.createElement("div", { "class":`wp-action-row`,});
       { svelteHTML.createElement("label", { "class":`checkbox-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":amend,});/*Ωignore_startΩ*/() => amend = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        
       }
       { svelteHTML.createElement("label", {     "class":`checkbox-row`,"title":signCfg?.signing_key
          ? `Sign with ${signCfg.format ?? 'gpg'} key ${signCfg.signing_key}`
          : 'Sign this commit',});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":signOn,});/*Ωignore_startΩ*/() => signOn = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        
       }
      if(signCfgError){
         { svelteHTML.createElement("span", { "class":`sign-err`,});
              
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => loadSignCfg(repoId),"disabled":signCfgLoading,});signCfgLoading ? 'Retrying…' : 'Retry'; }
         }
      }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(commitHint){ { svelteHTML.createElement("span", {   "id":`wp-commit-hint`,"class":`wp-commit-hint dim`,});commitHint; }}
       { svelteHTML.createElement("button", {           "class":`btn primary`,"disabled":!canCommit,"onclick":commit,"title":commitHint || 'Commit (⌘↵)',"aria-describedby":commitHint ? 'wp-commit-hint' : undefined,});
        committing ? 'Committing…' : `${amend ? 'Amend' : 'Commit'}${staged.length > 0 ? ` (${staged.length})` : ''}`;
       }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WipPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WipPanel__SvelteComponent_ = ReturnType<typeof WipPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default WipPanel__SvelteComponent_;