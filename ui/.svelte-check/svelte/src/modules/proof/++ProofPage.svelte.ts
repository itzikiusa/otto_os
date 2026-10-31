///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
import PathField from '../../lib/components/PathField.svelte';
// Proof section: a two-pane viewer of proof packs. Left = status filter chips
// + the pack list; right = the open pack's detail (badges, artifacts grouped
// by kind, and assemble / add-artifact / waive / delete actions).
import { untrack } from 'svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import { initialSelection, rememberSelection } from '../../lib/lastSelection';
import ProofBadges from '../../lib/components/ProofBadges.svelte';
import ProofStatusChip from '../../lib/components/ProofStatusChip.svelte';
import DoneContractMeter from '../../lib/components/DoneContractMeter.svelte';
import ProofSnapshotList from '../../lib/components/ProofSnapshotList.svelte';
import { proof } from '../../lib/stores/proof.svelte';
import { addArtifact, archiveStaleSessionPacks, artifactBlobUrl, artifactContent, assembleProof, attachApiEvidence, attachDbEvidence, attachKafkaEvidence, attachMediaRaw,
    ciRefresh, createProofPack, deleteArtifact, deleteProofPack, getRepoProofConfig, proofReport, runPrCheck, setRepoProofConfig, waiveProof } from '../../lib/api/proof';
