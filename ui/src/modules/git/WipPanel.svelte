<script lang="ts">
  import { announce } from '../../lib/announce.svelte';
  import { plural } from '../../lib/plural';
  // The working-tree "changes" panel: shown in the graph's RIGHT detail pane
  // when the WIP row is selected. Conflicts / Unstaged / Untracked / Staged
  // sections (wip/WipSection.svelte: list or folder tree, stage checkboxes,
  // discard, +/− counts), the selected file's working diff — BESIDE the list
  // when the pane is wide, below it when narrow — and the commit composer
  // (wip/WipComposer.svelte). Replaces the old separate "Changes" tab.
  import { onDestroy, tick, untrack } from 'svelte';
  import type { Attachment } from 'svelte/attachments';
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
  import { ListWindow } from '../../lib/listWindow.svelte';
  import { lsGet, lsSet } from '../../lib/storage';
  import WipSection from './wip/WipSection.svelte';
  import WipComposer from './wip/WipComposer.svelte';
  import {
    AUTO_COLLAPSE_FILES, KIND_WORD, buildTree, listRows, splitPath, treeRows,
    type LineCounts, type Row, type TFolder, type WipSection as Section,
  } from './wip/wipTree';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { draftInFlight, readComposer, startDraft, takePendingDraft, watchComposer, writeComposer } from './commitComposer';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Called after a successful commit so the graph can reload its log. */
    oncommitted: () => void;
    onclose: () => void;
    /** Open the conflict resolver (RepoView's tab) for a conflicted file. */
    onresolve?: () => void;
  }
  let { repoId, status, onstatus, oncommitted, onclose, onresolve }: Props = $props();

  // Conflicted files get their OWN section with resolve actions — they must
  // not sit in the stage/unstage trees, where the checkbox would `git add`
  // a file that still contains conflict markers and silently mark it resolved.
  const conflicted = $derived(status.changes.filter((c) => c.kind === 'conflicted'));
  // A partially staged path (`MM`) is ONE row with both flags and belongs in
  // BOTH lists — filtering on `!staged` hid its unstaged half, so "Stage all"
  // skipped it and the commit silently left those edits out.
  // Untracked files are their own section (no hunks to stage, and a discard
  // deletes them); `unstaged` is the TRACKED worktree side.
  const unstaged = $derived(status.changes.filter((c) => c.unstaged && c.kind !== 'conflicted' && c.kind !== 'untracked'));
  const untracked = $derived(status.changes.filter((c) => c.kind === 'untracked'));
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

  // ── Keyboard focus across index moves ─────────────────────────────────
  // Staging moves a path to the other list, so Svelte destroys the row —
  // and the focused checkbox with it — and focus fell to <body>. A row
  // control passes its origin; after the new status renders, focus goes to
  // the checkbox now at the same index in the same section (else the last
  // row, else the section heading), and a polite status says what happened.
  type Origin = { section: Section; key: string };
  let panelEl = $state<HTMLDivElement | null>(null);
  let announcement = $state('');
  const diffId = `wp-diff-${Math.random().toString(36).slice(2, 10)}`;
  const rowsOf = (section: Section): Row[] =>
    section === 'unstaged' ? unstagedRows : section === 'untracked' ? untrackedRows : stagedRows;
  const indexOf = (origin: Origin | undefined): number =>
    origin ? Math.max(0, rowsOf(origin.section).findIndex((r) => r.key === origin.key)) : -1;

  async function settleFocus(origin: Origin | undefined, before: number, message: string): Promise<void> {
    announcement = '';
    await tick();
    announcement = message;
    if (!origin) return;
    const rows = rowsOf(origin.section);
    // An emptied Untracked section unmounts: fall back to the Staged header.
    const head =
      panelEl?.querySelector<HTMLElement>(`[data-wp-head="${origin.section}"]`) ??
      panelEl?.querySelector<HTMLElement>('[data-wp-head="staged"]');
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

  /** Discard `paths`. From the Unstaged / Untracked lists only the unstaged
   *  side goes (`keep_staged`): staged hunks of a partially staged file
   *  survive, untracked files are deleted. From the Staged list the whole
   *  change is reverted to HEAD. */
  async function discardPaths(
    paths: string[],
    label: string,
    section: Section = 'staged',
    origin?: Origin,
  ): Promise<void> {
    if (paths.length === 0) return;
    const before = indexOf(origin);
    const keepStaged = section !== 'staged';
    const them = paths.length === 1 ? 'it' : 'them';
    const ok = await confirmer.ask(
      section === 'untracked'
        ? `Delete ${label}? Untracked files aren’t in git yet, so deleting ${them} cannot be undone.`
        : keepStaged
          ? `Discard unstaged changes to ${label}? This reverts ${them} to the staged version (or the last commit) — staged changes are kept — and cannot be undone.`
          : `Discard changes to ${label}? This reverts ${them} to the last commit (new files are deleted) and cannot be undone.`,
      {
        title: section === 'untracked' ? 'Delete untracked files' : keepStaged ? 'Discard unstaged changes' : 'Discard changes',
        confirmLabel: section === 'untracked' ? 'Delete' : 'Discard',
        danger: true,
      },
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

  function fileMenu(e: MouseEvent, c: FileChange, section: Section): void {
    e.preventDefault();
    const inStaged = section === 'staged';
    ctxMenu.show(e, [
      {
        label: inStaged ? 'Unstage' : 'Stage',
        action: () => void stagePaths([c.path], !inStaged),
      },
      { separator: true },
      {
        label: inStaged ? 'Discard…' : section === 'untracked' ? 'Delete…' : 'Discard unstaged changes…',
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
    section: Section,
  ): void {
    e.preventDefault();
    const paths = node.files.map((f) => f.path);
    const n = `${plural(paths.length, 'file')}`;
    ctxMenu.show(e, [
      {
        label: `${section === 'staged' ? 'Unstage' : 'Stage'} ${node.name}/ (${n})`,
        action: () => void stagePaths(paths, section !== 'staged'),
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

  // ── Rows: the folder tree (default, ordered by folder) or a flat list ─────────────────────
  // The view is a per-viewer convenience (lib/storage swallows a blocked
  // localStorage). Folders fold by "section:path" so the same folder can fold
  // independently per section; one never toggled auto-collapses past
  // AUTO_COLLAPSE_FILES (wipTree.ts). Both views window, so a 20k-file
  // list costs what a 50-file one does.
  const VIEW_KEY = 'otto.git.wip.view';
  let treeView = $state(lsGet(VIEW_KEY) !== 'list');
  function toggleView(): void {
    treeView = !treeView;
    lsSet(VIEW_KEY, treeView ? 'tree' : 'list');
  }
  let folds = $state.raw<Map<string, boolean>>(new Map());
  function isCollapsed(key: string, node: TFolder): boolean {
    return folds.get(key) ?? node.files.length > AUTO_COLLAPSE_FILES;
  }
  function toggleFolder(key: string, node: TFolder): void {
    const next = new Map(folds);
    next.set(key, !isCollapsed(key, node));
    folds = next;
  }
  function rowsFor(changes: FileChange[], section: Section): Row[] {
    if (!treeView) return listRows(changes);
    return treeRows(buildTree(changes), (n) => isCollapsed(`${section}:${n.path}`, n));
  }
  const unstagedRows = $derived(rowsFor(unstaged, 'unstaged'));
  const untrackedRows = $derived(rowsFor(untracked, 'untracked'));
  const stagedRows = $derived(rowsFor(staged, 'staged'));
  // Each section is WINDOWED against the shared `.wp-scroll` scroller: only
  // the rows in view (+ overscan) mount, between spacers sized to the rest;
  // ⌘F searches every row.
  const unstagedWin = new ListWindow('.wp-file, .wp-folder');
  const untrackedWin = new ListWindow('.wp-file, .wp-folder');
  const stagedWin = new ListWindow('.wp-file, .wp-folder');
  $effect(() => unstagedWin.findRows(() => null, () => unstagedRows, (r) => r.node.name));
  $effect(() => untrackedWin.findRows(() => null, () => untrackedRows, (r) => r.node.name));
  $effect(() => stagedWin.findRows(() => null, () => stagedRows, (r) => r.node.name));

  // Section collapse (all open by default; folding is still handy).
  let sectionOpen = $state<Record<Section, boolean>>({ unstaged: true, untracked: true, staged: true });

  // Later sections sit BELOW earlier ones: when rows (or a fold) change, their
  // offset in the scroller moves without a scroll.
  $effect(() => {
    void unstagedRows.length;
    void untrackedRows.length;
    void stagedRows.length;
    void sectionOpen.unstaged;
    void sectionOpen.untracked;
    void sectionOpen.staged;
    unstagedWin.refresh();
    untrackedWin.refresh();
    stagedWin.refresh();
  });

  /** Highlight a row as the open diff — for a partially staged path only on
   *  the side the diff shows. */
  const isSelected = (section: Section) => (path: string): boolean =>
    selectedPath === path && (!partial.has(path) || (section === 'staged') === stagedView);
  /** A partially staged path is in both lists: open the side clicked. A
   *  second click on the open row closes the diff. */
  function selectFile(path: string, section: Section): void {
    const same = isSelected(section)(path);
    stagedView = section === 'staged';
    selectedPath = same ? null : path;
  }

  // ── +/− line counts (one `summary=true` numstat per side, no patches) ──
  // Refreshed (debounced) on every status / live change. Past COUNTS_MAX
  // files the counts are skipped: the list must never wait on them, and a
  // failed read just leaves the rows without counts.
  const COUNTS_MAX = 2000;
  let unstagedCounts = $state.raw<LineCounts | null>(null);
  let stagedCounts = $state.raw<LineCounts | null>(null);
  function countsOf(d: DiffResp): LineCounts {
    const m: LineCounts = new Map();
    for (const f of d.files) {
      m.set(f.path, f.is_binary ? { added: null, deleted: null } : { added: f.added ?? 0, deleted: f.deleted ?? 0 });
    }
    return m;
  }
  $effect(() => {
    const id = repoId;
    const wantU = unstaged.length > 0 && unstaged.length <= COUNTS_MAX;
    const wantS = staged.length > 0 && staged.length <= COUNTS_MAX;
    void status.changes;
    void git.liveRev[id];
    const ctl = new AbortController();
    const load = (target: 'worktree' | 'staged', set: (m: LineCounts | null) => void): void => {
      void api
        .get<DiffResp>(`/repos/${id}/diff?target=${target}&summary=true`, ctl.signal)
        .then((d) => {
          if (!ctl.signal.aborted) set(countsOf(d));
        })
        .catch(() => {});
    };
    const t = setTimeout(() => {
      if (wantU) load('worktree', (m) => (unstagedCounts = m));
      else unstagedCounts = null;
      if (wantS) load('staged', (m) => (stagedCounts = m));
      else stagedCounts = null;
    }, 250);
    return () => {
      clearTimeout(t);
      ctl.abort();
    };
  });
  const totals = $derived.by(() => {
    let added = 0;
    let deleted = 0;
    for (const m of [unstagedCounts, stagedCounts]) {
      for (const v of m?.values() ?? []) {
        added += v.added ?? 0;
        deleted += v.deleted ?? 0;
      }
    }
    return added + deleted > 0 ? { added, deleted } : null;
  });

  // ── Keyboard: ↑/↓ (Home/End) move between rows across sections, ←/→ fold
  // a folder; with a diff open the moved-to file opens in its place (Tower /
  // VS Code). The checkbox and discard stay one Tab away.
  const rowNav: Attachment<HTMLElement> = (el) => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      const cur = (e.target as HTMLElement | null)?.closest?.<HTMLElement>('[data-wp-nav]');
      if (!cur) return;
      const all = Array.from(el.querySelectorAll<HTMLElement>('[data-wp-nav]'));
      const i = all.indexOf(cur);
      const rtl = getComputedStyle(el).direction === 'rtl';
      let next: HTMLElement | undefined;
      if (e.key === 'ArrowDown') next = all[i + 1];
      else if (e.key === 'ArrowUp') next = all[i - 1];
      else if (e.key === 'Home') next = all[0];
      else if (e.key === 'End') next = all.at(-1);
      else if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && cur.classList.contains('wp-fold-name')) {
        e.preventDefault();
        const expand = (e.key === 'ArrowRight') !== rtl;
        if (expand !== (cur.getAttribute('aria-expanded') === 'true')) cur.click();
        return;
      } else return;
      e.preventDefault();
      if (!next) return;
      next.focus();
      next.scrollIntoView({ block: 'nearest' });
      const key = next.dataset.wpNav ?? '';
      const section = next.closest<HTMLElement>('[data-wp-section]')?.dataset.wpSection as Section | undefined;
      if (selectedPath !== null && section && key.startsWith('f:')) {
        stagedView = section === 'staged';
        selectedPath = key.slice(2);
      }
    };
    el.addEventListener('keydown', onKey);
    return () => el.removeEventListener('keydown', onKey);
  };

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
</script>

{#snippet section(id: Section, title: string, changes: FileChange[], rows: Row[], lw: ListWindow, empty: string, counts: LineCounts | null)}
  <WipSection section={id} {title} count={changes.length} {rows} {lw} {empty} tree={treeView} {partial} {counts} {diffId}
    bind:open={() => sectionOpen[id], (v) => (sectionOpen[id] = v)}
    all={changes.map((c) => c.path)}
    selected={isSelected(id)}
    collapsed={(n) => isCollapsed(`${id}:${n.path}`, n)}
    ontoggle={(n) => toggleFolder(`${id}:${n.path}`, n)}
    onstage={(paths, origin) => void stagePaths(paths, id !== 'staged', origin)}
    ondiscard={(paths, label, origin) => void discardPaths(paths, label, id, origin)}
    onselect={(path) => selectFile(path, id)}
    onfilemenu={(e, c) => fileMenu(e, c, id)}
    onfoldermenu={(e, n) => folderMenu(e, n, id)}
    footer={id === 'untracked' ? untrackedCap : undefined}
  />
{/snippet}

{#snippet untrackedCap()}
  {#if hiddenUntracked > 0}
    <p class="wp-untracked-cap" role="note">
      <Icon name="info" size={12} />
      <span
        >{hiddenUntracked.toLocaleString()} more untracked {hiddenUntracked === 1 ? 'file' : 'files'} not shown. If they’re
        build output or dependencies, add them to <code>.gitignore</code>.</span
      >
    </p>
  {/if}
{/snippet}

{#snippet signError()}
  {#if signCfgError}
    <span class="sign-err">
      Couldn’t read signing settings ·
      <button class="btn small ghost" onclick={() => loadSignCfg(repoId)} disabled={signCfgLoading}>{signCfgLoading ? 'Retrying…' : 'Retry'}</button>
    </span>
  {/if}
{/snippet}

<div class="wip-panel" bind:this={panelEl}>
  <div class="sr-only" role="status" aria-live="polite">{announcement}</div>
  <div class="wp-head">
    <span class="wp-title">Changes</span>
    <span class="wp-branch" title="On branch {status.branch}"><Icon name="branch" size={12} /><span dir="ltr">{status.branch}</span></span>
    <span class="wp-count">{plural(status.changes.length, 'file')} changed</span>
    {#if totals}
      <span class="wp-totals" title="{totals.added.toLocaleString()} lines added, {totals.deleted.toLocaleString()} deleted">
        <span class="wp-add">+{totals.added.toLocaleString()}</span>
        <span class="wp-del">−{totals.deleted.toLocaleString()}</span>
      </span>
    {/if}
    <span class="grow"></span>
    {#if unstaged.length + untracked.length > 0}
      <button
        type="button"
        class="btn ghost small wp-stage-all"
        title="Stage every unstaged and untracked file"
        onclick={() => void stagePaths([...unstaged, ...untracked].map((c) => c.path), true)}
      >Stage all</button>
    {/if}
    <button
      type="button"
      class="icon-btn wp-view-btn"
      aria-pressed={treeView}
      onclick={toggleView}
      title={treeView ? 'Show as a list' : 'Group by folder'}
      aria-label="Group by folder"
    ><Icon name="folder" size={14} /></button>
    <button class="icon-btn wp-close" onclick={onclose} title="Close the changes panel" aria-label="Close the changes panel"><Icon name="x" size={14} /></button>
  </div>

  <div class="wp-body" class:has-diff={selectedPath !== null}>
    <div class="wp-scroll" {@attach rowNav}>
      <!-- Conflicts (own section, resolve actions instead of stage checkboxes) -->
      {#if conflicted.length > 0}
        <section class="wp-conflicts" aria-label="Conflicts">
          <div class="wp-conflict-head">
            <Icon name="merge" size={12} />
            <span>Conflicts</span>
            <span class="wp-ccount">{conflicted.length}</span>
            <span class="grow"></span>
            {#if onresolve}
              <button type="button" class="btn ghost small" onclick={() => onresolve?.()}>Open resolver</button>
            {/if}
          </div>
          {#each conflicted as c (c.path)}
            {@const parts = splitPath(c.path)}
            <div class="wp-crow">
              <span class="wp-ckind" title={KIND_WORD.conflicted} aria-hidden="true">!</span>
              <button class="wp-cname" onclick={() => onresolve?.()} title="Resolve {c.path} in the conflict resolver">
                <span class="wp-cfile" dir="ltr">{parts.name}</span>
                {#if parts.dir}<span class="wp-cdir" dir="ltr">{parts.dir}</span>{/if}
              </button>
              <button class="btn small ghost" title="Keep your version (ours)" onclick={() => void takeSide(c.path, 'ours')}>Ours</button>
              <button class="btn small ghost" title="Keep their version (theirs)" onclick={() => void takeSide(c.path, 'theirs')}>Theirs</button>
            </div>
          {/each}
        </section>
      {/if}

      {@render section('unstaged', 'Unstaged files', unstaged, unstagedRows, unstagedWin, 'Nothing unstaged.', unstagedCounts)}
      {#if untracked.length > 0 || hiddenUntracked > 0}
        {@render section('untracked', 'Untracked files', untracked, untrackedRows, untrackedWin, 'No untracked files shown.', null)}
      {/if}
      {@render section('staged', 'Staged files', staged, stagedRows, stagedWin, 'Nothing staged yet.', stagedCounts)}
    </div>

    <!-- Per-file diff: a SIBLING of the list scroller, never its tail (inside
         the scroller it rendered below every row — off-screen on a big
         changeset). Narrow: its own region under the list. Wide (container
         query): the column beside the list and composer. -->
    {#if selectedPath !== null}
      {@const parts = splitPath(selectedPath)}
      <div class="wp-diff" id={diffId}>
        <div class="wp-diff-head">
          <Icon name="file" size={12} />
          <span class="wp-diff-path" title={selectedPath} dir="ltr"><strong>{parts.name}</strong>{#if parts.dir}<span class="wp-diff-dir">{parts.dir}</span>{/if}</span>
          <span class="grow"></span>
          {#if partial.has(selectedPath)}
            <!-- Both sides exist: pick which one the hunk actions operate on. -->
            <div class="segmented wp-target" role="group" aria-label="Diff side">
              <button class:active={!stagedView} aria-pressed={!stagedView} onclick={() => (stagedView = false)}>Unstaged</button>
              <button class:active={stagedView} aria-pressed={stagedView} onclick={() => (stagedView = true)}>Staged</button>
            </div>
          {/if}
          <button class="icon-btn wp-close" onclick={() => (selectedPath = null)} title="Close diff" aria-label="Close diff"><Icon name="x" size={14} /></button>
        </div>
        <div class="wp-diff-body">
          {#if diffLoading && !diff}
            <div class="wp-diff-skel"><Skeleton rows={5} height={20} /></div>
          {:else if diff && diff.files.length > 0}
            <DiffViewer
              {diff}
              {repoId}
              loadFile={loadWipFile}
              wip={selTarget === 'working'
                ? undefined
                : {
                    target: selTarget,
                    onapplied: (r) => {
                      onstatus(r.status);
                      diff = r.diff;
                      if (r.backup_stash)
                        toasts.info('Backup stash kept', r.backup_stash.slice(0, 8));
                    },
                  }}
            />
          {:else if diffError}
            <LoadState variant="compact" what="this file’s diff" error={diffError} empty onretry={() => diffRetry++} />
          {:else}
            <div class="wp-empty">No textual diff for this file.</div>
          {/if}
        </div>
      </div>
    {/if}

    <div class="wp-compose">
      <WipComposer
        bind:subject bind:body bind:amend bind:signOn bind:showDraftTerm
        {amendSubject} {amendPublished} {drafting} {draftElapsed} {draftable} {draftedAt} {liveDraftId}
        {committing} {canCommit} {commitHint} stagedCount={staged.length} {signCfg} {signError}
        onkey={commitKey} ondraft={draftMessage} oncommit={commit}
      />
    </div>
  </div>
</div>

<style>
  .wip-panel {
    container: wip / inline-size;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--surface);
  }
  .wp-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 40px;
    padding-block: 4px;
    padding-inline: 14px 8px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
    min-width: 0;
  }
  .wp-title {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    flex-shrink: 0;
  }
  .wp-branch {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    max-width: 30%;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .wp-branch > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wp-branch :global(svg) {
    flex-shrink: 0;
  }
  .wp-count {
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .wp-totals {
    display: inline-flex;
    gap: 4px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .wp-add {
    color: var(--success);
  }
  .wp-del {
    color: var(--danger);
  }
  .wp-stage-all {
    flex-shrink: 0;
  }
  .wp-view-btn[aria-pressed='true'] {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  /* .icon-btn; only placement is local. */
  .wp-close {
    flex-shrink: 0;
  }
  .grow {
    flex: 1;
  }

  /* ── Layout: narrow = list / diff / composer stacked; wide = the diff in its
     own column beside the list + composer (container query, not viewport). */
  .wp-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .wp-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  /* With a diff open the list yields: capped (still scrollable) so the diff
     region below is always on-screen. */
  .wp-body.has-diff .wp-scroll {
    flex: 0 1 auto;
    max-height: 45%;
  }
  .wp-compose {
    flex-shrink: 0;
  }
  @container wip (min-width: 760px) {
    .wp-body.has-diff {
      display: grid;
      grid-template-columns: minmax(280px, 36%) minmax(0, 1fr);
      grid-template-rows: minmax(0, 1fr) auto;
      grid-template-areas: 'list diff' 'compose diff';
    }
    .wp-body.has-diff .wp-scroll {
      grid-area: list;
      max-height: none;
    }
    .wp-body.has-diff .wp-compose {
      grid-area: compose;
    }
    .wp-body.has-diff .wp-diff {
      grid-area: diff;
      border-block-start: none;
      border-inline-start: 1px solid var(--border);
    }
  }
  @container wip (max-width: 420px) {
    .wp-branch,
    .wp-totals {
      display: none;
    }
  }

  .wp-empty {
    padding: 8px 14px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }

  /* Conflicts: a warning-tinted header and side-pick buttons instead of the
     stage checkbox (staging an unresolved file would bury its markers). */
  .wp-conflicts {
    padding-block-end: 4px;
  }
  .wp-conflict-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 30px;
    padding-inline: 10px 4px;
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .wp-conflict-head > :global(svg) {
    color: var(--warning);
  }
  .wp-ccount {
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
    line-height: 16px;
  }
  .wp-crow {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    margin-inline: 4px;
    padding-inline: 8px 4px;
    border-radius: var(--radius-s);
  }
  .wp-crow:hover {
    background: var(--hover);
  }
  .wp-ckind {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--danger);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .wp-cname {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
    text-align: start;
  }
  .wp-cfile,
  .wp-cdir {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    unicode-bidi: isolate;
  }
  .wp-cfile {
    flex: 0 1 auto;
    font-size: var(--fs-m);
  }
  .wp-cdir {
    flex: 1 1 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }

  .wp-diff {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    border-block-start: 1px solid var(--border);
  }
  .wp-diff-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 36px;
    padding-block: 4px;
    padding-inline: 12px 8px;
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
    min-width: 0;
  }
  .wp-diff-head > :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .wp-diff-path {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    font-size: var(--fs-m);
    unicode-bidi: isolate;
  }
  .wp-diff-path > strong {
    font-weight: 600;
    flex-shrink: 0;
  }
  .wp-diff-dir {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .wp-diff-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .wp-diff-skel {
    padding: 10px;
  }
  .wp-target > button {
    height: 20px;
    padding: 0 6px;
    font-size: var(--fs-xs);
  }

  .sign-err {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .wp-untracked-cap {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 4px 10px 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-xs);
    line-height: 1.4;
  }
  .wp-untracked-cap code { font-size: var(--fs-xs); }

  /* ≤1024px (the graph stacks as an accordion): touch-height rows. */
  @media (max-width: 1024px) {
    .wp-crow {
      height: 36px;
    }
  }
</style>
