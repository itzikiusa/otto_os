<script lang="ts">
  // Two-pane: LEFT = refs tree (local/remote/tags), MIDDLE = commit graph, RIGHT = commit detail/diff.
  import { api } from '../../lib/api/client';
  import type { CommitInfo, RefsResp, RefBranch, RefTag, RepoStatusResp, DiffResp, FileDiff, DiffLine } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CreatePr from './CreatePr.svelte';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Drag-to-merge: dropping branch `source` onto a different local branch
     *  `target` asks the parent to open the merge approval modal. Nothing merges
     *  here — the parent owns the modal + conflict-resolver routing. */
    onmergerequest?: (source: string, target: string) => void;
  }
  let { repoId, status, onstatus, onmergerequest }: Props = $props();

  // ── Drag-to-merge state ─────────────────────────────────────────────────────
  // The branch currently being dragged ({ name, remote }), and the local branch
  // the pointer is hovering as a candidate drop target. A valid drop = a local
  // branch whose name differs from the drag source's name.
  let dragSource = $state<{ name: string; remote: boolean } | null>(null);
  let dragOverTarget = $state<string | null>(null);

  function dragSourceName(): string | null {
    if (!dragSource) return null;
    // A remote ref like "origin/feature" merges as its local short name.
    return dragSource.remote ? dragSource.name.replace(/^[^/]+\//, '') : dragSource.name;
  }

  function isValidDropTarget(localName: string): boolean {
    const src = dragSourceName();
    return src !== null && src !== localName;
  }

  function onRefDragStart(e: DragEvent, name: string, remote: boolean): void {
    dragSource = { name, remote };
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Stash a payload too so the drag is "real" in all browsers/webviews.
      e.dataTransfer.setData('text/plain', name);
    }
  }

  function onRefDragEnd(): void {
    dragSource = null;
    dragOverTarget = null;
  }

  function onTargetDragOver(e: DragEvent, localName: string): void {
    if (!isValidDropTarget(localName)) return;
    e.preventDefault(); // allow drop
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverTarget = localName;
  }

  function onTargetDragLeave(localName: string): void {
    if (dragOverTarget === localName) dragOverTarget = null;
  }

  function onTargetDrop(e: DragEvent, localName: string): void {
    if (!isValidDropTarget(localName)) return;
    e.preventDefault();
    const source = dragSourceName();
    dragOverTarget = null;
    dragSource = null;
    if (source && onmergerequest) onmergerequest(source, localName);
  }

  // ── Mobile (phone) layout ─────────────────────────────────────────────────
  // On a phone the 3-pane desktop layout (refs | graph | diff) can't fit, so we
  // stack the panes as COLLAPSIBLE accordion sections, each independently
  // scrollable. Tracked via matchMedia so the same component serves both layouts
  // without duplicating markup. The breakpoint is 1024px so it catches phone
  // portrait + landscape AND the tablet range (iPad portrait 834, real-phone
  // landscape 932) — the desktop 3-pane (refs | graph | diff) can't fit there
  // without the diff getting clipped off-screen, so those widths get the stacked
  // accordion + wrapping-diff layout. Desktop (≥1025) keeps the 3-pane view.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });

  // Which mobile sections are expanded. The refs (branches) tree starts collapsed
  // — the commit graph is what users want first — and the diff opens on select.
  let secRefsOpen = $state(false);
  let secCommitsOpen = $state(true);

  // ── Refs ──────────────────────────────────────────────────────────────────
  let refs: RefsResp | null = $state(null);
  let refsLoading = $state(true);

  // Section collapse state
  let localOpen = $state(true);
  let remoteOpen = $state(true);
  let tagsOpen = $state(false);

  let checkoutBusy = $state('');

  // ── Commits / graph ───────────────────────────────────────────────────────
  let commits: CommitInfo[] = $state([]);
  let commitsLoading = $state(true);

  // ── Commit detail / diff ─────────────────────────────────────────────────
  let selectedSha = $state<string | null>(null);
  let selectedCommit = $state<CommitInfo | null>(null);
  let diffResp = $state<DiffResp | null>(null);
  let diffLoading = $state(false);
  // Track which files are collapsed (path → true = collapsed)
  let fileCollapsed = $state<Record<string, boolean>>({});

  $effect(() => {
    const id = repoId;
    refsLoading = true;
    commitsLoading = true;
    refs = null;
    commits = [];

    void api
      .get<RefsResp>(`/repos/${id}/refs`)
      .then((r) => (refs = r))
      .catch(() => (refs = { local: [], remote: [], tags: [] }))
      .finally(() => (refsLoading = false));

    void api
      .get<CommitInfo[]>(`/repos/${id}/log?all=true&limit=200`)
      .then((c) => (commits = c))
      .catch(() => (commits = []))
      .finally(() => (commitsLoading = false));
  });

  // ── Context-menu helpers ────────────────────────────────────────────────────
  /** Copy `text` to the clipboard and toast success/failure with `label`. */
  async function clip(text: string, label: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      toasts.success('Copied', label);
    } catch (e) {
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    }
  }

  /** After a mutating graph op: propagate the returned status (if any) to the
   *  parent and re-query refs + log so the graph reflects the change. */
  async function refreshAfter(status?: RepoStatusResp): Promise<void> {
    if (status) onstatus(status);
    await Promise.all([
      api.get<RefsResp>(`/repos/${repoId}/refs`).then((r) => (refs = r)).catch(() => {}),
      api
        .get<CommitInfo[]>(`/repos/${repoId}/log?all=true&limit=200`)
        .then((c) => (commits = c))
        .catch(() => {}),
    ]);
  }

  /** Run a mutating POST that returns RepoStatusResp, then refresh + toast.
   *  Errors are surfaced via toast and swallowed (the graph is left untouched). */
  async function mutate(
    path: string,
    body: unknown,
    okTitle: string,
    okDetail?: string,
  ): Promise<void> {
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}${path}`, body);
      await refreshAfter(s);
      toasts.success(okTitle, okDetail);
    } catch (e) {
      toasts.error(`${okTitle} failed`, e instanceof Error ? e.message : String(e));
    }
  }

  // ── Create-PR sheet (opened from branch / commit menus with a pre-filled
  //    source). Reuses CreatePr.svelte; `initialSource` defaults the dropdown. ──
  let createPrOpen = $state(false);
  let createPrSource = $state<string | undefined>(undefined);

  function openCreatePr(source: string): void {
    createPrSource = source;
    createPrOpen = true;
  }

  // ── GitFlow: start a feature/release/hotfix branch off a commit or branch tip.
  //    Reuses the same /branch endpoint (checkout:true) as the plain create. ──
  /** Normalize a user-typed flow name: trim, drop a leading flow prefix if they
   *  typed it, and collapse internal whitespace to dashes. '' → skip. */
  function normalizeFlowName(raw: string): string {
    return raw
      .trim()
      .replace(/^(feature|release|hotfix)\//i, '')
      .replace(/\s+/g, '-');
  }

  async function startFlowBranch(
    flow: 'feature' | 'release' | 'hotfix',
    startPoint: string,
  ): Promise<void> {
    const label = flow[0].toUpperCase() + flow.slice(1);
    const raw = await confirmer.promptText(`New ${flow} branch name`, {
      title: `Start ${flow}`,
      confirmLabel: 'Start',
      placeholder: 'short-name',
    });
    if (raw === null) return;
    const norm = normalizeFlowName(raw);
    if (norm === '') return;
    const name = `${flow}/${norm}`;
    await mutate('/branch', { name, start_point: startPoint, checkout: true }, `${label} branch created`, name);
  }

  /** Menu items for the "GitFlow" section, parameterised by start point
   *  (a commit sha or a branch tip name). Shared by commit + branch menus. */
  function gitFlowItems(startPoint: string): MenuItem[] {
    return [
      { label: 'Start feature branch…', icon: 'branch', action: () => void startFlowBranch('feature', startPoint) },
      { label: 'Start release branch…', icon: 'branch', action: () => void startFlowBranch('release', startPoint) },
      { label: 'Start hotfix branch…', icon: 'branch', action: () => void startFlowBranch('hotfix', startPoint) },
    ];
  }

  // ── Commit context menu ─────────────────────────────────────────────────────
  function commitMenu(e: MouseEvent, c: CommitInfo): void {
    const { currentBranch } = refKnowledge;
    const items: MenuItem[] = [
      {
        label: 'Check out commit (detached)',
        icon: 'branch',
        action: () => void checkout(c.sha, false),
      },
      { label: 'Cherry-pick commit', icon: 'note', action: () => void cherryPick(c) },
      { separator: true },
      { label: 'Create branch here…', icon: 'branch', action: () => void createBranchAt(c.sha) },
      { label: 'Create tag here…', icon: 'tag', action: () => void createTagAt(c.sha, false) },
      {
        label: 'Create annotated tag here…',
        icon: 'tag',
        action: () => void createTagAt(c.sha, true),
      },
    ];
    // Open a PR from the checked-out branch (the "from a commit while on a
    // branch" case) — only meaningful when a branch is checked out.
    if (currentBranch) {
      items.push({ separator: true });
      items.push({
        label: `Create PR from ${currentBranch}…`,
        icon: 'send',
        action: () => openCreatePr(currentBranch),
      });
    }
    // GitFlow section — start a flow branch off this exact commit.
    items.push({ separator: true });
    items.push({ label: 'GitFlow', disabled: true });
    items.push(...gitFlowItems(c.sha));
    items.push(
      { separator: true },
      { label: 'Revert commit', icon: 'refresh', danger: true, action: () => void revertCommit(c) },
      { separator: true },
      { label: 'Copy commit SHA', icon: 'note', action: () => void clip(c.sha, c.sha) },
      { label: 'Copy short SHA', icon: 'note', action: () => void clip(c.short_sha, c.short_sha) },
      {
        label: 'Copy commit message',
        icon: 'note',
        action: () => void clip(c.subject, c.subject),
      },
    );
    ctxMenu.show(e, items);
  }

  async function cherryPick(c: CommitInfo): Promise<void> {
    await mutate('/cherry-pick', { sha: c.sha }, 'Cherry-picked', c.short_sha);
  }

  async function revertCommit(c: CommitInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Revert commit ${c.short_sha} — "${c.subject}"? This creates a new commit undoing its changes.`,
      { title: 'Revert commit', confirmLabel: 'Revert', danger: true },
    );
    if (!ok) return;
    await mutate('/revert', { sha: c.sha }, 'Reverted', c.short_sha);
  }

  async function createBranchAt(startPoint: string): Promise<void> {
    const name = await confirmer.promptText('New branch name', {
      title: 'Create branch',
      confirmLabel: 'Create',
      placeholder: 'feature/my-branch',
    });
    if (!name) return;
    await mutate(
      '/branch',
      { name, start_point: startPoint, checkout: true },
      'Branch created',
      name,
    );
  }

  async function createTagAt(sha: string, annotated: boolean): Promise<void> {
    const name = await confirmer.promptText('Tag name', {
      title: annotated ? 'Create annotated tag' : 'Create tag',
      confirmLabel: 'Create',
      placeholder: 'v1.0.0',
    });
    if (!name) return;
    let message: string | null = null;
    if (annotated) {
      message = await confirmer.promptText('Tag message', {
        title: 'Annotated tag message',
        confirmLabel: 'Create',
        placeholder: 'Release notes…',
      });
      if (!message) return;
    }
    // Ask whether to push the new tag straight to origin.
    const push = await confirmer.ask(`Push tag "${name}" to origin?`, {
      title: 'Push tag',
      confirmLabel: 'Push',
      danger: false,
    });
    await mutate(
      '/tag',
      annotated ? { name, sha, message, push } : { name, sha, push },
      'Tag created',
      name,
    );
  }

  /** Local branches OTHER than `selfMerge` (the menu subject's merge name),
   *  as "Merge into <t>" items routing through the existing merge+conflict flow
   *  via `onmergerequest(<source merge name>, t)`. Empty if nothing to merge into. */
  function mergeIntoItems(selfMerge: string): MenuItem[] {
    const targets = (refs?.local ?? []).map((b) => b.name).filter((t) => t !== selfMerge);
    if (!onmergerequest || targets.length === 0) return [];
    return targets.map((t) => ({
      label: `Merge into ${t}`,
      icon: 'branch',
      action: () => onmergerequest?.(selfMerge, t),
    }));
  }

  // ── Branch context menu (local + remote ref rows) ───────────────────────────
  function branchMenu(e: MouseEvent, b: RefBranch): void {
    const { remoteNames, currentBranch } = refKnowledge;
    const items: MenuItem[] = [];

    if (b.remote) {
      // Remote ref row: checkout as a local tracking branch; delete on origin.
      const localName = b.name.replace(/^[^/]+\//, '');
      items.push({ label: 'Checkout', icon: 'branch', action: () => checkoutRemote(b) });
      items.push({ separator: true });
      items.push({
        label: 'Create branch from here…',
        icon: 'branch',
        action: () => void createBranchFrom(b.name),
      });
      // Merge into … — remote refs merge by their short local name (drag logic).
      const mergeInto = mergeIntoItems(localName);
      if (mergeInto.length > 0) {
        items.push({ separator: true });
        items.push({ label: `Merge ${localName} into`, disabled: true });
        items.push(...mergeInto);
      }
      // GitFlow — start a flow branch off the remote tip.
      items.push({ separator: true });
      items.push({ label: 'GitFlow', disabled: true });
      items.push(...gitFlowItems(b.name));
      items.push({ separator: true });
      items.push({
        label: `Delete ${b.name}`,
        icon: 'trash',
        danger: true,
        action: () => void deleteRemoteBranch(localName),
      });
      items.push({ separator: true });
      items.push({ label: 'Copy branch name', icon: 'note', action: () => void clip(localName, localName) });
    } else {
      const isCurrent = b.name === currentBranch;
      items.push({
        label: 'Checkout',
        icon: 'branch',
        disabled: isCurrent,
        action: () => !isCurrent && checkout(b.name, false),
      });
      items.push({ separator: true });
      items.push({
        label: 'Create branch from here…',
        icon: 'branch',
        action: () => void createBranchFrom(b.name),
      });
      items.push({ label: 'Rename…', icon: 'edit', action: () => void renameBranch(b.name) });
      // Open a PR sourced from this branch.
      items.push({ separator: true });
      items.push({
        label: `Create pull request from ${b.name}…`,
        icon: 'send',
        action: () => openCreatePr(b.name),
      });
      // Merge into … — pick any OTHER local branch as the destination.
      const mergeInto = mergeIntoItems(b.name);
      if (mergeInto.length > 0) {
        items.push({ separator: true });
        items.push({ label: `Merge ${b.name} into`, disabled: true });
        items.push(...mergeInto);
      }
      // GitFlow — start a flow branch off this branch tip.
      items.push({ separator: true });
      items.push({ label: 'GitFlow', disabled: true });
      items.push(...gitFlowItems(b.name));
      // A matching remote branch (origin/<name>) → offer remote deletes too.
      const hasRemote = remoteNames.has(`origin/${b.name}`);
      // Never offer Delete on the checked-out branch (git refuses).
      if (!isCurrent) {
        items.push({ separator: true });
        items.push({
          label: `Delete ${b.name}`,
          icon: 'trash',
          danger: true,
          action: () => void deleteLocalBranch(b.name, false),
        });
        if (hasRemote) {
          items.push({
            label: `Delete origin/${b.name}`,
            icon: 'trash',
            danger: true,
            action: () => void deleteRemoteBranch(b.name),
          });
          items.push({
            label: 'Delete local + remote',
            icon: 'trash',
            danger: true,
            action: () => void deleteLocalBranch(b.name, true),
          });
        }
      }
      items.push({ separator: true });
      items.push({ label: 'Copy branch name', icon: 'note', action: () => void clip(b.name, b.name) });
      if (b.upstream) {
        items.push({
          label: 'Copy upstream name',
          icon: 'note',
          action: () => void clip(b.upstream!, b.upstream!),
        });
      }
    }
    ctxMenu.show(e, items);
  }

  async function createBranchFrom(ref: string): Promise<void> {
    const name = await confirmer.promptText(`New branch from ${ref}`, {
      title: 'Create branch',
      confirmLabel: 'Create',
      placeholder: 'feature/my-branch',
    });
    if (!name) return;
    await mutate('/branch', { name, start_point: ref, checkout: true }, 'Branch created', name);
  }

  async function renameBranch(from: string): Promise<void> {
    const to = await confirmer.promptText(`Rename branch "${from}" to`, {
      title: 'Rename branch',
      confirmLabel: 'Rename',
      initial: from,
    });
    if (!to || to === from) return;
    await mutate('/branch/rename', { from, to }, 'Branch renamed', `${from} → ${to}`);
  }

  async function deleteLocalBranch(name: string, alsoRemote: boolean): Promise<void> {
    const ok = await confirmer.ask(
      alsoRemote
        ? `Delete branch "${name}" locally AND on origin? This cannot be undone.`
        : `Delete local branch "${name}"?`,
      { title: 'Delete branch', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    await mutate(
      '/branch/delete',
      { name, remote: alsoRemote },
      alsoRemote ? 'Branch deleted (local + remote)' : 'Branch deleted',
      name,
    );
  }

  async function deleteRemoteBranch(name: string): Promise<void> {
    const ok = await confirmer.ask(`Delete branch "${name}" on origin? This cannot be undone.`, {
      title: 'Delete remote branch',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    // Remote-only: leave any local branch of the same name intact.
    await mutate(
      '/branch/delete',
      { name, remote: true, local: false, force: true },
      'Remote branch deleted',
      `origin/${name}`,
    );
  }

  // ── Tag context menu ────────────────────────────────────────────────────────
  function tagMenu(e: MouseEvent, t: RefTag): void {
    const items: MenuItem[] = [
      {
        label: 'Check out tag (detached)',
        icon: 'branch',
        action: () => void checkout(t.name, false),
      },
      { separator: true },
      { label: 'Push tag to origin', icon: 'send', action: () => void pushTag(t.name) },
      {
        label: 'Delete tag',
        icon: 'trash',
        danger: true,
        action: () => void deleteTag(t.name, false),
      },
      {
        label: 'Delete tag on origin',
        icon: 'trash',
        danger: true,
        action: () => void deleteTag(t.name, true),
      },
      { separator: true },
      { label: 'Copy tag name', icon: 'note', action: () => void clip(t.name, t.name) },
    ];
    ctxMenu.show(e, items);
  }

  async function pushTag(name: string): Promise<void> {
    await mutate('/tag/push', { name }, 'Tag pushed', name);
  }

  async function deleteTag(name: string, remote: boolean): Promise<void> {
    const ok = await confirmer.ask(
      remote ? `Delete tag "${name}" on origin?` : `Delete local tag "${name}"?`,
      { title: 'Delete tag', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    await mutate('/tag/delete', { name, remote }, remote ? 'Tag deleted on origin' : 'Tag deleted', name);
  }

  async function checkout(branch: string, create: boolean): Promise<void> {
    checkoutBusy = branch;
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/checkout`, { branch, create });
      onstatus(s);
      toasts.success(create ? 'Branch created' : 'Switched branch', branch);
      // Refresh refs after checkout
      refs = await api.get<RefsResp>(`/repos/${repoId}/refs`);
    } catch (e) {
      toasts.error('Checkout failed', e instanceof Error ? e.message : String(e));
    } finally {
      checkoutBusy = '';
    }
  }

  function checkoutRemote(b: RefBranch): void {
    // strip "origin/" prefix to get local branch name
    const localName = b.name.replace(/^[^/]+\//, '');
    void checkout(localName, true);
  }

  async function selectCommit(commit: CommitInfo): Promise<void> {
    if (selectedSha === commit.sha) {
      // clicking again deselects
      clearSelection();
      return;
    }
    selectedSha = commit.sha;
    selectedCommit = commit;
    diffResp = null;
    diffLoading = true;
    fileCollapsed = {};
    // On a phone, collapse the commit list so the diff section gets the room.
    if (isMobile) secCommitsOpen = false;
    try {
      const resp = await api.get<DiffResp>(
        `/repos/${repoId}/diff?target=${encodeURIComponent('commit:' + commit.sha)}`
      );
      diffResp = resp;
      // Auto-collapse files with >400 changed lines
      const next: Record<string, boolean> = {};
      for (const f of resp.files) {
        const { add, del } = changedLinesCount(f);
        next[f.path] = add + del > 400;
      }
      fileCollapsed = next;
    } catch (e) {
      toasts.error('Failed to load diff', e instanceof Error ? e.message : String(e));
      diffResp = { files: [] };
    } finally {
      diffLoading = false;
    }
  }

  function clearSelection(): void {
    selectedSha = null;
    selectedCommit = null;
    diffResp = null;
    diffLoading = false;
    fileCollapsed = {};
    // Re-open the commit list when the diff closes (mobile accordion).
    if (isMobile) secCommitsOpen = true;
  }

  // ── Lane / graph algorithm ────────────────────────────────────────────────
  // palette of 8 colors (CSS vars so they adapt to theme)
  const PALETTE = [
    '#5B8BF5', '#E06C75', '#56B6C2', '#E5C07B',
    '#98C379', '#C678DD', '#61AFEF', '#D19A66',
  ];

  interface LaneRow {
    commit: CommitInfo;
    col: number;       // which column this commit's node lands on
    lines: LaneLine[]; // segments to draw in the SVG gutter
    color: string;     // node color
  }

  interface LaneLine {
    fromCol: number;
    toCol: number;
    color: string;
    // vert      = straight continuation in a lane
    // merge-in  = this node's extra parent heading down into another lane
    // converge  = a duplicate-awaiting lane from above folding into this node
    kind: 'vert' | 'merge-in' | 'branch-out' | 'converge';
  }

  const LANE_W = 14; // pixels per lane column
  const NODE_R = 4;  // node radius

  // Hard cap on lane count. Past this we stop opening brand-new lanes for extra
  // (merge) parents and route them to the node's own column, so a pathological
  // `--all` fan-out degrades gracefully instead of exploding sideways.
  const MAX_LANES = 24;

  // The graph: rows + the widest lane count reached (drives the gutter width so
  // it reflects the real fan-out, not just node columns). Computed in one pass.
  const graph = $derived.by((): { rows: LaneRow[]; widest: number } => {
    if (commits.length === 0) {
      return { rows: [], widest: 1 };
    }

    // lanes[i] = sha of the commit expected next in lane i (null = free).
    // laneColors[i] = palette index for lane i — kept index-aligned with lanes[]
    // through every push / pop / null so colors never drift off their lane.
    const lanes: (string | null)[] = [];
    const laneColors: number[] = [];

    let colorIdx = 0;
    let widest = 1;

    function laneColorAt(i: number): string {
      return PALETTE[(laneColors[i] ?? 0) % PALETTE.length];
    }

    function allocateLane(sha: string | null): number {
      // prefer reusing a free slot
      const free = lanes.indexOf(null);
      if (free !== -1) {
        lanes[free] = sha;
        if (laneColors[free] === undefined) laneColors[free] = colorIdx++ % PALETTE.length;
        return free;
      }
      lanes.push(sha);
      laneColors.push(colorIdx++ % PALETTE.length);
      return lanes.length - 1;
    }

    const rows: LaneRow[] = [];

    for (const commit of commits) {
      // Find this commit's column (which lane is "expecting" this sha)
      let col = lanes.indexOf(commit.sha);
      if (col === -1) {
        col = allocateLane(commit.sha);
      }
      const color = laneColorAt(col);

      // Build line segments for BEFORE this node (connecting to previous)
      const lines: LaneLine[] = [];

      // Collapse duplicate-awaiting lanes: more than one lane can await the SAME
      // parent sha (shared history / repeated merges in `--all`). `indexOf` only
      // resolved the first; the rest used to run parallel forever and pile up
      // into a "wall" of dangling verticals. Converge them onto `col` and free
      // the lane so the gutter can shrink back.
      for (let i = 0; i < lanes.length; i++) {
        if (i === col) continue;
        if (lanes[i] === commit.sha) {
          lines.push({ fromCol: i, toCol: col, color: laneColorAt(i), kind: 'converge' });
          lanes[i] = null;
        }
      }

      // Vertical continuations for all active lanes (before node)
      for (let i = 0; i < lanes.length; i++) {
        if (i === col) continue;
        if (lanes[i] !== null) {
          lines.push({ fromCol: i, toCol: i, color: laneColorAt(i), kind: 'vert' });
        }
      }

      // Now update lanes: replace this lane with first parent
      const [firstParent, ...extraParents] = commit.parents;
      if (firstParent) {
        lanes[col] = firstParent;
      } else {
        lanes[col] = null; // root commit or end of history
      }

      // For additional parents (merge commits): find existing lane or create one
      for (const parent of extraParents) {
        const existingLane = lanes.indexOf(parent);
        if (existingLane !== -1) {
          // already tracked — draw merge-in line
          lines.push({ fromCol: col, toCol: existingLane, color, kind: 'merge-in' });
        } else if (lanes.length >= MAX_LANES && lanes.indexOf(null) === -1) {
          // Safety cap: no free lane and we're at the ceiling — don't open a new
          // lane for this merge parent (it goes untracked); just draw a stub
          // converging on the node's own column so the merge is still visible.
          // Degrades gracefully instead of widening the gutter without bound.
          lines.push({ fromCol: col, toCol: col, color, kind: 'merge-in' });
        } else {
          const newLane = allocateLane(parent);
          lines.push({ fromCol: col, toCol: newLane, color: laneColorAt(newLane), kind: 'merge-in' });
        }
      }

      // Trim trailing free lanes so the gutter shrinks back as branches close
      // (keeps laneColors index-aligned by popping its tail in lockstep).
      while (lanes.length && lanes[lanes.length - 1] === null) {
        lanes.pop();
        laneColors.pop();
      }

      if (lanes.length > widest) widest = lanes.length;

      rows.push({ commit, col, lines, color });
    }

    return { rows, widest };
  });

  const laneRows = $derived(graph.rows);

  // Gutter width shared by every row so lane dots line up vertically. Sized to
  // the widest lane count actually reached (plus a half-lane for the node
  // radius), and capped so a wide fan-out can't blow out the layout.
  const MAX_GUTTER_W = 200;
  const gutterWidth = $derived.by(() => {
    if (graph.rows.length === 0) return LANE_W;
    return Math.min(graph.widest * LANE_W + LANE_W / 2, MAX_GUTTER_W);
  });

  // ── Helpers ───────────────────────────────────────────────────────────────
  function fmtDate(iso: string): string {
    const d = new Date(iso);
    const now = Date.now();
    const diff = Math.floor((now - d.getTime()) / 1000);
    if (diff < 60) return 'just now';
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    if (diff < 86400 * 30) return `${Math.floor(diff / 86400)}d ago`;
    return d.toLocaleDateString([], { month: 'short', day: 'numeric', year: '2-digit' });
  }

  function isTagRef(ref: string): boolean {
    return ref.startsWith('tag: ') || ref.startsWith('tag:');
  }

  function refLabel(ref: string): string {
    return ref.replace(/^tag:\s*/, '');
  }

  // ── Ref-chip classification (GitKraken-style) ─────────────────────────────
  // Decoration strings from `git log %D` arrive as e.g. "HEAD -> main",
  // "HEAD" (detached), "origin/main", "main", "tag: v1.0". We classify each into
  // a chip kind so the row instantly shows what's local vs remote vs the
  // checked-out branch. `RefsResp` (local/remote names) disambiguates a bare
  // name when the decoration alone is ambiguous.
  type ChipKind = 'head' | 'local' | 'remote' | 'tag' | 'detached';
  interface RefChip {
    kind: ChipKind;
    label: string; // text shown on the chip
    current: boolean; // the checked-out branch (most prominent)
  }

  // Set of remote-branch names (e.g. "origin/main") from the refs response, used
  // to classify a decoration token that isn't obviously a tag/HEAD.
  // Snapshot the refs response into plain arrays + lookup sets. Reading `refs`
  // through a local const sidesteps a runes/null-narrowing quirk in svelte-check.
  const refKnowledge = $derived.by(() => {
    const r: RefsResp | null = refs;
    const local = r?.local ?? [];
    const remote = r?.remote ?? [];
    return {
      remoteNames: new Set<string>(remote.map((b) => b.name)),
      localNames: new Set<string>(local.map((b) => b.name)),
      currentBranch: local.find((b) => b.is_current)?.name ?? status.branch ?? null,
    };
  });

  function classifyRef(ref: string): RefChip {
    if (isTagRef(ref)) return { kind: 'tag', label: refLabel(ref), current: false };
    // "HEAD -> branch": the checked-out branch — the most prominent chip.
    const arrow = ref.match(/^HEAD\s*->\s*(.+)$/);
    if (arrow) return { kind: 'head', label: arrow[1].trim(), current: true };
    // A bare "HEAD" decoration = detached HEAD (no branch).
    if (ref === 'HEAD') return { kind: 'detached', label: 'HEAD', current: false };
    // Remote-tracking ref (origin/…): match the refs response, else fall back to
    // the "<remote>/<name>" shape (but never a purely-local name).
    const { remoteNames, localNames, currentBranch } = refKnowledge;
    if (remoteNames.has(ref) || (/^[^/]+\/.+/.test(ref) && !localNames.has(ref))) {
      return { kind: 'remote', label: ref, current: false };
    }
    // Otherwise a local branch; it's "current" if it's the checked-out branch.
    return { kind: 'local', label: ref, current: ref === currentBranch };
  }

  // Chips for one commit row, ordered head → local → remote → tag for a tidy row.
  function chipsFor(c: CommitInfo): RefChip[] {
    const order: Record<ChipKind, number> = { head: 0, detached: 0, local: 1, remote: 2, tag: 3 };
    return c.refs.map(classifyRef).sort((a, b) => order[a.kind] - order[b.kind]);
  }

  // A commit is the HEAD ("you are here") when any decoration is HEAD-ish.
  function isHeadCommit(c: CommitInfo): boolean {
    return c.refs.some((r) => r === 'HEAD' || /^HEAD\s*->/.test(r));
  }

  // ── Inline diff helpers ───────────────────────────────────────────────────
  function changedLinesCount(f: FileDiff): { add: number; del: number } {
    let add = 0;
    let del = 0;
    for (const h of f.hunks) {
      for (const l of h.lines) {
        if (l.origin === 'add') add++;
        else if (l.origin === 'del') del++;
      }
    }
    return { add, del };
  }

  function toggleFileCollapse(path: string): void {
    fileCollapsed = { ...fileCollapsed, [path]: !fileCollapsed[path] };
  }

  function lineClass(origin: DiffLine['origin']): string {
    if (origin === 'add') return 'dl-add';
    if (origin === 'del') return 'dl-del';
    return 'dl-ctx';
  }

  function lineSign(origin: DiffLine['origin']): string {
    if (origin === 'add') return '+';
    if (origin === 'del') return '−';
    return ' ';
  }

  const detailTotals = $derived.by(() => {
    if (!diffResp) return { add: 0, del: 0 };
    let add = 0;
    let del = 0;
    for (const f of diffResp.files) {
      const c = changedLinesCount(f);
      add += c.add;
      del += c.del;
    }
    return { add, del };
  });
</script>

<div class="graphview" class:mobile={isMobile}>
  <!-- ── LEFT: refs tree ───────────────────────────────────────────────────── -->
  {#if isMobile}
    <!-- Mobile accordion header for the branches/refs tree. -->
    <button
      class="mob-sec-head"
      class:open={secRefsOpen}
      onclick={() => (secRefsOpen = !secRefsOpen)}
      aria-expanded={secRefsOpen}
    >
      <Icon name={secRefsOpen ? 'chevronDown' : 'chevronRight'} size={13} />
      <Icon name="branch" size={13} />
      <span>Branches &amp; Tags</span>
      <span class="grow"></span>
      {#if refs}<span class="mob-sec-count">{refs.local.length + refs.remote.length + refs.tags.length}</span>{/if}
    </button>
  {/if}
  <aside class="refs-panel" class:mob-collapsed={isMobile && !secRefsOpen}>
    {#if refsLoading}
      <div style="padding: 10px"><Skeleton rows={6} height={22} /></div>
    {:else if refs}
      <!-- LOCAL -->
      <div class="ref-section">
        <button class="ref-header" onclick={() => (localOpen = !localOpen)}>
          <Icon name={localOpen ? 'chevronDown' : 'chevronRight'} size={11} />
          <Icon name="branch" size={12} />
          <span>LOCAL</span>
          <span class="ref-count">{refs.local.length}</span>
        </button>
        {#if localOpen}
          {#each refs.local as b (b.name)}
            <button
              class="ref-row"
              class:current={b.is_current}
              class:drag-target={dragOverTarget === b.name}
              class:dragging={dragSource?.name === b.name && !dragSource.remote}
              draggable={checkoutBusy === ''}
              ondragstart={(e) => onRefDragStart(e, b.name, false)}
              ondragend={onRefDragEnd}
              ondragover={(e) => onTargetDragOver(e, b.name)}
              ondragleave={() => onTargetDragLeave(b.name)}
              ondrop={(e) => onTargetDrop(e, b.name)}
              disabled={checkoutBusy !== ''}
              onclick={() => !b.is_current && checkout(b.name, false)}
              oncontextmenu={(e) => branchMenu(e, b)}
              title={dragSource && isValidDropTarget(b.name)
                ? `Merge ${dragSourceName()} → ${b.name}`
                : b.name}
            >
              {#if b.is_current}
                <span class="cur-pip" title="Checked out"><Icon name="check" size={9} /></span>
              {:else}
                <Icon name="dot" size={10} />
              {/if}
              <span class="mono ref-name">{b.name}</span>
              {#if b.is_current && (status.ahead > 0 || status.behind > 0)}
                <span class="ref-ab" title="{status.ahead} ahead · {status.behind} behind">
                  {#if status.ahead > 0}<span class="ab-ahead">↑{status.ahead}</span>{/if}
                  {#if status.behind > 0}<span class="ab-behind">↓{status.behind}</span>{/if}
                </span>
              {/if}
              {#if b.upstream}<span class="ref-upstream mono dim" title="upstream: {b.upstream}">{b.upstream}</span>{/if}
              {#if checkoutBusy === b.name}<span class="dim">…</span>{/if}
            </button>
          {:else}
            <div class="dim ref-empty">No local branches</div>
          {/each}
        {/if}
      </div>

      <!-- REMOTE -->
      <div class="ref-section">
        <button class="ref-header" onclick={() => (remoteOpen = !remoteOpen)}>
          <Icon name={remoteOpen ? 'chevronDown' : 'chevronRight'} size={11} />
          <Icon name="globe" size={12} />
          <span>REMOTE</span>
          <span class="ref-count">{refs.remote.length}</span>
        </button>
        {#if remoteOpen}
          {#each refs.remote as b (b.name)}
            <button
              class="ref-row remote"
              class:dragging={dragSource?.name === b.name && dragSource.remote}
              draggable={checkoutBusy === ''}
              ondragstart={(e) => onRefDragStart(e, b.name, true)}
              ondragend={onRefDragEnd}
              disabled={checkoutBusy !== ''}
              onclick={() => checkoutRemote(b)}
              oncontextmenu={(e) => branchMenu(e, b)}
              title="Drag onto a local branch to merge, or click to checkout as a local tracking branch"
            >
              <Icon name="dot" size={10} />
              <span class="mono ref-name">{b.name}</span>
              {#if checkoutBusy === b.name.replace(/^[^/]+\//, '')}<span class="dim">…</span>{/if}
            </button>
          {:else}
            <div class="dim ref-empty">No remote branches</div>
          {/each}
        {/if}
      </div>

      <!-- TAGS -->
      <div class="ref-section">
        <button class="ref-header" onclick={() => (tagsOpen = !tagsOpen)}>
          <Icon name={tagsOpen ? 'chevronDown' : 'chevronRight'} size={11} />
          <Icon name="tag" size={12} />
          <span>TAGS</span>
          <span class="ref-count">{refs.tags.length}</span>
        </button>
        {#if tagsOpen}
          {#each refs.tags as t (t.name)}
            <div
              class="ref-row tag"
              role="button"
              tabindex="0"
              title={t.name}
              oncontextmenu={(e) => tagMenu(e, t)}
            >
              <Icon name="tag" size={10} />
              <span class="mono ref-name">{t.name}</span>
            </div>
          {:else}
            <div class="dim ref-empty">No tags</div>
          {/each}
        {/if}
      </div>
    {/if}
  </aside>

  <!-- ── MIDDLE: commit graph ─────────────────────────────────────────────── -->
  {#if isMobile}
    <button
      class="mob-sec-head"
      class:open={secCommitsOpen}
      onclick={() => (secCommitsOpen = !secCommitsOpen)}
      aria-expanded={secCommitsOpen}
    >
      <Icon name={secCommitsOpen ? 'chevronDown' : 'chevronRight'} size={13} />
      <Icon name="commit" size={13} />
      <span>Commits</span>
      <span class="grow"></span>
      {#if !commitsLoading}<span class="mob-sec-count">{commits.length}</span>{/if}
    </button>
  {/if}
  <div
    class="graph-panel"
    class:panel-shrunk={selectedSha !== null && !isMobile}
    class:mob-collapsed={isMobile && !secCommitsOpen}
  >
    {#if commitsLoading}
      <div style="padding: 10px"><Skeleton rows={12} height={28} /></div>
    {:else if commits.length === 0}
      <div class="dim" style="padding: 18px; font-size: 12px">No commits found.</div>
    {:else}
      <div class="graph-list">
        {#each laneRows as row (row.commit.sha)}
          {@const svgW = gutterWidth}
          {@const cx = row.col * LANE_W + LANE_W / 2}
          {@const cy = 14}
          {@const totalH = 28}
          {@const isSelected = selectedSha === row.commit.sha}
          {@const isHead = isHeadCommit(row.commit)}
          <button
            class="graph-row"
            class:graph-row-selected={isSelected}
            class:graph-row-head={isHead}
            onclick={() => selectCommit(row.commit)}
            oncontextmenu={(e) => commitMenu(e, row.commit)}
            title={isHead ? `${row.commit.subject} — you are here (HEAD)` : row.commit.subject}
            aria-pressed={isSelected}
          >
            <!-- SVG gutter -->
            <svg
              class="gutter"
              width={svgW}
              height={totalH}
              style="flex-shrink: 0; width: {svgW}px;"
            >
              <!-- draw lane lines (background, behind node) -->
              {#each row.lines as line}
                {@const x1 = line.fromCol * LANE_W + LANE_W / 2}
                {@const x2 = line.toCol * LANE_W + LANE_W / 2}
                {#if line.kind === 'vert'}
                  <line x1={x1} y1={0} x2={x1} y2={totalH} stroke={line.color} stroke-width="1.5" />
                {:else if line.kind === 'converge'}
                  <!-- converge: a lane from above (fromCol) folding into this node -->
                  <path
                    d="M{x1},0 Q{x1},{cy - 10} {cx},{cy}"
                    stroke={line.color}
                    stroke-width="1.5"
                    fill="none"
                  />
                {:else}
                  <!-- merge-in: curved path from node down to target column -->
                  <path
                    d="M{cx},{cy} Q{cx},{cy + 10} {x2},{totalH}"
                    stroke={line.color}
                    stroke-width="1.5"
                    fill="none"
                  />
                {/if}
              {/each}
              <!-- vertical continuation for current lane -->
              <line x1={cx} y1={cy + NODE_R} x2={cx} y2={totalH} stroke={row.color} stroke-width="1.5" />
              <!-- node circle -->
              <circle cx={cx} cy={cy} r={NODE_R} fill={row.color} />
            </svg>

            <!-- commit info -->
            <div class="commit-info">
              <div class="ci-top">
                {#if isHead}
                  <span class="here-pip" title="You are here (HEAD)" aria-label="HEAD">
                    <Icon name="check" size={8} />
                  </span>
                {/if}
                <!-- Ref chips sit on the LEFT, attached to the graph node (like
                     GitKraken's BRANCH/TAG column) so the checked-out branch reads
                     on the graph itself, not floated to the far right. -->
                {#each chipsFor(row.commit) as chip}
                  <span
                    class="ref-chip kind-{chip.kind}"
                    class:current-chip={chip.current}
                    title={chip.current ? `Checked out · ${chip.label}` : chip.label}
                  >
                    {#if chip.current}<Icon name="check" size={8} />{/if}
                    {#if chip.kind === 'remote'}<Icon name="globe" size={8} />{/if}
                    {#if chip.kind === 'tag'}<Icon name="tag" size={8} />{/if}
                    {chip.label}
                    {#if chip.current && (status.ahead > 0 || status.behind > 0)}
                      <span class="chip-ab">
                        {#if status.ahead > 0}<span class="ab-ahead">↑{status.ahead}</span>{/if}
                        {#if status.behind > 0}<span class="ab-behind">↓{status.behind}</span>{/if}
                      </span>
                    {/if}
                  </span>
                {/each}
                <span class="ci-subject">{row.commit.subject}</span>
              </div>
              <div class="ci-meta">
                <span class="mono ci-sha">{row.commit.short_sha}</span>
                <span class="dim ci-author">{row.commit.author}</span>
                <span class="grow"></span>
                <span class="dim ci-date">{fmtDate(row.commit.date)}</span>
              </div>
            </div>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <!-- Create-PR sheet, opened from a branch/commit context menu. -->
  {#if createPrOpen}
    <CreatePr
      {repoId}
      initialSource={createPrSource}
      onclose={() => (createPrOpen = false)}
      oncreated={() => {
        createPrOpen = false;
        void refreshAfter();
      }}
    />
  {/if}

  <!-- ── RIGHT: commit detail + diff ─────────────────────────────────────── -->
  {#if isMobile && selectedSha !== null}
    <!-- Mobile: a sticky section header for the open commit's diff, with a tap
         target to close it (re-opening the commit list). -->
    <button class="mob-sec-head mob-diff-head" onclick={clearSelection} aria-expanded="true">
      <Icon name="chevronDown" size={13} />
      <Icon name="file" size={13} />
      <span class="mob-diff-title mono">{selectedCommit?.short_sha ?? 'Diff'}</span>
      <span class="grow"></span>
      {#if diffResp}<span class="mob-sec-count">{diffResp.files.length} file{diffResp.files.length === 1 ? '' : 's'}</span>{/if}
      <span class="mob-close">✕</span>
    </button>
  {/if}
  <div
    class="detail-panel"
    class:detail-visible={selectedSha !== null && !isMobile}
    class:mob-visible={isMobile && selectedSha !== null}
    class:mob-hidden={isMobile && selectedSha === null}
  >
    {#if selectedSha === null}
      <div class="detail-empty">
        <span class="detail-empty-label">COMMIT</span>
        <span class="dim detail-empty-hint">select a commit</span>
      </div>
    {:else if selectedCommit !== null}
      <!-- Header -->
      <div class="detail-header">
        <div class="detail-header-main">
          <div class="detail-title-row">
            <span class="mono detail-sha">{selectedCommit.short_sha}</span>
            {#each chipsFor(selectedCommit) as chip}
              <span class="ref-chip kind-{chip.kind}" class:current-chip={chip.current}>
                {#if chip.current}<Icon name="check" size={8} />{/if}
                {#if chip.kind === 'remote'}<Icon name="globe" size={8} />{/if}
                {#if chip.kind === 'tag'}<Icon name="tag" size={8} />{/if}
                {chip.label}
              </span>
            {/each}
            <span class="grow"></span>
            <button class="detail-close" onclick={clearSelection} title="Close" aria-label="Close commit detail">
              ✕
            </button>
          </div>
          <div class="detail-subject">{selectedCommit.subject}</div>
          <div class="detail-meta">
            <span class="detail-author">{selectedCommit.author}</span>
            <span class="dim detail-dot">·</span>
            <span class="dim detail-date">{fmtDate(selectedCommit.date)}</span>
          </div>
        </div>
      </div>

      <!-- Diff area -->
      <div class="detail-diff">
        {#if diffLoading}
          <div class="detail-diff-loading">
            <Skeleton rows={8} height={20} />
          </div>
        {:else if diffResp !== null}
          {#if diffResp.files.length === 0}
            <div class="dim" style="padding: 18px; font-size: 12px; text-align: center">No file changes.</div>
          {:else}
            <!-- Diff summary bar -->
            <div class="diff-summary-bar">
              <span class="dim" style="font-size: 11px">{diffResp.files.length} file{diffResp.files.length === 1 ? '' : 's'}</span>
              <span class="ds-add">+{detailTotals.add}</span>
              <span class="ds-del">−{detailTotals.del}</span>
            </div>

            <!-- Per-file diff -->
            {#each diffResp.files as file (file.path)}
              {@const stats = changedLinesCount(file)}
              {@const isCollapsed = fileCollapsed[file.path] ?? false}
              <div class="df-block">
                <!-- File header -->
                <button
                  class="df-head"
                  onclick={() => toggleFileCollapse(file.path)}
                  title={file.path}
                >
                  <span class="df-chevron dim">
                    {isCollapsed ? '▶' : '▼'}
                  </span>
                  <span class="mono df-path">
                    {#if file.old_path}{file.old_path} → {/if}{file.path}
                  </span>
                  <span class="grow"></span>
                  <span class="ds-add">+{stats.add}</span>
                  <span class="ds-del">−{stats.del}</span>
                </button>

                {#if !isCollapsed}
                  {#if file.is_binary}
                    <div class="df-binary dim">Binary file — no text diff.</div>
                  {:else}
                    <div class="df-hunks">
                      {#each file.hunks as hunk, hi (hi)}
                        <div class="hunk-header mono">{hunk.header}</div>
                        <table class="dl-table">
                          <tbody>
                            {#each hunk.lines as line, li (li)}
                              <tr class="dl-row {lineClass(line.origin)}">
                                <td class="dl-gut dl-old">{line.old_line ?? ''}</td>
                                <td class="dl-gut dl-new">{line.new_line ?? ''}</td>
                                <td class="dl-sign">{lineSign(line.origin)}</td>
                                <td class="dl-code mono">{line.content}</td>
                              </tr>
                            {/each}
                          </tbody>
                        </table>
                      {/each}
                    </div>
                  {/if}
                {/if}
              </div>
            {/each}
          {/if}
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .graphview {
    display: flex;
    height: 100%;
    min-height: 0;
  }

  /* ── refs panel ── */
  .refs-panel {
    width: 220px;
    flex-shrink: 0;
    overflow-y: auto;
    border-inline-end: 1px solid var(--border);
    padding: 6px 0;
  }
  .ref-section {
    margin-bottom: 2px;
  }
  .ref-header {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 5px 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    cursor: pointer;
    text-transform: uppercase;
    text-align: start;
    transition: color 110ms ease-out;
  }
  .ref-header:hover {
    color: var(--text);
  }
  .ref-count {
    margin-inline-start: auto;
    background: var(--surface-2);
    border-radius: 999px;
    font-size: 9.5px;
    padding: 1px 5px;
    font-weight: 600;
    letter-spacing: 0;
  }
  .ref-row {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    height: 24px;
    padding: 0 10px 0 22px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    text-align: start;
    overflow: hidden;
    transition: background 100ms ease-out, color 100ms ease-out;
  }
  .ref-row:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }
  .ref-row:disabled {
    cursor: default;
  }
  .ref-row.current {
    color: var(--accent);
    font-weight: 600;
  }
  .ref-row[draggable='true'] {
    cursor: grab;
  }
  .ref-row.dragging {
    opacity: 0.45;
  }
  /* Accent outline while a valid merge source hovers this local branch. */
  .ref-row.drag-target {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--text);
    outline: 1.5px solid var(--accent);
    outline-offset: -1.5px;
    border-radius: var(--radius-s);
  }
  .ref-row.tag {
    cursor: default;
    color: var(--text-dim);
  }
  .ref-row.remote {
    color: var(--text-dim);
  }
  .ref-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
  }
  /* Left-panel current-branch marker: a filled accent check pip. */
  .cur-pip {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--accent);
    color: #fff;
  }
  .ref-ab {
    display: inline-flex;
    gap: 3px;
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .ref-upstream {
    flex-shrink: 0;
    font-size: 9.5px;
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ref-empty {
    padding: 4px 22px 6px;
    font-size: 11px;
  }

  /* ── graph panel ── */
  .graph-panel {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    overflow-x: auto;
    transition: flex 180ms ease-out;
  }
  /* When detail is open, the commit list becomes a fixed-width column and the
     detail panel flexes to fill the rest of the page (see .detail-visible). */
  .graph-panel.panel-shrunk {
    flex: 0 0 380px;
    width: auto;
    min-width: 280px;
    border-inline-end: 1px solid var(--border);
  }
  .graph-list {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .graph-row {
    display: flex;
    align-items: center;
    width: 100%;
    height: 28px;
    padding-inline-end: 12px;
    border: none;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
    background: transparent;
    cursor: pointer;
    text-align: start;
    transition: background 100ms ease-out;
  }
  .graph-row:hover {
    background: var(--surface-2);
  }
  .graph-row-selected,
  .graph-row-selected:hover {
    /* Clear selected state: accent wash + inset accent bar (inset avoids a
       layout shift that a left border would cause on the selected row only). */
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    box-shadow: inset 2px 0 0 0 var(--accent);
  }
  .graph-row-selected .ci-subject {
    color: var(--accent);
    font-weight: 600;
  }
  /* The HEAD commit ("you are here") — a left accent rail + faint wash so the
     checked-out tip is obvious at a glance, even when not selected. */
  .graph-row-head:not(.graph-row-selected) {
    background: color-mix(in srgb, var(--accent) 7%, transparent);
    box-shadow: inset 2px 0 0 0 color-mix(in srgb, var(--accent) 70%, transparent);
  }
  .gutter {
    display: block;
    flex-shrink: 0;
    overflow: visible;
  }
  .commit-info {
    flex: 1;
    min-width: 0;
    padding: 0 8px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 1px;
  }
  .ci-top {
    display: flex;
    align-items: center;
    gap: 5px;
    overflow: hidden;
  }
  .ci-subject {
    font-size: 12px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  /* ── Color-coded ref chips (classified local / remote / tag / HEAD) ── */
  .ref-chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
    font-size: 9.5px;
    font-weight: 600;
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--surface-2);
    color: var(--text-dim);
    border: 1px solid transparent;
    white-space: nowrap;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Local branch — subtle, neutral. */
  .ref-chip.kind-local {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
  }
  /* Remote-tracking branch — distinct teal/cyan so it never reads as local. */
  .ref-chip.kind-remote {
    background: color-mix(in srgb, #56b6c2 18%, transparent);
    color: #2a8d99;
  }
  /* Tag — gold. */
  .ref-chip.kind-tag {
    background: color-mix(in srgb, #e5c07b 22%, transparent);
    color: #b8860b;
  }
  /* Detached HEAD — muted warning. */
  .ref-chip.kind-detached {
    background: color-mix(in srgb, #e06c75 18%, transparent);
    color: #c0392b;
  }
  /* The checked-out branch — the most prominent chip (filled accent). */
  .ref-chip.kind-head,
  .ref-chip.current-chip {
    background: var(--accent);
    color: #fff;
    border-color: color-mix(in srgb, var(--accent) 60%, #000);
  }
  .chip-ab {
    display: inline-flex;
    gap: 2px;
    margin-inline-start: 2px;
    padding-inline-start: 3px;
    border-inline-start: 1px solid color-mix(in srgb, #fff 45%, transparent);
    font-variant-numeric: tabular-nums;
  }
  /* "You are here" pip on the HEAD row. */
  .here-pip {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--accent);
    color: #fff;
  }
  .ab-ahead {
    color: var(--status-working, #98c379);
  }
  .ab-behind {
    color: var(--status-exited, #e06c75);
  }
  .current-chip .ab-ahead,
  .current-chip .ab-behind,
  .kind-head .ab-ahead,
  .kind-head .ab-behind {
    color: #fff;
  }
  .ci-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 10px;
  }
  .ci-sha {
    color: var(--accent);
    font-size: 10px;
  }
  .ci-author {
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ci-date {
    font-size: 10px;
    white-space: nowrap;
  }

  /* ── detail panel ── */
  .detail-panel {
    width: 0;
    min-width: 0;
    flex-shrink: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    transition: width 180ms ease-out;
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
  }
  .detail-panel.detail-visible {
    /* Fill all remaining width after refs + commit-list so the page isn't
       half-empty when a commit is open. */
    flex: 1 1 0;
    width: auto;
    min-width: 360px;
    overflow: hidden;
  }
  .detail-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 6px;
  }
  .detail-empty-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .detail-empty-hint {
    font-size: 12px;
  }

  /* Detail header */
  .detail-header {
    flex-shrink: 0;
    padding: 10px 12px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
  }
  .detail-header-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .detail-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .detail-sha {
    font-size: 11px;
    color: var(--accent);
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .detail-subject {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    line-height: 1.35;
    word-break: break-word;
  }
  .detail-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
  }
  .detail-author {
    color: var(--text-dim);
    font-size: 11px;
  }
  .detail-dot {
    font-size: 10px;
  }
  .detail-date {
    font-size: 11px;
  }
  .detail-close {
    flex-shrink: 0;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1;
    padding: 2px 4px;
    border-radius: var(--radius-s, 3px);
    margin-inline-start: auto;
    transition: color 100ms, background 100ms;
  }
  .detail-close:hover {
    color: var(--text);
    background: var(--surface-2);
  }

  /* Diff area */
  .detail-diff {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .detail-diff-loading {
    padding: 12px;
  }
  .diff-summary-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 11px;
    position: sticky;
    top: 0;
    z-index: 1;
  }
  .ds-add {
    color: var(--status-working);
    font-weight: 600;
    font-size: 11px;
  }
  .ds-del {
    color: var(--status-exited);
    font-weight: 600;
    font-size: 11px;
  }

  /* Per-file diff block */
  .df-block {
    border-bottom: 1px solid var(--border);
  }
  .df-head {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 5px 10px;
    border: none;
    background: var(--surface-2);
    cursor: pointer;
    font-size: 11px;
    color: var(--text);
    text-align: start;
    transition: background 80ms;
  }
  .df-head:hover {
    background: color-mix(in srgb, var(--accent) 7%, var(--surface-2));
  }
  .df-chevron {
    font-size: 8px;
    flex-shrink: 0;
  }
  .df-path {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
    color: var(--text);
  }
  .df-binary {
    padding: 10px 12px;
    font-size: 11.5px;
  }
  .df-hunks {
    overflow-x: auto;
  }

  /* Hunk header */
  .hunk-header {
    padding: 2px 10px;
    font-size: 10px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }

  /* Diff line table */
  .dl-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
    line-height: 1.5;
  }
  .dl-gut {
    width: 34px;
    min-width: 34px;
    text-align: end;
    padding: 0 5px 0 3px;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: 9.5px;
    user-select: none;
    vertical-align: top;
    border-inline-end: 1px solid var(--border);
  }
  .dl-sign {
    width: 14px;
    text-align: center;
    color: var(--text-dim);
    user-select: none;
    font-family: var(--font-mono);
    font-size: 11px;
    vertical-align: top;
    padding: 0 1px;
  }
  .dl-code {
    padding: 0 8px 0 3px;
    white-space: pre;
    word-break: normal;
    user-select: text;
    font-size: 11px;
    font-family: var(--font-mono);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  tr.dl-row.dl-add {
    background: color-mix(in srgb, var(--status-working) 11%, transparent);
  }
  tr.dl-row.dl-add .dl-sign {
    color: var(--status-working);
  }
  tr.dl-row.dl-del {
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
  }
  tr.dl-row.dl-del .dl-sign {
    color: var(--status-exited);
  }
  tr.dl-row.dl-ctx {
    /* context lines: slightly dimmed */
    color: var(--text-dim);
  }

  /* Shared utilities */
  .grow {
    flex: 1;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ── Mobile accordion section headers (phones) ─────────────────────────────
     Each pane (Branches / Commits / Diff) gets a tappable header; the body
     below collapses or expands and scrolls independently. */
  .mob-sec-head {
    display: none;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 11px 14px;
    border: none;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    text-align: start;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .mob-sec-head:active {
    background: color-mix(in srgb, var(--accent) 10%, var(--surface-2));
  }
  .mob-sec-count {
    font-size: 11px;
    font-weight: 700;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
  }
  .mob-diff-head {
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-2));
    position: sticky;
    top: 0;
    z-index: 3;
  }
  .mob-diff-title {
    font-size: 13px;
    color: var(--accent);
    font-weight: 700;
  }
  .mob-close {
    font-size: 16px;
    color: var(--text-dim);
    line-height: 1;
    padding-inline-start: 6px;
  }

  /* ── Mobile + tablet (≤1024px) layout: vertical accordion ── */
  @media (max-width: 1024px) {
    .graphview.mobile {
      flex-direction: column;
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
    .mobile .mob-sec-head { display: flex; }

    /* Branches/refs pane: capped height, scrolls on its own when expanded. */
    .mobile .refs-panel {
      width: 100%;
      flex: 0 0 auto;
      max-height: 38vh;
      overflow-y: auto;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    /* Commit list: takes the remaining room, scrolls on its own. */
    .mobile .graph-panel,
    .mobile .graph-panel.panel-shrunk {
      flex: 1 1 auto;
      min-width: 0;
      width: auto;
      min-height: 180px;
      overflow-y: auto;
      overflow-x: auto;
      border-inline-end: none;
    }
    /* Diff/detail pane: the prominent section once a commit is open. */
    .mobile .detail-panel {
      width: 100%;
      border-inline-start: none;
    }
    .mobile .detail-panel.mob-visible {
      flex: 1 1 auto;
      min-width: 0;
      min-height: 50vh;
      width: 100%;
      overflow: hidden;
      display: flex;
    }

    /* A collapsed section hides its body entirely (header stays tappable). */
    .mobile .mob-collapsed {
      display: none !important;
    }
    /* No commit selected → drop the empty diff pane so the list fills the view. */
    .mobile .detail-panel.mob-hidden {
      display: none !important;
    }

    /* Bigger touch targets + legible text on the commit rows. */
    .mobile .graph-row { height: 46px; }
    .mobile .ci-subject { font-size: 14px; }
    .mobile .ci-meta { font-size: 12px; }
    .mobile .ci-sha { font-size: 12px; }
    .mobile .ci-author { font-size: 12px; max-width: 110px; }
    .mobile .ci-date { font-size: 12px; }
    .mobile .ref-chip { font-size: 11px; max-width: 130px; }

    /* Refs rows: bigger tap targets + legible text. */
    .mobile .ref-row { height: 36px; font-size: 14px; }
    .mobile .ref-name { font-size: 13px; }
    .mobile .ref-header { font-size: 12px; padding: 8px 12px; }

    /* Diff: bump the tiny code + gutter text so it's legible on a phone. */
    .mobile .detail-diff { -webkit-overflow-scrolling: touch; }
    .mobile .diff-summary-bar { font-size: 13px; padding: 9px 12px; }
    .mobile .diff-summary-bar .dim { font-size: 13px !important; }
    .mobile .ds-add,
    .mobile .ds-del { font-size: 13px; }
    .mobile .df-head { font-size: 13px; padding: 9px 12px; }
    .mobile .df-path { font-size: 13px; }
    .mobile .hunk-header { font-size: 12px; padding: 4px 10px; }
    /* table-layout:fixed pins the gutter/sign columns to their declared widths
       and hands the rest to the code column, so a long unbroken line wraps
       INSIDE that column instead of widening the table past the viewport (the
       auto layout otherwise sizes to the content's min-width and overflows). */
    .mobile .df-hunks { overflow-x: hidden; }
    .mobile .dl-table { font-size: 12.5px; table-layout: fixed; width: 100%; }
    /* Wrap long code lines so they're readable without horizontal scrolling.
       break-word keeps whole words together when they fit; overflow-wrap +
       a width:auto cell let a 140-char unbroken token still break to fit. */
    .mobile .dl-code {
      font-size: 12.5px;
      width: auto;
      white-space: pre-wrap;
      word-break: break-word;
      overflow-wrap: anywhere;
      overflow: visible;
      text-overflow: clip;
    }
    .mobile .dl-gut { font-size: 11px; width: 30px; min-width: 30px; }
    .mobile .dl-sign { font-size: 12.5px; }
    .mobile .detail-subject { font-size: 14px; }
    .mobile .detail-sha { font-size: 12px; }
    .mobile .detail-author,
    .mobile .detail-date { font-size: 12px; }
  }
</style>