import { downloadText } from '../../lib/components/exporters';
import { ws } from '../../lib/stores/workspace.svelte';
import { api } from '../../lib/api/client';
import { ui } from '../../lib/stores/ui.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { registry } from '../../lib/commands.svelte';
import { router } from '../../lib/router.svelte';
import type { AssembleResp, ProofArtifactView, Repo } from '../../lib/api/types';
import { announce } from '../../lib/announce.svelte';
import { assembleOutcome } from './assembleOutcome';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { sentenceCase } from '../../lib/status';
import { rel } from '../../lib/stores/now.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  /** Enum → words for kinds and statuses (self_review → "Self review",
   *  pr_check → "PR check", ci → "CI", api → "API", db → "Database"). */
  const WORDS: Record<string, string> = { ci: 'CI', api: 'API', db: 'Database', pr_check: 'PR check', self_review: 'Self review', all: 'All' };
  function words(raw: string | null | undefined): string {
    const k = (raw ?? '').trim();
    return WORDS[k] ?? sentenceCase(k || 'unknown');
  }

  const MEDIA_KINDS = new Set(['screenshot', 'video']);

  const STATUS_FILTERS = ['all', 'passed', 'failed', 'partial', 'missing', 'waived'] as const;
  type StatusFilter = (typeof STATUS_FILTERS)[number];
  let filter = $state<StatusFilter>('all');

  const ARTIFACT_KINDS = [
    'command', 'log', 'screenshot', 'diff', 'ci', 'api', 'db', 'review', 'approval', 'self_review',
  ];
  const ARTIFACT_STATUSES = ['info', 'passed', 'failed', 'pending'];

  // Load the list (for the active filter) + the summary roll-up for this ws.
  // A failed load lands in `proof.error` (never an empty list), so the page
  // shows it inline with Retry instead of "No proof packs yet".
  const listError = $derived(proof.error);
  let listLoaded = $state(false);
  async function loadList(id: string, f: StatusFilter): Promise<void> {
    await proof.loadPacks(id, f === 'all' ? undefined : { status: f });
    listLoaded = true;
  }
  function retryList(): void {
    if (ws.currentId) void loadList(ws.currentId, filter);
  }
  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    const f = filter;
    void loadList(id, f);
  });

  // Event-driven list/detail refreshes run only while this page is mounted.
  $effect(() => proof.watch());

  const detail = $derived(proof.detail);

  // Never open onto an empty "select a pack" pane when packs exist: restore the
  // last-opened pack (or the first) once per workspace load. Not on a phone,
  // where an open pack replaces the list.
  let autoPickedFor = $state<string | null>(null);
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || proof.wsId !== wsId || autoPickedFor === wsId || viewport.isPhone) return;
    if (proof.detail) {
      autoPickedFor = wsId;
      return;
    }
    // A deep link opens below. A workspace switch clears the previous link;
    // observe that change and only mark selection complete once a pack opens.
    if (routePackId()) return;
    if (proof.loading || proof.packs.length === 0) return;
    autoPickedFor = wsId;
    const id = initialSelection('proof', proof.packs, (p) => p.id);
    if (id) open(id);
  });

  /** The pack the URL names (`#/proof/<id>`), or null. */
  function routePackId(): string | null {
    const [mod, id] = router.parts;
    return mod === 'proof' && id ? id : null;
  }
  // The URL carries the open pack so a reload / share / notification lands on
  // it; the last pick is remembered as the fallback. (Nothing open yet must
  // not strip a deep link: only a CLOSE clears the URL.)
  // The store keeps the last pack across visits; on arrival a link naming a
  // DIFFERENT pack wins (the effect below opens it) instead of being replaced.
  let routedId: string | null = null;
  let arrived = false;
  $effect(() => {
    const id = detail?.pack.id ?? null;
    if (id) rememberSelection('proof', id);
    const was = routedId;
    routedId = id;
    const first = !arrived;
    arrived = true;
    if (!id && !was) return;
    if (first && untrack(() => routePackId()) && untrack(() => routePackId()) !== id) return;
    untrack(() => {
      if (router.module === 'proof' && routePackId() !== id) router.replace(id ? `proof/${id}` : 'proof');
    });
  });
  // A route change while the page is up (a notification, Back/Forward, a
  // pasted link) opens that pack.
  $effect(() => {
    const linked = routePackId();
    // On a cold deep link the route arrives before workspace bootstrap. Opening
    // then would be invalidated by loadPacks' workspace reset, with no route
    // change left to retry it. Follow workspace readiness as well as the URL.
    const workspaceId = ws.currentId;
    if (!linked || !workspaceId || proof.wsId !== workspaceId) return;
    untrack(() => {
      if (linked !== proof.detail?.pack.id && linked !== openingId) void open(linked);
    });
  });
  // The pack list's width — drag / ←→ on the divider, remembered across visits.
  let listW = $state(loadPaneWidth('proof.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  // An empty, unfiltered list has nothing to show: the page-level empty state
  // owns the page (a filtered-empty list keeps the rail so the filter can change).
  const showRail = $derived(proof.packs.length > 0 || filter !== 'all');
  const filterCounts = $derived.by(() => {
    // Counts only mean something on the unfiltered list.
    if (filter !== 'all' || proof.nextCursor) return null;
    const c: Record<string, number> = { all: proof.packs.length };
    for (const p of proof.packs) c[p.status] = (c[p.status] ?? 0) + 1;
    return c;
  });

  // "12 packs · 7 passed · 3 failed" — what the empty detail pane says instead of
  // a bare "pick one" (only on the unfiltered list, where the counts are true).
  const packSummary = $derived.by(() => {
    const c = filterCounts;
    if (!c || !c.all) return '';
    const parts = STATUS_FILTERS.filter((f) => f !== 'all' && (c[f] ?? 0) > 0).map((f) => `${c[f]} ${f}`);
    return `${plural(c.all, 'pack')}${parts.length ? ` · ${parts.join(' · ')}` : ''}. `;
  });

  // Group the open pack's artifacts by kind for display.
  const artifactGroups = $derived.by((): [string, ProofArtifactView[]][] => {
    if (!detail) return [];
    const by: Record<string, ProofArtifactView[]> = {};
    for (const a of detail.artifacts) (by[a.kind] ??= []).push(a);
    return Object.entries(by);
  });

  // Per-artifact: previews are expandable, and "Load full" pulls uncapped content.
  let expanded = $state<Record<string, boolean>>({});
  let fullContent = $state<Record<string, string>>({});

  function toggleExpand(id: string): void {
    expanded[id] = !expanded[id];
  }

  /** One line of a PR-consistency report (mirrors PrConsistencyCheck). */
  interface PrCheckLine {
    label: string;
    passed: boolean;
    detail: string;
  }
  /** The PR-consistency report a `pr_check` artifact carries in `metadata.report`. */
  interface PrCheckReport {
    score: number;
    passed: boolean;
    hard_fail: boolean;
    checks: PrCheckLine[];
  }
  /** Extract the structured PR-consistency report from a pr_check artifact's
   *  metadata, or null when it isn't present/shaped as expected. */
  function prReport(meta: unknown): PrCheckReport | null {
    const r = (meta as { report?: unknown } | null | undefined)?.report as
      | PrCheckReport
      | undefined;
    return r && Array.isArray(r.checks) ? r : null;
  }

  async function loadFull(id: string): Promise<void> {
    try {
      const c = await artifactContent(id);
      fullContent[id] = c.content ?? '(no content)';
      expanded[id] = true;
    } catch (e) {
      toasts.error("Couldn’t load the artifact", loadErrorText(e));
    }
  }

  // Opens a pack into the detail pane. A failure lands in `proof.detailError`
  // and shows inline in the pane (with Retry), never as a vanished click.
  let openingId = $state<string | null>(null);
  let lastOpenId: string | null = null;
  async function open(id: string): Promise<void> {
    openingId = id;
    lastOpenId = id;
    try {
      await proof.open(id);
    } finally {
      if (openingId === id) openingId = null;
    }
  }
  function retryOpen(): void {
    const id = lastOpenId ?? detail?.pack.id;
    if (id) void open(id);
  }

  // ---- inline media (R4): object URLs for screenshot/video artifacts --------
  // Keyed by artifact id. The first effect (re)syncs the set against the open
  // pack; the second revokes every URL on unmount. `untrack` keeps the effect's
  // only dependency `detail` — reads/writes of `mediaUrls` must not re-trigger it.
  let mediaUrls = $state<Record<string, string>>({});
  let mediaErrors = $state<Record<string, string>>({});

  $effect(() => {
    const wanted = (detail?.artifacts ?? [])
      .filter((a) => MEDIA_KINDS.has(a.kind))
      .map((a) => a.id);
    untrack(() => syncMedia(wanted));
  });

  $effect(() => () => untrack(() => {
    for (const url of Object.values(mediaUrls)) URL.revokeObjectURL(url);
    // A fetch still in flight lands AFTER this: an empty wanted-set makes it
    // revoke its URL instead of parking a ≤25 MiB blob on a dead instance.
    wantedMedia = new Set();
  }));

  // Media is fetched LAZILY (backlog B6 / SE-16): a screenshot when its
  // placeholder scrolls into view, a video only on "Load video" — opening a
  // pack used to pull every blob (≤25 MiB each) up front.
  let wantedMedia = new Set<string>();
  const mediaInflight = new Set<string>();
  function syncMedia(wanted: string[]): void {
    const set = new Set(wanted);
    wantedMedia = set;
    const stale = Object.keys(mediaUrls).some((id) => !set.has(id));
    if (stale) {
      const next: Record<string, string> = {};
      for (const [id, url] of Object.entries(mediaUrls)) {
        if (set.has(id)) next[id] = url;
        else URL.revokeObjectURL(url);
      }
      mediaUrls = next;
    }
  }

  async function fetchMedia(id: string): Promise<void> {
    if (id in mediaUrls || mediaInflight.has(id)) return;
    mediaInflight.add(id);
    delete mediaErrors[id];
    try {
      const url = await artifactBlobUrl(id);
      // The pack was switched (or the artifact removed) while it loaded:
      // revoke instead of leaking the object URL until the page unmounts.
      if (!wantedMedia.has(id)) URL.revokeObjectURL(url);
      else mediaUrls[id] = url;
    } catch (e) {
      if (wantedMedia.has(id)) {
        mediaErrors[id] = loadErrorText(e);
        // Spoken here, at the edge: the inline error below is plain text.
        announce(`Couldn’t load the media. ${mediaErrors[id]}`, { assertive: true });
      }
    } finally {
      mediaInflight.delete(id);
    }
  }

  /** Svelte action: run `load` once the node is (nearly) on screen. */
  function whenVisible(node: HTMLElement, load: () => void) {
    if (typeof IntersectionObserver === 'undefined') {
      load();
      return {};
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          io.disconnect();
          load();
        }
      },
      { rootMargin: '200px' },
    );
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  // ---- Add-artifact modal --------------------------------------------------
  let addOpen = $state(false);
  let aKind = $state('command');
  let aTitle = $state('');
  let aContent = $state('');
  let aStatus = $state('info');

  function resetAdd(): void {
    aKind = 'command';
    aTitle = '';
    aContent = '';
    aStatus = 'info';
  }

  async function submitAdd(): Promise<void> {
    if (!detail || !aTitle.trim()) return;
    try {
      await addArtifact(detail.pack.id, {
        kind: aKind,
        title: aTitle.trim(),
        content: aContent.trim() || undefined,
        status: aStatus,
      });
      await proof.refreshDetail();
      addOpen = false;
      resetAdd();
    } catch (e) {
      toasts.error("Couldn’t add the artifact", loadErrorText(e));
    }
  }

  async function removeArtifact(id: string): Promise<void> {
    if (!detail) return;
    const name = detail.artifacts.find((x) => x.id === id)?.title;
    if (!(await confirmer.ask(name ? `Delete the artifact “${name}” from this proof pack? The pack’s status is re-derived without it.` : 'Delete this artifact from the proof pack?', { title: 'Delete artifact' }))) return;
    try {
      await deleteArtifact(id);
      await proof.refreshDetail();
    } catch (e) {
      toasts.error("Couldn’t delete the artifact", loadErrorText(e));
    }
  }

  // ---- pack-level actions --------------------------------------------------
  // Anchored under the button (not at the cursor); ctxMenu clamps it. When
  // the button is collapsed into the header's ⋯ menu, showAt falls back to
  // that ⋯ button.
  function openAddMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'Add artifact…', icon: 'plus', action: () => { resetAdd(); addOpen = true; } },
      { label: 'Add media…', icon: 'file', action: () => { resetMedia(); mediaOpen = true; } },
      { label: 'Add evidence…', icon: 'db', action: () => { resetEvidence(); evidenceOpen = true; } },
      { label: 'PR check…', icon: 'pr', action: () => { resetPr(); prOpen = true; void packRepoPath().then(({ path, error }) => { if (!prOpen) return; if (!prCwd) prCwd = path; prRepoError = error; if (error) announce(`Couldn’t prefill the pack’s repository folder: ${error}`); }); } },
    ]);
  }

  function openMoreMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      ...(detail
        ? [
            { label: 'Waive…', icon: 'check', title: 'Record an approved exception for this pack', action: () => { waiveReason = ''; waiveOpen = true; } },
            ...(detail.pack.repo_id ? [{ label: ciRefreshing ? 'Refreshing CI…' : 'Refresh CI', icon: 'fetch', disabled: ciRefreshing, action: () => void refreshCi() }] : []),
            { separator: true },
          ]
        : []),
      { label: 'Archive stale session packs…', icon: 'archive', title: 'Hides, never deletes', action: () => void archiveStaleSessions() },
      ...(detail ? [{ separator: true }, { label: 'Delete pack…', icon: 'trash', danger: true, action: () => void removePack() }] : []),
    ]);
  }

  // Long actions (assemble, CI refresh, PR check, add evidence) each take
  // seconds: their buttons show progress, carry aria-busy and refuse a second
  // click while one is in flight.
  let assembling = $state(false);
  let ciRefreshing = $state(false);
  let prRunning = $state(false);
  let evidenceSaving = $state(false);

  /** The pack's repository folder — the folder prompts default to it. There is
   *  no `GET /repos/{id}`: the path comes from the pack workspace's repo list
   *  (`GET /workspaces/{id}/repos`, ws viewer). A failed lookup is returned as
   *  `error`, so the dialog says why the folder is empty instead of hiding it. */
  async function packRepoPath(): Promise<{ path: string; error: string | null }> {
    const pack = detail?.pack;
    const rid = pack?.repo_id;
    if (!pack || !rid) return { path: '', error: null };
    try {
      const repos = await api.get<Repo[]>(`/workspaces/${encodeURIComponent(pack.workspace_id)}/repos`);
      const repo = repos.find((r) => r.id === rid);
      if (!repo) return { path: '', error: 'The pack’s repository is no longer registered in its workspace.' };
      return { path: repo.path ?? '', error: null };
    } catch (e) {
      return { path: '', error: loadErrorText(e) };
    }
  }

  async function assemble(): Promise<void> {
    if (!detail || assembling) return;
    const packId = detail.pack.id;
    const { path: initial, error: repoError } = await packRepoPath();
    const prompt = repoError
      ? `Working folder to assemble proof from. Couldn’t prefill the pack’s repository folder: ${repoError}`
      : 'Working folder to assemble proof from:';
    const cwd = await confirmer.promptText(prompt, {
      title: 'Assemble proof',
      browseFolder: true,
      confirmLabel: 'Assemble',
      placeholder: 'e.g. ~/code/my-repo',
      initial,
    });
    if (cwd === null) return;
    assembling = true;
    try {
      // The daemon answers `AssembleResp` (the pack + what it assembled).
      const res = (await assembleProof(packId, { cwd: cwd.trim() || undefined })) as AssembleResp;
      if (detail?.pack.id === packId) await proof.refreshDetail();
      const o = assembleOutcome(res.assembled);
      toasts[o.tone](o.title, o.body);
    } catch (e) {
      toasts.error("Couldn’t assemble proof", loadErrorText(e));
    } finally {
      assembling = false;
    }
  }

  // ---- waive (R10): require an approver reason (≥10 chars) -----------------
  let waiveOpen = $state(false);
  let waiveReason = $state('');

  async function submitWaive(): Promise<void> {
    if (!detail || waiveReason.trim().length < 10) return;
    try {
      await waiveProof(detail.pack.id, waiveReason.trim());
      await proof.refreshDetail();
      waiveOpen = false;
      waiveReason = '';
      toasts.success('Proof gate waived', 'Recorded with you as the approver.');
    } catch (e) {
      toasts.error("Couldn’t waive the proof gate", loadErrorText(e));
    }
  }

  // ---- media evidence (R4): screenshot / video upload ----------------------
  let mediaOpen = $state(false);
  let mKind = $state('screenshot');
  let mTitle = $state('');
  let mFile = $state<File | null>(null);

  function resetMedia(): void {
    mKind = 'screenshot';
    mTitle = '';
    mFile = null;
  }

  async function submitMedia(): Promise<void> {
    if (!detail || !mFile || !mTitle.trim()) return;
    if (mFile.size > 25 * 1024 * 1024) {
      toasts.warn('File too large', 'Media evidence must be 25 MiB or smaller.');
      return;
    }
    try {
      // Raw body: the file goes over the wire as is (no base64 / JSON).
      await attachMediaRaw(
        detail.pack.id,
        mKind === 'video' ? 'video' : 'screenshot',
        mTitle.trim(),
        mFile,
        mFile.type || (mKind === 'video' ? 'video/mp4' : 'image/png'),
      );
      await proof.refreshDetail();
      mediaOpen = false;
      resetMedia();
    } catch (e) {
      toasts.error("Couldn’t attach the media", loadErrorText(e));
    }
  }

  // ---- API / DB / Kafka evidence (R5/R6) -----------------------------------
  let evidenceOpen = $state(false);
  let eType = $state('api');
  let eTitle = $state('');
  let eMethod = $state('GET');
  let eUrl = $state('');
  let eStatus = $state('200');
  let eEngine = $state('');
  let eQuery = $state('');
  let eRowCount = $state('');
  let eTopic = $state('');
  let eMsgCount = $state('');
  let eSample = $state('');
  let eResponse = $state('');

  function resetEvidence(): void {
    eType = 'api';
    eTitle = '';
    eMethod = 'GET';
    eUrl = '';
    eStatus = '200';
    eEngine = '';
    eQuery = '';
    eRowCount = '';
    eTopic = '';
    eMsgCount = '';
    eSample = '';
    eResponse = '';
  }

  function numOrUndef(s: string): number | undefined {
    const t = s.trim();
    if (!t) return undefined;
    const n = Number(t);
    return Number.isFinite(n) ? n : undefined;
  }

  const evidenceValid = $derived(
    eTitle.trim().length > 0 &&
      (eType === 'api'
        ? eUrl.trim().length > 0
        : eType === 'kafka'
          ? eTopic.trim().length > 0
          : true),
  );

  async function submitEvidence(): Promise<void> {
    if (!detail || !evidenceValid || evidenceSaving) return;
    evidenceSaving = true;
    try {
      const id = detail.pack.id;
      if (eType === 'api') {
        await attachApiEvidence(id, {
          title: eTitle.trim(),
          method: eMethod,
          url: eUrl.trim(),
          status: numOrUndef(eStatus) ?? 0,
          response: eResponse.trim() || undefined,
        });
      } else if (eType === 'db') {
        await attachDbEvidence(id, {
          title: eTitle.trim(),
          engine: eEngine.trim() || undefined,
          query: eQuery.trim() || undefined,
          row_count: numOrUndef(eRowCount),
          sample: eSample.trim() || undefined,
        });
      } else {
        await attachKafkaEvidence(id, {
          title: eTitle.trim(),
          topic: eTopic.trim(),
          message_count: numOrUndef(eMsgCount),
          sample: eSample.trim() || undefined,
        });
      }
      await proof.refreshDetail();
      evidenceOpen = false;
      resetEvidence();
    } catch (e) {
      toasts.error("Couldn’t add the evidence", loadErrorText(e));
    } finally {
      evidenceSaving = false;
    }
  }

  // ---- CI refresh (R2) -----------------------------------------------------
  async function refreshCi(): Promise<void> {
    if (!detail || ciRefreshing) return;
    ciRefreshing = true;
    const pending = toasts.info('Refreshing CI…', 'Pulling live CI status for this pack.');
    try {
      await ciRefresh(detail.pack.id, {});
      await proof.refreshDetail();
      toasts.success('CI refreshed', 'Live CI status pulled into a CI artifact.');
    } catch (e) {
      toasts.error("Couldn’t refresh CI", loadErrorText(e));
    } finally {
      ciRefreshing = false;
      toasts.dismiss(pending);
    }
  }

  // ---- PR consistency check (R7) -------------------------------------------
  let prOpen = $state(false);
  let prTitle = $state('');
  let prDesc = $state('');
  let prBase = $state('');
  let prCwd = $state('');
  /** Why the working dir could not be prefilled from the pack's repo. */
  let prRepoError = $state<string | null>(null);

  function resetPr(): void {
    prTitle = '';
    prDesc = '';
    prBase = '';
    prCwd = '';
    prRepoError = null;
  }

  async function submitPr(): Promise<void> {
    if (!detail || !prTitle.trim() || !prDesc.trim() || prRunning) return;
    prRunning = true;
    try {
      await runPrCheck(detail.pack.id, {
        title: prTitle.trim(),
        description: prDesc.trim(),
        base: prBase.trim() || undefined,
        cwd: prCwd.trim() || undefined,
      });
      await proof.refreshDetail();
      prOpen = false;
      resetPr();
    } catch (e) {
      toasts.error("Couldn’t run the PR check", loadErrorText(e));
    } finally {
      prRunning = false;
    }
  }

  // ---- report export (R9) --------------------------------------------------
  async function exportReport(format: 'md' | 'html'): Promise<void> {
    if (!detail) return;
    try {
      const text = await proofReport(detail.pack.id, format);
      downloadText(
        text,
        `proof-${detail.pack.id}.${format}`,
        format === 'md' ? 'text/markdown' : 'text/html',
      );
    } catch (e) {
      toasts.error("Couldn’t export the report", loadErrorText(e));
    }
  }

  function openExportMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'Markdown (.md)', icon: 'download', action: () => void exportReport('md') },
      { label: 'HTML (.html)', icon: 'download', action: () => void exportReport('html') },
    ]);
  }

  // ---- per-repo proof requirements (R3) ------------------------------------
  let cfgOpen = $state(false);
  let cfgLoading = $state(false);
  let cfg = $state({
    require_test: false,
    test_cmd: '',
    require_ci: false,
    require_pr_consistency: false,
    require_review: false,
  });

  async function openConfig(): Promise<void> {
    const repoId = detail?.pack.repo_id;
    if (!repoId) return;
    cfgOpen = true;
    cfgLoading = true;
    try {
      const c = await getRepoProofConfig(repoId);
      cfg = {
        require_test: !!c.require_test,
        test_cmd: c.test_cmd ?? '',
        require_ci: !!c.require_ci,
        require_pr_consistency: !!c.require_pr_consistency,
        require_review: !!c.require_review,
      };
    } catch (e) {
      toasts.error("Couldn’t load the proof requirements", loadErrorText(e));
      cfgOpen = false;
    } finally {
      cfgLoading = false;
    }
  }

  async function saveConfig(): Promise<void> {
    const repoId = detail?.pack.repo_id;
    if (!repoId) return;
    try {
      await setRepoProofConfig(repoId, {
        require_test: cfg.require_test,
        test_cmd: cfg.test_cmd.trim() || null,
        require_ci: cfg.require_ci,
        require_pr_consistency: cfg.require_pr_consistency,
        require_review: cfg.require_review,
      });
      await proof.refreshDetail();
      cfgOpen = false;
      toasts.success('Requirements saved', 'Proof requirements updated for this repo.');
    } catch (e) {
      toasts.error("Couldn’t save the requirements", loadErrorText(e));
    }
  }

  async function removePack(): Promise<void> {
    if (!detail) return;
    const packId = detail.pack.id;
    const wsId = ws.currentId;
    const t = detail.pack.title || 'this pack';
    const n = detail.artifacts.length;
    if (!(await confirmer.ask(`Delete proof pack “${t}”? Its ${plural(n, 'artifact')} and snapshots are deleted too.`, { title: 'Delete proof pack' }))) {
      return;
    }
    try {
      await deleteProofPack(packId);
      if (detail?.pack.id !== packId || ws.currentId !== wsId) return;
      proof.closeDetail();
      toasts.success('Proof pack deleted', t);
      if (wsId) await loadList(wsId, filter);
      if (ws.currentId !== wsId || detail) return;
      // Land on the next pack instead of an empty "pick one" pane.
      if (!viewport.isPhone && proof.packs.length > 0) void open(proof.packs[0].id);
    } catch (e) {
      toasts.error("Couldn’t delete the proof pack", loadErrorText(e));
    }
  }

  // ---- opt-in archive of stale session packs (hides, never deletes) --------
  async function archiveStaleSessions(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    try {
      const dry = await archiveStaleSessionPacks(wsId, { older_than_days: 30 });
      if (dry.matched === 0) {
        toasts.info('Nothing to archive', 'No session proof packs older than 30 days are without evidence.');
        return;
      }
      const n = dry.matched;
      const ok = await confirmer.ask(
        `Archive ${plural(n, 'session proof pack')} with no evidence, untouched for 30+ days? They’re hidden from the Proof list and the sidebar chips, not deleted — a pack comes back by itself as soon as it changes or gets evidence.`,
        { title: 'Archive stale session packs', confirmLabel: 'Archive' },
      );
      if (!ok) return;
      const r = await archiveStaleSessionPacks(wsId, { older_than_days: 30, apply: true });
      toasts.success('Session packs archived', `${r.archived} hidden (nothing deleted).`);
      if (ws.currentId === wsId) await loadList(wsId, filter);
    } catch (e) {
      toasts.error("Couldn’t archive session packs", loadErrorText(e));
    }
  }

  // ---- create a manual pack ------------------------------------------------
  async function newPack(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('No workspace selected');
      return;
    }
    const title = await confirmer.promptText('Title for the new manual proof pack:', {
      title: 'New proof pack',
      confirmLabel: 'Create',
      placeholder: 'e.g. Release 1.4 verification',
    });
    if (!title || !title.trim()) return;
    try {
      const created = await createProofPack(wsId, {
        work_item_kind: 'manual',
        work_item_id: crypto.randomUUID(),
        title: title.trim(),
      });
      if (ws.currentId !== wsId) return;
      await loadList(wsId, filter);
      if (ws.currentId === wsId) await open(created.id);
    } catch (e) {
      toasts.error("Couldn’t create the proof pack", loadErrorText(e));
    }
  }

  // ⌘K: the page's verbs (pack verbs only while a pack is open).
  $effect(() => {
    const d = detail;
    return registry.register('proof', [
      { id: 'proof.new', title: 'New proof pack…', group: 'Proof', keywords: 'create manual evidence', run: () => void newPack() },
      ...(d
        ? [
            { id: 'proof.assemble', title: 'Assemble proof pack…', group: 'Proof', keywords: 'rebuild refresh evidence', run: () => void assemble() },
            { id: 'proof.add', title: 'Add artifact to proof pack…', group: 'Proof', keywords: 'evidence attach log', run: () => { resetAdd(); addOpen = true; } },
            { id: 'proof.waive', title: 'Waive proof pack…', group: 'Proof', keywords: 'approve override gate', run: () => { waiveReason = ''; waiveOpen = true; } },
            { id: 'proof.export', title: 'Export proof report (Markdown)', group: 'Proof', keywords: 'download md report', run: () => void exportReport('md') },
          ]
        : []),
    ]);
  });
