<script lang="ts">
  import { plural } from '../../lib/plural';
  // GitKraken-style WIP panel: shown in the graph's RIGHT detail pane when the
  // WIP row is selected. Unstaged / Staged file trees (per-file + per-folder
  // stage toggles, discard), a per-file working diff, and the commit composer.
  // Replaces the old separate "Changes" tab — staging now lives on the graph.
  import { untrack } from 'svelte';
  import { toastError } from '../../lib/toastError';
  import { api, isAbortError } from '../../lib/api/client';
  import type {
    CommitConfig,
    CommitInfo,
    DiffResp,
    DiscardReq,
    DraftCommitMessageResp,
    FileChange,
    RepoStatusResp,
  } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import DiffViewer from './DiffViewer.svelte';
  import { repoDiffFileLoader } from './diff-load';
  import Icon from '../../lib/components/Icon.svelte';
  import AgentByline from '../../lib/components/AgentByline.svelte';
  import { ListWindow } from './list-window.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

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
        diffError = e instanceof Error ? e.message : String(e);
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

  async function stagePaths(paths: string[], stage: boolean): Promise<void> {
    if (paths.length === 0) return;
    try {
      const s = await api.post<RepoStatusResp>(
        `/repos/${repoId}/${stage ? 'stage' : 'unstage'}`,
        { paths },
      );
      onstatus(s);
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
  ): Promise<void> {
    if (paths.length === 0) return;
    const keepStaged = section === 'unstaged';
    const them = paths.length === 1 ? 'it' : 'them';
    const ok = await confirmer.ask(
      keepStaged
        ? `Discard unstaged changes to ${label}? This reverts ${them} to the staged version (or the last commit) — staged changes are kept, untracked files are deleted — and cannot be undone.`
        : `Discard changes to ${label}? This reverts ${them} to the last commit (new files are deleted) and cannot be undone.`,
      { title: keepStaged ? 'Discard unstaged changes' : 'Discard changes', confirmLabel: 'Discard' },
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
        label: inStaged ? 'Discard' : 'Discard unstaged changes',
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
        label: `Discard ${node.name}/ (${n})`,
        icon: 'trash',
        danger: true,
        action: () => void discardPaths(paths, `${node.path}/ (${n})`, section),
      },
    ]);
  }

  // ── Folder tree (shared shape with the old Changes tab) ────────────────────
  type TFile = { type: 'file'; name: string; change: FileChange };
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
  // the rows in view (+ overscan) mount, between spacers sized to the rest,
  // so a 20k-file expanded folder costs the same as a 50-file one. Lists up
  // to 300 rows render whole (find-in-page still sees them).
  const unstagedWin = new ListWindow('.wp-file, .wp-folder');
  const stagedWin = new ListWindow('.wp-file, .wp-folder');
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
        if (seq === signCfgSeq) signCfgError = true;
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
  /** HEAD is already on the upstream: amending rewrites a pushed commit, and
   *  the next push needs a force (with lease). Said BEFORE the user commits. */
  const amendRewritesPushed = $derived(amend && status.upstream != null && status.ahead === 0);

  const canCommit = $derived(
    !committing && !drafting && (subject.trim() !== '' || amend) && (staged.length > 0 || amend),
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
    drafting = true;
    draftSessionId = null;
    draftElapsed = 0;
    // Slight backdate so a session created in the same second still matches.
    draftStartedAt = new Date(Date.now() - 2000).toISOString();
    const tick = setInterval(() => (draftElapsed += 1), 1000);
    try {
      const d = await api.post<DraftCommitMessageResp>(
        `/repos/${repoId}/draft-commit-message`,
        {},
      );
      const text = d.message.trim();
      // Never overwrite what the person already typed without asking.
      if (subject.trim() || body.trim()) {
        const ok = await confirmer.ask(
          'The agent’s message will replace the summary and description you have typed.',
          { title: 'Replace your title and description?', confirmLabel: 'Replace', cancelLabel: 'Keep mine', danger: false },
        );
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
    } catch (e) {
      toastError('Couldn’t draft the message', e);
    } finally {
      clearInterval(tick);
      drafting = false;
    }
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

{#snippet fileRow(file: TFile, depth: number, section: 'unstaged' | 'staged')}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="wp-file"
    class:selected={selectedPath === file.change.path}
    style="padding-inline-start:{8 + depth * 14}px"
    oncontextmenu={(e) => fileMenu(e, file.change, section)}
  >
    <input
      type="checkbox"
      checked={section === 'staged'}
      onchange={() => stagePaths([file.change.path], section === 'unstaged')}
      title={section === 'staged' ? 'Unstage' : 'Stage'}
      aria-label="{section === 'staged' ? 'Unstage' : 'Stage'} {file.change.path}"
    />
    <button
      class="wp-name"
      onclick={() => {
        // A partially staged path is in both lists: open the side clicked.
        stagedView = section === 'staged';
        selectedPath = selectedPath === file.change.path ? null : file.change.path;
      }}
      title={file.change.path}
    >
      <span class="kind k-{file.change.kind}">{kindBadge[file.change.kind]}</span>
      <span class="mono wp-fname">{file.name}</span>
      {#if partial.has(file.change.path)}
        <!-- Same path in both trees: some hunks staged, some not. -->
        <span class="chip partial" title="Partially staged">partial</span>
      {/if}
    </button>
    <button
      class="wp-discard"
      title={section === 'unstaged' ? 'Discard unstaged changes to this file' : 'Discard changes to this file'}
      aria-label="Discard {file.change.path}"
      onclick={() => void discardPaths([file.change.path], file.change.path, section)}
    >
      <Icon name="trash" size={12} />
    </button>
  </div>
{/snippet}

{#snippet folderRow(node: TFolder, depth: number, section: 'unstaged' | 'staged')}
    {@const key = `${section}:${node.path}`}
    {@const shut = isCollapsed(key, node)}
    <!-- Folder row: the checkbox stages/unstages EVERY file under the folder
         (recursively — node.files carries the whole subtree) in one call, the
         same affordance file rows have. The name/chevron only folds — a name
         click must never mutate the index. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="wp-folder"
      style="padding-inline-start:{8 + depth * 14}px"
      oncontextmenu={(e) => folderMenu(e, node, section)}
    >
      <input
        type="checkbox"
        checked={section === 'staged'}
        onchange={() => stagePaths(node.files.map((f) => f.path), section === 'unstaged')}
        title="{section === 'staged' ? 'Unstage' : 'Stage'} {node.path}/ ({plural(node.files.length, 'file')})"
        aria-label="{section === 'staged' ? 'Unstage' : 'Stage'} folder {node.path}"
      />
      <button
        class="wp-fold-name"
        onclick={() => toggleFolder(key, node)}
        title="{node.path}/ ({node.files.length})"
        aria-expanded={!shut}
      >
        <Icon name={shut ? 'chevronRight' : 'chevronDown'} size={12} />
        <Icon name="folder" size={12} />
        <span class="wp-fold-label">{node.name}</span>
        <span class="wp-fold-count">{node.files.length}</span>
      </button>
      <!-- Discard the whole folder in one confirmed call — the counterpart to
           the folder checkbox, which already stages/unstages the subtree. -->
      <button
        class="wp-discard"
        title="Discard changes to every file under {node.path}/"
        aria-label="Discard folder {node.path}"
        onclick={() =>
          void discardPaths(
            node.files.map((f) => f.path),
            `${node.path}/ (${plural(node.files.length, 'file')})`,
            section,
          )}
      >
        <Icon name="trash" size={12} />
      </button>
    </div>
{/snippet}

{#snippet sectionRows(rows: Row[], section: 'unstaged' | 'staged', empty: string)}
  {@const lw = winOf(section)}
  {@const win = lw.range(rows.length)}
  {#if rows.length === 0}
    <div class="dim wp-empty">{empty}</div>
  {/if}
  <div class="wp-rows" {@attach lw.attach}>
    {#if win.top > 0}<div class="wp-spacer" style="height:{win.top}px" aria-hidden="true"></div>{/if}
    {#each rows.slice(win.start, win.end) as r (r.key)}
      {#if r.node.type === 'folder'}
        {@render folderRow(r.node, r.depth, section)}
      {:else}
        {@render fileRow(r.node, r.depth, section)}
      {/if}
    {/each}
    {#if win.bottom > 0}<div class="wp-spacer" style="height:{win.bottom}px" aria-hidden="true"></div>{/if}
  </div>
{/snippet}

<div class="wip-panel">
  <div class="wp-head">
    <span class="wp-title mono">// WIP</span>
    <span class="wp-count">{plural(status.changes.length, 'file')} changed</span>
    <span class="grow"></span>
    <button class="icon-btn wp-close" onclick={onclose} title="Close WIP panel" aria-label="Close WIP panel"><Icon name="x" size={14} /></button>
  </div>

  <div class="wp-scroll" class:has-diff={selectedPath !== null}>
    <!-- Conflicts (own section, resolve actions instead of stage checkboxes) -->
    {#if conflicted.length > 0}
      <div class="wp-section wp-conflicts">
        <div class="wp-sec-head wp-conflict-head">
          <Icon name="merge" size={12} />
          <span>Conflicts</span>
          <span class="wp-sec-count">{conflicted.length}</span>
          <span class="grow"></span>
          {#if onresolve}
            <button
              type="button"
              class="btn ghost small wp-sec-action"
              onclick={(e) => {
                e.stopPropagation();
                onresolve?.();
              }}
            >Open resolver</button>
          {/if}
        </div>
        <div class="wp-list">
          {#each conflicted as c (c.path)}
            <div class="wp-file wp-conflict-row">
              <span class="kind k-conflicted">!</span>
              <button
                class="wp-name"
                onclick={() => onresolve?.()}
                title="Resolve {c.path} in the conflict resolver"
              >
                <span class="mono wp-fname">{c.path}</span>
              </button>
              <button class="wp-side" title="Keep your version (ours)" onclick={() => void takeSide(c.path, 'ours')}>Ours</button>
              <button class="wp-side" title="Keep their version (theirs)" onclick={() => void takeSide(c.path, 'theirs')}>Theirs</button>
            </div>
          {/each}
        </div>
      </div>
    {/if}

    <!-- Unstaged -->
    <div class="wp-section">
      <div class="wp-sec-head">
        <button type="button" class="wp-sec-toggle" onclick={() => (unstagedOpen = !unstagedOpen)} aria-expanded={unstagedOpen}>
          <Icon name={unstagedOpen ? 'chevronDown' : 'chevronRight'} size={12} />
          <span>Unstaged files</span>
          <span class="wp-sec-count">{unstaged.length}</span>
        </button>
        {#if unstaged.length > 0}
          <button
            type="button"
            class="btn ghost small wp-sec-action"
            onclick={(e) => {
              e.stopPropagation();
              void stagePaths(unstaged.map((c) => c.path), true);
            }}
          >Stage all</button>
          <button
            type="button"
            class="btn ghost small wp-sec-action danger"
            title="Discard changes to all unstaged files"
            onclick={(e) => {
              e.stopPropagation();
              void discardPaths(unstaged.map((c) => c.path), 'all unstaged files', 'unstaged');
            }}
          >Discard all</button>
        {/if}
      </div>
      {#if unstagedOpen}
        <div class="wp-list">
          {@render sectionRows(unstagedRows, 'unstaged', 'Nothing unstaged.')}
        </div>
        {#if hiddenUntracked > 0}
          <p class="wp-untracked-cap" role="note">
            <Icon name="info" size={12} />
            <span
              >{hiddenUntracked.toLocaleString()} more untracked file{hiddenUntracked === 1 ? '' : 's'} not shown. If they’re
              build output or dependencies, add them to <code>.gitignore</code>.</span
            >
          </p>
        {/if}
      {/if}
    </div>

    <!-- Staged -->
    <div class="wp-section">
      <div class="wp-sec-head">
        <button type="button" class="wp-sec-toggle" onclick={() => (stagedOpen = !stagedOpen)} aria-expanded={stagedOpen}>
          <Icon name={stagedOpen ? 'chevronDown' : 'chevronRight'} size={12} />
          <span>Staged files</span>
          <span class="wp-sec-count">{staged.length}</span>
        </button>
        {#if staged.length > 0}
          <button
            type="button"
            class="btn ghost small wp-sec-action"
            onclick={(e) => {
              e.stopPropagation();
              void stagePaths(staged.map((c) => c.path), false);
            }}
          >Unstage all</button>
          <button
            type="button"
            class="btn ghost small wp-sec-action danger"
            title="Discard changes to all staged files"
            onclick={(e) => {
              e.stopPropagation();
              void discardPaths(staged.map((c) => c.path), 'all staged files');
            }}
          >Discard all</button>
        {/if}
      </div>
      {#if stagedOpen}
        <div class="wp-list">
          {@render sectionRows(stagedRows, 'staged', 'Nothing staged yet.')}
        </div>
      {/if}
    </div>

  </div>

  <!-- Per-file diff: a SIBLING of the tree scroller, not its tail — appended
       inside wp-scroll it sat below every tree row, so on a large changeset
       (hundreds of files) selecting a file loaded the diff far off-screen and
       looked like nothing happened. As its own flex region it is visible the
       instant a file is clicked, with its own scrollbar. -->
  {#if selectedPath !== null}
    <div class="wp-diff">
      <div class="wp-diff-head">
        <Icon name="file" size={12} />
        <span class="mono wp-diff-path" title={selectedPath}>{selectedPath}</span>
        <span class="grow"></span>
        {#if partial.has(selectedPath)}
          <!-- Both sides exist: pick which one the hunk actions operate on. -->
          <div class="segmented wp-target">
            <button class:active={!stagedView} onclick={() => (stagedView = false)}>Unstaged</button>
            <button class:active={stagedView} onclick={() => (stagedView = true)}>Staged</button>
          </div>
        {/if}
        <button class="icon-btn wp-close" onclick={() => (selectedPath = null)} title="Close diff" aria-label="Close diff"><Icon name="x" size={14} /></button>
      </div>
      <div class="wp-diff-body">
        {#if diffLoading && !diff}
          <div style="padding: 10px"><Skeleton rows={5} height={20} /></div>
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
          <div class="dim wp-empty">Couldn’t load the diff: {diffError}</div>
        {:else}
          <div class="dim wp-empty">No textual diff for this file.</div>
        {/if}
      </div>
    </div>
  {/if}

  <!-- Commit composer -->
  <div class="wp-composer">
    <div class="msg-box">
      <input
        class="input subject-input"
        bind:value={subject}
        placeholder={amend
          ? amendSubject
            ? `Keep “${amendSubject}” — or type a new message`
            : 'Keep the previous message — or type a new one'
          : 'Summary'}
        aria-label="Commit summary"
        spellcheck="false"
        onkeydown={commitKey}
      />
      <button
        class="btn small ghost draft-btn"
        disabled={drafting || committing || !draftable}
        onclick={draftMessage}
        title={draftable
          ? 'Draft a summary + description from your staged changes'
          : 'Stage a file first — drafting can’t see untracked files'}
      >
        {#if drafting}
          <span class="spinner-xs"></span>Drafting…{draftElapsed > 0 ? ` ${draftElapsed}s` : ''}
        {:else}
          <Icon name="zap" size={11} /> Draft
        {/if}
      </button>
      {#if liveDraftId}
        <button
          class="btn small ghost draft-btn"
          onclick={() => (showDraftTerm = !showDraftTerm)}
          title={showDraftTerm ? 'Hide the drafting agent' : 'Watch the drafting agent live'}
        >
          <Icon name={showDraftTerm ? 'chevronUp' : 'terminal'} size={11} />
          {showDraftTerm ? 'Hide agent' : 'Watch agent'}
        </button>
      {/if}
    </div>
    {#if draftedAt && (subject.trim() || body.trim())}
      <div class="draft-by"><AgentByline label="Draft" at={draftedAt} /></div>
    {/if}
    {#if liveDraftId && showDraftTerm}
      <div class="draft-term">
        <Terminal sessionId={liveDraftId} preferDom showToolbar={false} />
      </div>
    {/if}
    <textarea
      class="input body-input"
      rows="2"
      bind:value={body}
      placeholder="Why this change, in a sentence or two"
      aria-label="Commit description (optional)"
      spellcheck="false"
      onkeydown={commitKey}
    ></textarea>
    {#if amendRewritesPushed}
      <div class="amend-warn" role="note">
        <Icon name="warning" size={12} />
        <span>This commit is already on {status.upstream}. Amending rewrites it — the next push will need a force push with lease.</span>
      </div>
    {/if}
    <div class="row">
      <label class="checkbox-row">
        <input type="checkbox" bind:checked={amend} />
        Amend
      </label>
      <label
        class="checkbox-row"
        title={signCfg?.signing_key
          ? `Sign with ${signCfg.format ?? 'gpg'} key ${signCfg.signing_key}`
          : 'Sign this commit'}
      >
        <input type="checkbox" bind:checked={signOn} />
        Sign
      </label>
      {#if signCfgError}
        <span class="sign-err" role="status">
          Couldn’t read signing settings ·
          <button class="btn small ghost" onclick={() => loadSignCfg(repoId)} disabled={signCfgLoading}>{signCfgLoading ? 'Retrying…' : 'Retry'}</button>
        </span>
      {/if}
      <span class="grow"></span>
      <button
        class="btn primary"
        disabled={!canCommit}
        onclick={commit}
        title="Commit (⌘↵)"
      >
        {committing ? 'Committing…' : `${amend ? 'Amend' : 'Commit'}${staged.length > 0 ? ` (${staged.length})` : ''}`}
      </button>
    </div>
  </div>
</div>

<style>
  .draft-term {
    height: 220px;
    margin: 6px 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--term-bg);
  }
  .wip-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .wp-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .wp-title {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--accent-text);
  }
  .wp-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* .icon-btn; only placement is local. */
  .wp-close {
    flex-shrink: 0;
  }
  .grow {
    flex: 1;
  }

  .wp-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  /* With a diff open the trees yield the pane: capped (but still scrollable)
     so the diff region below is always on-screen. */
  .wp-scroll.has-diff {
    flex: 0 1 auto;
    max-height: 45%;
  }
  .wp-section {
    border-bottom: 1px solid var(--border);
  }
  .wp-sec-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 10px;
    border: none;
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
  }
  .wp-sec-toggle {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    letter-spacing: inherit;
    cursor: pointer;
    text-align: start;
  }
  .wp-sec-count {
    font-size: var(--fs-xs);
    font-weight: 600;
    min-width: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
    text-align: center;
  }
  .draft-by {
    padding: 2px 0;
  }
  .wp-sec-head .wp-sec-action {
    height: auto;
    border: none;
    background: transparent;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
    padding: 2px 6px;
    border-radius: var(--radius-s);
  }
  .wp-sec-head .wp-sec-action:hover {
    background: var(--accent-soft);
  }
  .wp-sec-head .wp-sec-action.danger {
    color: var(--danger);
  }
  .wp-sec-head .wp-sec-action.danger:hover {
    background: color-mix(in srgb, var(--danger) 14%, transparent);
  }
  .wp-list {
    padding: 4px 2px;
  }
  .wp-empty {
    padding: 6px 12px;
    font-size: var(--fs-xs);
  }
  .wp-file {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border-radius: var(--radius-s);
    padding-inline-end: 6px;
  }
  .wp-file.selected {
    background: var(--accent-soft);
  }
  .wp-name {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    height: 26px;
    border: none;
    background: transparent;
    cursor: pointer;
    color: var(--text);
    text-align: start;
  }
  .wp-fname {
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wp-discard {
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    line-height: 1;
    opacity: 0;
    transition: opacity var(--dur-fast) ease-out;
  }
  .wp-file:hover .wp-discard,
  .wp-file:focus-within .wp-discard,
  .wp-file.selected .wp-discard,
  .wp-folder:hover .wp-discard,
  .wp-folder:focus-within .wp-discard,
  .wp-discard:focus-visible {
    opacity: 1;
  }
  /* No hover on touch: the action can't stay hidden behind it. */
  @media (hover: none) {
    .wp-discard {
      opacity: 1;
    }
  }
  .wp-discard:hover {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .wp-folder {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 26px;
    padding-inline-end: 6px;
  }
  .wp-folder:hover {
    background: color-mix(in srgb, var(--accent) 7%, transparent);
  }
  .wp-fold-name {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    height: 26px;
    border: none;
    background: transparent;
    cursor: pointer;
    color: var(--text);
    text-align: start;
  }
  .wp-fold-label {
    font-size: var(--fs-s);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wp-fold-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 0 6px;
    line-height: 15px;
    flex-shrink: 0;
  }

  /* Conflicts section: warn-tinted header, side-pick buttons instead of the
     stage checkbox (staging an unresolved file would bury its markers). */
  .wp-conflict-head {
    background: var(--warning-soft);
    color: var(--warning);
    cursor: default;
  }
  .wp-conflict-row {
    padding-inline-start: 8px;
    height: 26px;
  }
  .wp-side {
    flex-shrink: 0;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 2px 6px;
    border-radius: var(--radius-s);
  }
  .wp-side:hover {
    color: var(--text);
    background: var(--hover);
  }

  .kind {
    width: 15px;
    height: 15px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    font-weight: 600;
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .k-modified {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .k-added,
  .k-untracked {
    background: color-mix(in srgb, var(--success) 22%, transparent);
    color: var(--success);
  }
  .k-deleted {
    background: color-mix(in srgb, var(--danger) 22%, transparent);
    color: var(--danger);
  }
  .k-renamed {
    background: var(--accent-soft-strong);
    color: var(--accent-text);
  }
  .k-conflicted {
    background: color-mix(in srgb, var(--danger) 35%, transparent);
    color: var(--danger);
  }

  .wp-diff {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
  }
  .wp-diff-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .wp-diff-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .wp-diff-path {
    font-size: var(--fs-xs);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .wp-composer {
    border-top: 1px solid var(--border);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex-shrink: 0;
  }
  .msg-box {
    position: relative;
  }
  .subject-input {
    width: 100%;
    height: 30px;
    padding-inline-end: 76px;
    font-weight: 600;
  }
  .body-input {
    width: 100%;
    resize: vertical;
  }
  .draft-btn {
    position: absolute;
    top: 4px;
    inset-inline-end: 6px;
  }
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: otto-spin 0.8s linear infinite;
    vertical-align: middle;
    margin-inline-end: 4px;
  }
  
  .sign-err {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .amend-warn {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--fs-s);
  }
  .amend-warn :global(svg) {
    flex-shrink: 0;
    margin-block-start: 2px;
  }
  .checkbox-row {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
  }
  /* "partial" = the path sits in BOTH trees (some hunks staged). Quiet — it
     annotates a row that is already busy with a kind badge and a name. */
  .chip.partial {
    height: 15px;
    padding: 0 4px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
    flex-shrink: 0;
  }
  .wp-target > button {
    height: 18px;
    padding: 0 6px;
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ≤1024px (the graph stacks as an accordion): the panel renders full-width
     under the mobile diff header; give touch targets real height. */
  @media (max-width: 1024px) {
    .wp-name,
    .wp-fold-name,
    .wp-folder {
      height: 34px;
    }
    .wp-fname,
    .wp-fold-label {
      font-size: var(--fs-m);
    }
    .wp-file input[type='checkbox'],
    .wp-folder input[type='checkbox'] {
      width: 17px;
      height: 17px;
    }
    .wp-discard {
      opacity: 1;
      padding: 6px 6px;
    }
    .subject-input {
      font-size: 16px;
      height: 38px;
    }
    .body-input {
      font-size: 16px;
      line-height: 1.45;
    }
    .wp-composer .btn.primary {
      height: 36px;
      padding: 0 16px;
      font-size: var(--fs-l);
    }
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
</style>