;
async () => {

 { svelteHTML.createElement("div", {  "class":`proof-page`,});viewport.isPhone;
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "class":`detail-head`,"title":detail ? detail.pack.title || detail.pack.work_item_id : 'Proof packs',leading:() => { async ()/*Ωignore_positionΩ*/ => {
      if(viewport.isPhone && detail){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => proof.closeDetail(),"aria-label":`Back to list`,"title":`Back to list`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
         }
      }
    };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
      if(detail){
         { const $$_pihCsutatSfoorP0C = __sveltets_2_ensureComponent(ProofStatusChip); new $$_pihCsutatSfoorP0C({ target: __sveltets_2_any(), props: {    "status":detail.pack.status,"risk":detail.pack.risk_score,}});}
         { svelteHTML.createElement("span", { "class":`kind-tag`,});words(detail.pack.work_item_kind); }
        if(detail.pack.pr_number != null){
           { svelteHTML.createElement("span", {   "class":`kind-tag pr`,"title":`Linked pull request`,}); detail.pack.pr_number; }
        }
      }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(detail){
        
         { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-icon":`plus`}),...__sveltets_2_empty({"data-label":`Add evidence…`}),"onclick":openAddMenu,"aria-haspopup":`menu`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":assemble,"disabled":assembling,"aria-busy":assembling,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} assembling ? 'Assembling…' : 'Assemble…'; }
      }
      
      if(ws.currentId){
       { svelteHTML.createElement("button", {               "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More actions…`}),"aria-haspopup":`menu`,"aria-label":`More actions`,"title":`More actions`,"onclick":openMoreMenu,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
    
   PageHeader}

   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
   { svelteHTML.createElement("div", { "class":`proof-split`,});
  
  if(showRail){
   { svelteHTML.createElement("aside", {    "class":`rail`,"style":`--list-pane-w:${listW}px`,});viewport.isPhone && detail;
     { svelteHTML.createElement("div", { "class":`rail-head`,});
       { svelteHTML.createElement("span", { "class":`section-title`,});  }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":newPack,"aria-label":`New proof pack`,"title":`New proof pack`,});
         { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});}
       }
     }
     { svelteHTML.createElement("div", {     "class":`filters`,"role":`group`,"aria-label":`Filter by status`,});
         for(let f of __sveltets_2_ensureArray(STATUS_FILTERS)){f;
         { svelteHTML.createElement("button", {      "class":`chip-btn`,"aria-pressed":filter === f,"onclick":() => (filter = f),});filter === f;
          words(f);if(filterCounts && (filterCounts[f] ?? 0) > 0){ { svelteHTML.createElement("span", { "class":`chip-n`,});filterCounts[f]; }}
         }
      }
     }
     { svelteHTML.createElement("div", { "class":`rail-list`,});
      if(listError){
        
         { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {            "what":`proof packs`,"error":listError,"empty":proof.packs.length === 0,"variant":`compact`,"loading":proof.loading,"onretry":retryList,}});}
      }
         for(let p of __sveltets_2_ensureArray(proof.packs)){p.id;
         { svelteHTML.createElement("button", {        "class":`pack-item`,"aria-current":detail?.pack.id === p.id ? 'true' : undefined,"aria-busy":openingId === p.id,"onclick":() => open(p.id),});detail?.pack.id === p.id;
           { svelteHTML.createElement("div", { "class":`pack-top`,});
             { svelteHTML.createElement("span", {   "class":`grow ellipsis pack-title`,"title":p.title || p.work_item_id,});p.title || p.work_item_id; }
             { svelteHTML.createElement("span", {   "class":`done-pill`,"title":`Done score ${p.done_score}/100`,});p.done_score;  }
             { const $$_pihCsutatSfoorP7C = __sveltets_2_ensureComponent(ProofStatusChip); new $$_pihCsutatSfoorP7C({ target: __sveltets_2_any(), props: {    "status":p.status,"risk":p.risk_score,}});}
           }
           { svelteHTML.createElement("div", { "class":`pack-meta`,});
             { svelteHTML.createElement("span", { "class":`kind-tag`,});words(p.work_item_kind); }
             { const $$_segdaBfoorP7C = __sveltets_2_ensureComponent(ProofBadges); new $$_segdaBfoorP7C({ target: __sveltets_2_any(), props: {  "badges":p.badges,}});}
           }
         }
      }
      if(proof.nextCursor){
        
         { svelteHTML.createElement("button", {       "class":`btn small ghost load-more`,"disabled":proof.loadingMore,"aria-busy":proof.loadingMore,"onclick":() => void proof.loadMore(),});
          proof.loadingMore ? 'Loading more packs…' : 'Load more';
         }
      }
      if(proof.packs.length === 0 && !listError){
        if(proof.loading && !listLoaded){
           { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {    "what":`proof packs`,"loading":true,"empty":true,"variant":`compact`,}});}
        }else{
           { svelteHTML.createElement("div", { "class":`empty`,});
             { svelteHTML.createElement("p", { "class":`dim`,}); words(filter).toLowerCase();   }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (filter = 'all'),});  }
           }
        }
      }
     }
   }
  if(!viewport.isPhone){
     { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`proof.listW`,"label":`Resize the proof packs list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
  }
  }

  
   { svelteHTML.createElement("section", {  "class":`main`,});viewport.isPhone && showRail && !detail && !openingId && !proof.detailError;
    if(!detail){
      if(!ws.currentId){
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`folder`,"title":`Add a workspace to get started`,"body":`Proof packs belong to a workspace. Add your project folder to collect and review evidence.`,"actionLabel":`Add workspace`,"actionIcon":`plus`,"onaction":() => (ui.newWorkspaceOpen = true),}});}
      } else if (proof.detailError && !openingId){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "what":`the proof pack`,"error":proof.detailError,"empty":true,"variant":`page`,"onretry":retryOpen,}});}
      } else if (listError && !showRail){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {          "what":`proof packs`,"error":listError,"empty":true,"variant":`page`,"loading":proof.loading,"onretry":retryList,}});}
      } else if (openingId){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {    "what":`the proof pack`,"loading":true,"empty":true,"variant":`page`,}});}
      } else if (showRail){
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`check`,"title":`No proof pack open`,"body":`${packSummary}Verified evidence — tests, diffs, CI, reviews, approvals — for each piece of work. Open a pack to inspect its artifacts and badges.`,}});}
      } else if (proof.loading || !listLoaded){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {    "what":`proof packs`,"loading":true,"empty":true,"variant":`page`,}});}
      }else{
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`check`,"title":`No proof packs yet`,"body":`Verified evidence — tests, diffs, CI, reviews, approvals — assembled for each piece of work.`,"actionLabel":`New proof pack`,"actionIcon":`plus`,"onaction":newPack,}});}
      }
    }else{
       { svelteHTML.createElement("div", { "class":`detail-body`,});
        if(proof.detailError && !openingId){
          
           { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {      "what":`the proof pack`,"error":proof.detailError,"onretry":retryOpen,}});}
        }
        
         { const $$_reteMtcartnoCenoD5C = __sveltets_2_ensureComponent(DoneContractMeter); new $$_reteMtcartnoCenoD5C({ target: __sveltets_2_any(), props: {  "contract":detail.done_contract,}});}

        
         { svelteHTML.createElement("div", { "class":`tools-row`,});
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":openExportMenu,"aria-haspopup":`menu`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});}    { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
          if(detail.pack.repo_id){
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":openConfig,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":12,}});}  }
          }
         }

        if(detail.badges.length > 0){
           { svelteHTML.createElement("div", { "class":`badges-row`,}); { const $$_segdaBfoorP6C = __sveltets_2_ensureComponent(ProofBadges); new $$_segdaBfoorP6C({ target: __sveltets_2_any(), props: {  "badges":detail.badges,}});} }
        }

        if(detail.pack.summary){
           { svelteHTML.createElement("p", { "class":`summary`,});detail.pack.summary; }
        }

        if(detail.pack.waived_reason){
           { svelteHTML.createElement("p", { "class":`waived-note`,});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});} detail.pack.waived_by ? ` by ${detail.pack.waived_by}` : '';detail.pack.waived_at ? ` · ${rel(detail.pack.waived_at)}` : ''; detail.pack.waived_reason;
           }
        }

        
         { const $$_tsiLtohspanSfoorP5C = __sveltets_2_ensureComponent(ProofSnapshotList); new $$_tsiLtohspanSfoorP5C({ target: __sveltets_2_any(), props: {       "packId":detail.pack.id,"snapshots":detail.snapshots,"onchange":() => proof.refreshDetail(),}});}

        
        if(detail.artifacts.length === 0){
           { svelteHTML.createElement("p", { "class":`dim empty`,});          }
        }else{
             for(let [kind, items] of __sveltets_2_ensureArray(artifactGroups)){kind;
             { svelteHTML.createElement("section", { "class":`art-group`,});
               { svelteHTML.createElement("h3", { "class":`group-title`,});words(kind);  { svelteHTML.createElement("span", { "class":`dim`,}); items.length; } }
                 for(let a of __sveltets_2_ensureArray(items)){a.id;
                 { svelteHTML.createElement("div", { "class":`art-row`,});
                   { svelteHTML.createElement("div", { "class":`art-top`,});
                     { svelteHTML.createElement("span", {   "class":`art-status ${a.status}`,"aria-hidden":`true`,}); }
                     { svelteHTML.createElement("span", {   "class":`grow ellipsis art-title`,"title":a.title,});a.title; }
                    if(a.content_sha256){
                       { svelteHTML.createElement("span", {   "class":`sha-chip`,"title":`content sha256: ${a.content_sha256}`,}); a.content_sha256.slice(0, 8);  }
                    }
                     { svelteHTML.createElement("span", { "class":`art-status-label ${a.status}`,});words(a.status); }
                    
                    if(a.preview != null){
                       { svelteHTML.createElement("button", {     "class":`link-btn`,"aria-expanded":!!expanded[a.id],"onclick":() => toggleExpand(a.id),});
                        expanded[a.id] ? 'Hide' : 'Show';
                       }
                    } else if (fullContent[a.id] == null){
                       { svelteHTML.createElement("button", {   "class":`link-btn`,"onclick":() => loadFull(a.id),});  }
                    }else{
                       { svelteHTML.createElement("button", {     "class":`link-btn`,"aria-expanded":!!expanded[a.id],"onclick":() => toggleExpand(a.id),});
                        expanded[a.id] ? 'Hide' : 'Show';
                       }
                    }
                     { svelteHTML.createElement("button", {       "class":`icon-btn small artifact-delete`,"onclick":() => removeArtifact(a.id),"aria-label":`Delete artifact ${a.title}`,"title":`Delete artifact`,}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
                   }
                  if(MEDIA_KINDS.has(a.kind)){
                    if(mediaUrls[a.id]){
                      if(a.kind === 'video'){
                         { svelteHTML.createElement("video", {     "class":`art-media`,"controls":true,"preload":`none`,"src":mediaUrls[a.id],}); { svelteHTML.createElement("track", {  "kind":`captions`,});} }
                      }else{
                         { svelteHTML.createElement("img", {      "class":`art-media`,"src":mediaUrls[a.id],"alt":a.title,});}
                      }
                    } else if (mediaErrors[a.id]){
                       { svelteHTML.createElement("div", {});
                         { svelteHTML.createElement("p", {});    }
                         { svelteHTML.createElement("p", { "class":`dim`,});mediaErrors[a.id]; }
                         { svelteHTML.createElement("button", {     "class":`btn small`,"aria-label":`Retry media`,"onclick":() => void fetchMedia(a.id),});  }
                       }
                    } else if (a.kind === 'video'){
                       { svelteHTML.createElement("button", {   "class":`btn small media-load`,"onclick":() => void fetchMedia(a.id),});
                         { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}  
                       }
                    }else{
                       {const $$action_0 = __sveltets_2_ensureAction(whenVisible(svelteHTML.mapElementTag('div'),(() => void fetchMedia(a.id))));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`media-loading`,}); { const $$_notelekS8C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS8C({ target: __sveltets_2_any(), props: {      "rows":1,"height":120,"label":`the media`,}});} }}
                    }
                  }
                  if(a.kind === 'pr_check'){
                    const rep = prReport(a.metadata);
                    if(rep){
                       { svelteHTML.createElement("div", { "class":`pr-report`,});
                         { svelteHTML.createElement("div", { "class":`pr-report-head`,});
                            { svelteHTML.createElement("strong", {});rep.score;  } 
                           { svelteHTML.createElement("span", { "class":rep.hard_fail ? 'bad' : rep.passed ? 'ok' : 'warn',});
                            rep.hard_fail ? 'inconsistent with the change' : rep.passed ? 'consistent' : 'weak — review the description';
                           }
                         }
                         { svelteHTML.createElement("ul", { "class":`pr-checks`,});
                             for(let c of __sveltets_2_ensureArray(rep.checks)){c.label;
                             { svelteHTML.createElement("li", { "class":c.passed ? 'ok' : 'miss',});
                               { svelteHTML.createElement("span", {     "class":`tick`,"role":`img`,"aria-label":c.passed ? 'Passed' : 'Missing',}); { const $$_nocI11C = __sveltets_2_ensureComponent(Icon); new $$_nocI11C({ target: __sveltets_2_any(), props: {    "name":c.passed ? 'check' : 'x',"size":12,}});} }
                               { svelteHTML.createElement("span", { "class":`lbl`,});c.label; }
                               { svelteHTML.createElement("span", { "class":`dim`,}); c.detail; }
                             }
                          }
                         }
                       }
                    }
                  }
                  if(expanded[a.id]){
                     { svelteHTML.createElement("pre", { "class":`art-content`,});fullContent[a.id] ?? a.preview ?? ''; }
                    if(a.truncated && fullContent[a.id] == null){
                       { svelteHTML.createElement("button", {   "class":`link-btn trunc`,"onclick":() => loadFull(a.id),});    }
                    }
                  }
                 }
              }
             }
          }
        }

        
        if(detail.children.length > 0){
           { svelteHTML.createElement("section", { "class":`art-group`,});
             { svelteHTML.createElement("h3", { "class":`group-title`,});   { svelteHTML.createElement("span", { "class":`dim`,}); detail.children.length; } }
               for(let c of __sveltets_2_ensureArray(detail.children)){c.id;
               { svelteHTML.createElement("button", {   "class":`child-link`,"onclick":() => open(c.id),});
                 { svelteHTML.createElement("span", {   "class":`grow ellipsis`,"title":c.title || c.work_item_id,});c.title || c.work_item_id; }
                 { svelteHTML.createElement("span", { "class":`kind-tag`,});words(c.work_item_kind); }
                 { const $$_pihCsutatSfoorP7C = __sveltets_2_ensureComponent(ProofStatusChip); new $$_pihCsutatSfoorP7C({ target: __sveltets_2_any(), props: {    "status":c.status,"risk":c.risk_score,}});}
               }
            }
           }
        }
       }
    }
   }
   }
   PageBody}
 }

if(addOpen && detail){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Add artifact`,"width":520,"onclose":() => (addOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (addOpen = false),});  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":submitAdd,"disabled":!aTitle.trim(),"title":aTitle.trim() ? undefined : 'Give the artifact a title',});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`a-kind`,});  }
       { svelteHTML.createElement("select", {     "id":`a-kind`,"class":`input`,"bind:value":aKind,});/*Ωignore_startΩ*/() => aKind = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let k of __sveltets_2_ensureArray(ARTIFACT_KINDS)){k; { svelteHTML.createElement("option", { "value":k,});words(k); }}
       }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`a-title`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`a-title`,"class":`input`,"bind:value":aTitle,"placeholder":`e.g. cargo test output`,});/*Ωignore_startΩ*/() => aTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`a-status`,});  }
       { svelteHTML.createElement("select", {     "id":`a-status`,"class":`input`,"bind:value":aStatus,});/*Ωignore_startΩ*/() => aStatus = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let s of __sveltets_2_ensureArray(ARTIFACT_STATUSES)){s; { svelteHTML.createElement("option", { "value":s,});words(s); }}
       }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`a-content`,});  }
       { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`a-content`,"class":`input`,"rows":6,"bind:value":aContent,"placeholder":`e.g. test result: 214 passed; 0 failed`,});/*Ωignore_startΩ*/() => aContent = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }
    
   Modal}
}

if(waiveOpen && detail){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Waive proof gate`,"width":480,"onclose":() => (waiveOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (waiveOpen = false),});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":submitWaive,"disabled":waiveReason.trim().length < 10,});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("p", { "class":`modal-hint`,});
                    
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`w-reason`,});  }
       { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`w-reason`,"class":`input`,"rows":4,"bind:value":waiveReason,"placeholder":`e.g. verified manually in staging; CI flaky on unrelated job`,});/*Ωignore_startΩ*/() => waiveReason = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("span", {  "class":`char-hint`,});waiveReason.trim().length < 10;
        waiveReason.trim().length;  
       }
     }
    
   Modal}
}

if(mediaOpen && detail){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Add media evidence`,"width":480,"onclose":() => (mediaOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (mediaOpen = false),});  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":submitMedia,"disabled":!mFile || !mTitle.trim(),"title":!mFile ? 'Choose a file' : !mTitle.trim() ? 'Give it a title' : undefined,});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`m-kind`,});  }
       { svelteHTML.createElement("select", {     "id":`m-kind`,"class":`input`,"bind:value":mKind,});/*Ωignore_startΩ*/() => mKind = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`screenshot`,});  }
         { svelteHTML.createElement("option", { "value":`video`,});  }
       }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`m-title`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`m-title`,"class":`input`,"bind:value":mTitle,"placeholder":`e.g. Dashboard after fix`,});/*Ωignore_startΩ*/() => mTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`m-file`,});  { svelteHTML.createElement("span", { "class":`dim`,});   } }
       { svelteHTML.createElement("input", {           "id":`m-file`,"class":`input`,"type":`file`,"accept":mKind === 'video' ? 'video/*' : 'image/*',"onchange":(e) => (mFile = e.currentTarget.files?.[0] ?? null),});}
     }
    
   Modal}
}

if(evidenceOpen && detail){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Add evidence`,"width":520,"onclose":() => (evidenceOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (evidenceOpen = false),});  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":submitEvidence,"disabled":!evidenceValid || evidenceSaving,"aria-busy":evidenceSaving,});evidenceSaving ? 'Adding…' : 'Add evidence'; }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`e-type`,});  }
       { svelteHTML.createElement("select", {     "id":`e-type`,"class":`input`,"bind:value":eType,});/*Ωignore_startΩ*/() => eType = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`api`,});  }
         { svelteHTML.createElement("option", { "value":`db`,});  }
         { svelteHTML.createElement("option", { "value":`kafka`,});  }
       }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`e-title`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`e-title`,"class":`input`,"bind:value":eTitle,"placeholder":`e.g. GET /health → 200`,});/*Ωignore_startΩ*/() => eTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    if(eType === 'api'){
       { svelteHTML.createElement("div", { "class":`field-row`,});
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`e-method`,});  }
           { svelteHTML.createElement("select", {     "id":`e-method`,"class":`input`,"bind:value":eMethod,});/*Ωignore_startΩ*/() => eMethod = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let m of __sveltets_2_ensureArray((['GET', 'POST', 'PUT', 'PATCH', 'DELETE']))){m; { svelteHTML.createElement("option", { "value":m,});m; }}
           }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`e-status`,});  }
           { svelteHTML.createElement("input", {            "dir":`auto`,"id":`e-status`,"class":`input`,"inputmode":`numeric`,"bind:value":eStatus,"placeholder":`e.g. 200`,});/*Ωignore_startΩ*/() => eStatus = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`e-url`,});  }
         { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`e-url`,"class":`input`,"bind:value":eUrl,"placeholder":`e.g. https://api.example.com/health`,});/*Ωignore_startΩ*/() => eUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`e-response`,});  }
         { svelteHTML.createElement("textarea", {           "dir":`ltr`,"id":`e-response`,"class":`input`,"rows":4,"bind:value":eResponse,"placeholder":'e.g. {"status": "ok"}',});/*Ωignore_startΩ*/() => eResponse = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
    } else if (eType === 'db'){
       { svelteHTML.createElement("div", { "class":`field-row`,});
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`e-engine`,});  }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`e-engine`,"class":`input`,"bind:value":eEngine,"placeholder":`e.g. mysql`,});/*Ωignore_startΩ*/() => eEngine = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`e-rows`,});   }
           { svelteHTML.createElement("input", {            "dir":`auto`,"id":`e-rows`,"class":`input`,"inputmode":`numeric`,"bind:value":eRowCount,"placeholder":`e.g. 42`,});/*Ωignore_startΩ*/() => eRowCount = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`e-query`,});  }
         { svelteHTML.createElement("textarea", {           "dir":`ltr`,"id":`e-query`,"class":`input`,"rows":3,"bind:value":eQuery,"placeholder":`e.g. SELECT count(*) FROM orders`,});/*Ωignore_startΩ*/() => eQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`e-sample`,});  }
         { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`e-sample`,"class":`input`,"rows":4,"bind:value":eSample,"placeholder":`e.g. count(*) = 42`,});/*Ωignore_startΩ*/() => eSample = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`field-row`,});
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`e-topic`,});  }
           { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`e-topic`,"class":`input`,"bind:value":eTopic,"placeholder":`e.g. orders.events`,});/*Ωignore_startΩ*/() => eTopic = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`e-msgs`,});   }
           { svelteHTML.createElement("input", {            "dir":`auto`,"id":`e-msgs`,"class":`input`,"inputmode":`numeric`,"bind:value":eMsgCount,"placeholder":`e.g. 100`,});/*Ωignore_startΩ*/() => eMsgCount = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`e-ksample`,});  }
         { svelteHTML.createElement("textarea", {           "dir":`ltr`,"id":`e-ksample`,"class":`input`,"rows":4,"bind:value":eSample,"placeholder":'e.g. {"order_id": 42, "status": "paid"}',});/*Ωignore_startΩ*/() => eSample = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
    }
    
   Modal}
}

if(prOpen && detail){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`PR consistency check`,"width":520,"onclose":() => (prOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (prOpen = false),});  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":submitPr,"disabled":!prTitle.trim() || !prDesc.trim() || prRunning,"aria-busy":prRunning,});prRunning ? 'Running check…' : 'Run check'; }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("p", { "class":`modal-hint`,});
                 
               
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`pr-title`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`pr-title`,"class":`input`,"bind:value":prTitle,"placeholder":`e.g. PR #123 description check`,});/*Ωignore_startΩ*/() => prTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`pr-desc`,});  }
       { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`pr-desc`,"class":`input`,"rows":6,"bind:value":prDesc,"placeholder":`e.g. Fixes the login redirect and adds tests for expired sessions`,});/*Ωignore_startΩ*/() => prDesc = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }
     { svelteHTML.createElement("div", { "class":`field-row`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pr-base`,});  }
         { svelteHTML.createElement("input", {          "dir":`auto`,"id":`pr-base`,"class":`input`,"bind:value":prBase,"placeholder":`e.g. main`,});/*Ωignore_startΩ*/() => prBase = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pr-cwd`,});   }
         { const $$_dleiFhtaP3C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP3 = new $$_dleiFhtaP3C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:prCwd,}});/*Ωignore_startΩ*/() => prCwd = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP3.$$bindings = 'value'; { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`pr-cwd`,"class":`input`,"bind:value":prCwd,"placeholder":`e.g. ~/code/my-repo`,"aria-describedby":prRepoError ? 'pr-cwd-note' : undefined,});/*Ωignore_startΩ*/() => prCwd = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
        if(prRepoError){ { svelteHTML.createElement("p", {   "id":`pr-cwd-note`,"class":`pr-cwd-note`,});      prRepoError; }}
       }
     }
    
   Modal}
}

if(cfgOpen && detail){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Proof requirements`,"width":480,"onclose":() => (cfgOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (cfgOpen = false),});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":saveConfig,"disabled":cfgLoading,});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    if(cfgLoading){
        { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {  "what":`the proof requirements`,"loading":true,"empty":true,}});}
    }else{
       { svelteHTML.createElement("p", { "class":`modal-hint`,});          }
       { svelteHTML.createElement("label", { "class":`check-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":cfg.require_test,});/*Ωignore_startΩ*/() => cfg.require_test = __sveltets_2_any(null);/*Ωignore_endΩ*/}   
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cfg-cmd`,});   }
         { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`cfg-cmd`,"class":`input`,"bind:value":cfg.test_cmd,"placeholder":`e.g. cargo test --workspace`,});/*Ωignore_startΩ*/() => cfg.test_cmd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("label", { "class":`check-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":cfg.require_ci,});/*Ωignore_startΩ*/() => cfg.require_ci = __sveltets_2_any(null);/*Ωignore_endΩ*/}   
       }
       { svelteHTML.createElement("label", { "class":`check-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":cfg.require_pr_consistency,});/*Ωignore_startΩ*/() => cfg.require_pr_consistency = __sveltets_2_any(null);/*Ωignore_endΩ*/}   
       }
       { svelteHTML.createElement("label", { "class":`check-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":cfg.require_review,});/*Ωignore_startΩ*/() => cfg.require_review = __sveltets_2_any(null);/*Ωignore_endΩ*/}   
       }
    }
    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProofPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProofPage__SvelteComponent_ = ReturnType<typeof ProofPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProofPage__SvelteComponent_;