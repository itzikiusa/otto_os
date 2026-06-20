<script lang="ts">
  // AI review panel: start review agents, poll until done, approve/decline
  // individual draft comments. Supports live per-agent progress cards and
  // a configure-agents modal.
  import { api, ApiError } from '../../lib/api/client';
  import type {
    Review,
    ReviewComment,
    ReviewConfig,
    ReviewAgentCfg,
    StartReviewReq,
    DiffResp,
    FileDiff,
    DiffLine,
  } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import JiraIssuePicker from '../agents/JiraIssuePicker.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import ReviewAgents from './ReviewAgents.svelte';

  // --- Installed-skill metadata (library API; category + version are new) ----
  // Defined locally so this panel can read the extended fields without touching
  // the shared LibrarySkill type.
  interface LibrarySkillMeta {
    name: string;
    description: string;
    category?: string | null;
    version?: number | null;
    body?: string;
  }
  type BundledState = 'not_installed' | 'up_to_date' | 'update_available' | 'ahead';
  interface BundledSkill {
    name: string;
    category?: string | null;
    version?: number | null;
    description?: string;
    installed_version?: number | null;
    state: BundledState;
  }

  interface Props {
    repoId: string;
    prNumber: number;
  }
  let { repoId, prNumber }: Props = $props();

  let review: Review | null = $state(null);
  // History: all past runs (newest first); review is always history[0] when set
  let history: Review[] = $state([]);
  let loading = $state(true);
  let starting = $state(false);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollCount = $state(0);
  let pollPaused = $state(false);

  function pollDelay(count: number): number {
    if (count < 5) return 2000;
    if (count < 15) return 5000;
    return 10000;
  }
  // Which older run indices are expanded (history[1+])
  let historyExpanded: Record<number, boolean> = $state({});
  // A child <ReviewAgents> retried one agent: adopt the refreshed review and
  // resume polling so we keep tracking the re-run.
  function onAgentRetried(r: Review): void {
    review = r;
    history = history.length > 0 ? [r, ...history.slice(1)] : [r];
    pollCount = 0;
    schedulePoll();
  }

  // Per-comment action busy state keyed by comment id
  let actionBusy: Record<string, 'approve' | 'decline'> = $state({});

  // Jira story attachment state
  let showJiraPicker = $state(false);
  // The currently attached issue (persists across re-runs within this component)
  let attachedIssue: { account_id: string; key: string; summary: string } | null = $state(null);

  // Free-text guidance for the review agents (e.g. "what to focus on"). Optional;
  // persists across re-runs within this component.
  let reviewContext = $state('');

  // Config modal state
  let showConfig = $state(false);
  let configLoading = $state(false);
  let configSaving = $state(false);
  let editAgents: ReviewAgentCfg[] = $state([]);
  let editSummarizer: ReviewAgentCfg = $state({
    name: 'Summarizer',
    provider: 'claude',
    providers: [],
    model: '',
    prompt: '',
  });
  // Custom presets (persisted in ReviewConfig.custom_presets)
  let editPresets: ReviewAgentCfg[] = $state([]);

  /** Ensure an agent always has a `providers` array (migration from old format). */
  function normalizeAgent(a: ReviewAgentCfg): ReviewAgentCfg {
    if (!a.providers || a.providers.length === 0) {
      return { ...a, providers: [a.provider || 'claude'] };
    }
    return a;
  }

  /** Toggle a provider in/out of the agent's providers list. Keeps at least one. */
  function toggleProvider(agentIdx: number, p: string): void {
    const current = editAgents[agentIdx].providers ?? ['claude'];
    const has = current.includes(p);
    let next: string[];
    if (has) {
      next = current.filter((x) => x !== p);
      if (next.length === 0) next = ['claude']; // never allow empty
    } else {
      next = [...current, p];
    }
    editAgents = editAgents.map((a, i) =>
      i === agentIdx ? { ...a, providers: next } : a
    );
  }

  // Diff state for per-comment snippets
  let diffData: DiffResp | null = $state(null);
  let diffLoading = $state(false);
  // Per-comment collapse state: true = expanded (default for draft)
  let diffExpanded: Record<string, boolean> = $state({});

  const PROVIDER_OPTIONS = ['claude', 'codex', 'agy'];
  const MAX_POLLS = 600; // ~20 min at 2s — covers long multi-agent live reviews

  // --- Data-driven review-skill presets ------------------------------------
  // One-click lenses derived from the installed `review`-category skills. Each
  // becomes a review agent that runs as a session with the skill materialized,
  // so "Apply the `<skill>` skill" makes it follow the skill's full method.
  let skillPresets: { name: string; focus: string }[] = $state([]);
  // Pre-check: review skills that are missing or have a newer bundled version.
  let missingReviewSkills = $state(0);
  let outdatedReviewSkills = $state(0);
  let precheckDismissed = $state(false);

  const showPrecheck = $derived(
    !precheckDismissed && (missingReviewSkills > 0 || outdatedReviewSkills > 0),
  );

  /** Prettify a skill name for display: "security-review" -> "Security review". */
  function prettifySkillName(name: string): string {
    const spaced = name.replace(/[-_]+/g, ' ').trim();
    if (spaced === '') return name;
    return spaced.charAt(0).toUpperCase() + spaced.slice(1);
  }

  /** Build a preset whose lens applies the named skill (materialized in-session). */
  function presetFromSkill(s: LibrarySkillMeta): { name: string; focus: string } {
    const desc = (s.description ?? '').trim();
    return {
      name: prettifySkillName(s.name),
      focus: `Apply the \`${s.name}\` skill (it is available to you): ${desc} ${JSON_INSTR}`,
    };
  }

  $effect(() => {
    void load(repoId, prNumber);
    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => {
      if (pollTimer !== null) clearTimeout(pollTimer);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  });

  // Load installed review skills (primary one-click lenses) + the missing/
  // outdated pre-check. Non-blocking: failures just leave the fallback presets.
  $effect(() => {
    void loadReviewSkills();
  });

  async function loadReviewSkills(): Promise<void> {
    try {
      const skills = await api.get<LibrarySkillMeta[]>('/library/skills');
      skillPresets = skills
        .filter((s) => s.category === 'review')
        .map((s) => presetFromSkill(s));
    } catch {
      // Library unreachable: keep skillPresets empty -> hardcoded fallback shows.
    }
    try {
      const bundled = await api.get<BundledSkill[]>('/library/bundled');
      const review = bundled.filter((b) => b.category === 'review');
      missingReviewSkills = review.filter((b) => b.state === 'not_installed').length;
      outdatedReviewSkills = review.filter((b) => b.state === 'update_available').length;
    } catch {
      // Pre-check is best-effort; absence simply hides the banner.
    }
  }

  // Open Settings → Skills (the bundled-skill install/update panel).
  function openSkillSettings(): void {
    router.go('settings/skills');
  }

  async function load(rid: string, num: number): Promise<void> {
    loading = true;
    diffData = null;
    try {
      const runs = await api.get<Review[]>(`/repos/${rid}/prs/${num}/reviews`);
      history = runs;
      review = runs.length > 0 ? runs[0] : null;
      if (review?.status === 'running') schedulePoll();
      if (review?.status === 'done' && review.comments.length > 0) {
        void loadDiff(rid, num);
      }
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        review = null;
        history = [];
      } else {
        toasts.error('Could not load review', e instanceof Error ? e.message : String(e));
      }
    } finally {
      loading = false;
    }
  }

  async function loadDiff(rid: string, num: number): Promise<void> {
    if (diffData !== null || diffLoading) return;
    diffLoading = true;
    try {
      diffData = await api.get<DiffResp>(`/repos/${rid}/prs/${num}/diff`);
    } catch {
      // non-blocking: diff preview simply won't show
    } finally {
      diffLoading = false;
    }
  }

  function schedulePoll(delay?: number): void {
    if (pollTimer !== null) clearTimeout(pollTimer);
    if (pollPaused) return;
    pollTimer = setTimeout(() => void poll(), delay ?? pollDelay(pollCount));
  }

  function handleVisibilityChange(): void {
    if (document.hidden) {
      pollPaused = true;
      if (pollTimer !== null) { clearTimeout(pollTimer); pollTimer = null; }
    } else {
      pollPaused = false;
      if (review?.status === 'running') schedulePoll(0);
    }
  }

  async function poll(): Promise<void> {
    pollCount++;
    try {
      const r = await api.get<Review>(`/repos/${repoId}/prs/${prNumber}/review`);
      // Update the latest run in-place within history
      review = r;
      if (history.length > 0) {
        history = [r, ...history.slice(1)];
      } else {
        history = [r];
      }
      // Keep polling while the review runs OR any agent is still in progress
      // (covers a single agent retried after the overall review finished).
      const anyActive = r.agents.some(
        (a) => a.status === 'pending' || a.status === 'running' || a.status === 'waiting',
      );
      if (r.status === 'running' || anyActive) {
        schedulePoll();
      } else {
        pollCount = 0;
        if (r.status === 'done' && r.comments.length > 0) {
          void loadDiff(repoId, prNumber);
        }
      }
    } catch {
      // silently retry
      schedulePoll();
    }
  }

  async function startReview(): Promise<void> {
    if (pollTimer !== null) clearTimeout(pollTimer);
    starting = true;
    pollCount = 0;
    diffData = null;
    try {
      const body: StartReviewReq = {};
      if (attachedIssue) {
        body.issue_account_id = attachedIssue.account_id;
        body.issue_key = attachedIssue.key;
      }
      const trimmedContext = reviewContext.trim();
      if (trimmedContext) body.context = trimmedContext;
      const newRun = await api.post<Review>(`/repos/${repoId}/prs/${prNumber}/review`, body);
      // Prepend new run; keep old runs in history
      review = newRun;
      history = [newRun, ...history];
      if (review.status === 'running') schedulePoll();
    } catch (e) {
      toasts.error('Could not start review', e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
    }
  }

  // --- Jira picker ---

  function openJiraPicker(): void {
    showJiraPicker = true;
  }

  function closeJiraPicker(): void {
    showJiraPicker = false;
  }

  function removeAttachedIssue(): void {
    attachedIssue = null;
  }

  function patchCommentInReview(r: Review, updated: ReviewComment): Review {
    return { ...r, comments: r.comments.map((x) => (x.id === updated.id ? updated : x)) };
  }

  async function approveComment(c: ReviewComment): Promise<void> {
    actionBusy = { ...actionBusy, [c.id]: 'approve' };
    try {
      const updated = await api.post<ReviewComment>(`/pr-review-comments/${c.id}/approve`);
      if (review) {
        review = patchCommentInReview(review, updated);
        if (history.length > 0) history = [review, ...history.slice(1)];
      }
      toasts.success('Comment posted');
    } catch (e) {
      toasts.error('Could not approve comment', e instanceof Error ? e.message : String(e));
    } finally {
      const next = { ...actionBusy };
      delete next[c.id];
      actionBusy = next;
    }
  }

  async function declineComment(c: ReviewComment): Promise<void> {
    actionBusy = { ...actionBusy, [c.id]: 'decline' };
    try {
      const updated = await api.post<ReviewComment>(`/pr-review-comments/${c.id}/decline`);
      if (review) {
        review = patchCommentInReview(review, updated);
        if (history.length > 0) history = [review, ...history.slice(1)];
      }
      toasts.info('Comment declined');
    } catch (e) {
      toasts.error('Could not decline comment', e instanceof Error ? e.message : String(e));
    } finally {
      const next = { ...actionBusy };
      delete next[c.id];
      actionBusy = next;
    }
  }

  // --- Config modal ---

  async function openConfig(): Promise<void> {
    showConfig = true;
    configLoading = true;
    try {
      const cfg = await api.get<ReviewConfig>('/settings/pr-review');
      // Normalize: ensure every reviewer agent has a populated `providers` array.
      editAgents = cfg.agents.map((a) => normalizeAgent({ ...a }));
      editSummarizer = { ...cfg.summarizer };
      editPresets = (cfg.custom_presets ?? []).map((p) => normalizeAgent({ ...p }));
    } catch (e) {
      toasts.error('Could not load config', e instanceof Error ? e.message : String(e));
      showConfig = false;
    } finally {
      configLoading = false;
    }
  }

  function closeConfig(): void {
    showConfig = false;
  }

  function addAgent(): void {
    editAgents = [
      ...editAgents,
      { name: 'New agent', provider: 'claude', providers: ['claude'], model: '', prompt: '' },
    ];
  }

  function removeAgent(i: number): void {
    editAgents = editAgents.filter((_, idx) => idx !== i);
  }

  /** Build the ReviewConfig body from current edit state (same shape as saveConfig). */
  function buildConfigFromEditState(presets: ReviewAgentCfg[]): ReviewConfig {
    const syncedAgents = editAgents.map((a) => {
      const ps = a.providers && a.providers.length > 0 ? a.providers : [a.provider || 'claude'];
      return { ...a, provider: ps[0], providers: ps };
    });
    return {
      agents: syncedAgents,
      summarizer: editSummarizer,
      custom_presets: presets.map((a) => {
        const ps = a.providers && a.providers.length > 0 ? a.providers : [a.provider || 'claude'];
        return { ...a, provider: ps[0], providers: ps };
      }),
    };
  }

  /** Persist presets immediately without closing the modal. */
  async function persistPresets(presets: ReviewAgentCfg[]): Promise<void> {
    try {
      await api.put('/settings/pr-review', buildConfigFromEditState(presets));
    } catch (e) {
      toasts.error('Could not persist presets', e instanceof Error ? e.message : String(e));
    }
  }

  // Save the agent at index i as a custom preset (dedupe by name) and persist immediately
  async function saveAsPreset(i: number): Promise<void> {
    const agent = editAgents[i];
    const effectiveProviders = (agent.providers && agent.providers.length > 0)
      ? agent.providers
      : [agent.provider || 'claude'];
    const copy: ReviewAgentCfg = {
      name: agent.name,
      provider: effectiveProviders[0],
      providers: effectiveProviders,
      model: agent.model,
      prompt: agent.prompt,
    };
    const existing = editPresets.findIndex((p) => p.name === copy.name);
    let next: ReviewAgentCfg[];
    if (existing >= 0) {
      next = editPresets.map((p, idx) => (idx === existing ? copy : p));
    } else {
      next = [...editPresets, copy];
    }
    editPresets = next;
    await persistPresets(next);
    toasts.success('Saved to presets');
  }

  // Add a custom preset to editAgents
  function addPresetToAgents(preset: ReviewAgentCfg): void {
    editAgents = [...editAgents, { ...preset }];
  }

  // Remove a custom preset by index and persist immediately
  async function removePreset(i: number): Promise<void> {
    const next = editPresets.filter((_, idx) => idx !== i);
    editPresets = next;
    await persistPresets(next);
  }

  // Preset reviewers — curated lenses the user can one-click add alongside
  // their own. Each produces a JSON-array of {path,line,severity,body}.
  const JSON_INSTR =
    'Output ONLY a JSON array (no prose, no markdown fence) of objects {"path":string,"line":number,"severity":"info"|"warn"|"bug","body":string} for issues you find in the diff. Empty array if none.';
  const REVIEWER_PRESETS: { name: string; focus: string }[] = [
    { name: 'Security & vulnerabilities', focus: 'Focus on security: injection, broken authn/authz, secret/credential handling, unsafe deserialization, SSRF, path traversal, and unvalidated input.' },
    { name: 'Performance & efficiency', focus: 'Focus on performance: N+1 queries, unnecessary allocations, O(n^2) hot paths, blocking I/O, missing caching/indexes, and oversized payloads.' },
    { name: 'Correctness & bugs', focus: 'Focus on correctness: logic errors, off-by-one, null/undefined handling, wrong edge cases, and broken invariants.' },
    { name: 'Tests & coverage', focus: 'Focus on testing: missing tests for new/changed behavior, untested edge cases, brittle or flaky tests, and missing assertions.' },
    { name: 'Error handling & resilience', focus: 'Focus on error handling: swallowed errors, missing propagation, unhandled failures, retries/timeouts, and resource cleanup.' },
    { name: 'Concurrency & races', focus: 'Focus on concurrency: data races, deadlocks, unsynchronized shared state, and incorrect async/await or locking.' },
    { name: 'API & interface design', focus: 'Focus on API design: breaking changes, inconsistent naming, unclear contracts, leaky abstractions, and backward compatibility.' },
    { name: 'Readability & style', focus: 'Focus on readability: unclear naming, dead code, overly complex functions, and inconsistent style.' },
    { name: 'Documentation', focus: 'Focus on documentation: missing or outdated doc comments, unclear public APIs, and undocumented behavior changes.' },
    { name: 'Dependencies & licensing', focus: 'Focus on dependencies: risky new deps, loose version pinning, known-vulnerable packages, and license concerns.' },
  ];

  function addPreset(p: { name: string; focus: string }): void {
    editAgents = [
      ...editAgents,
      { name: p.name, provider: 'claude', providers: ['claude'], model: '', prompt: `${p.focus} ${JSON_INSTR}` },
    ];
  }

  function openPresetMenu(e: MouseEvent): void {
    const taken = new Set(editAgents.map((a) => a.name));
    // Primary lenses: installed `review`-category skills. Fall back to the
    // hardcoded presets only when no review skills are installed, so the menu
    // is never empty.
    const lensPresets = skillPresets.length > 0 ? skillPresets : REVIEWER_PRESETS;
    const builtinItems = lensPresets.map((p) => ({
      label: p.name,
      disabled: taken.has(p.name),
      action: () => addPreset(p),
    }));
    const customItems = editPresets.map((p) => ({
      label: p.name,
      disabled: taken.has(p.name),
      action: () => addPresetToAgents(p),
    }));
    const items =
      customItems.length > 0
        ? [...builtinItems, { label: '─── Your presets ───', disabled: true, action: () => {} }, ...customItems]
        : builtinItems;
    ctxMenu.show(e, items);
  }

  async function saveConfig(): Promise<void> {
    configSaving = true;
    try {
      // Keep `provider` in sync with `providers[0]` for backward compatibility.
      const syncedAgents = editAgents.map((a) => {
        const ps = a.providers && a.providers.length > 0 ? a.providers : [a.provider || 'claude'];
        return { ...a, provider: ps[0], providers: ps };
      });
      const cfg: ReviewConfig = {
        agents: syncedAgents,
        summarizer: editSummarizer,
        custom_presets: editPresets.map((a) => {
          const ps = a.providers && a.providers.length > 0 ? a.providers : [a.provider || 'claude'];
          return { ...a, provider: ps[0], providers: ps };
        }),
      };
      await api.put('/settings/pr-review', cfg);
      toasts.success('Review config saved');
      showConfig = false;
    } catch (e) {
      toasts.error('Could not save config', e instanceof Error ? e.message : String(e));
    } finally {
      configSaving = false;
    }
  }

  // --- Diff snippet helpers ---

  const CONTEXT_LINES = 3;

  function getSnippetLines(c: ReviewComment): DiffLine[] | null {
    if (!diffData || c.path === null || c.line === null) return null;
    const fileDiff: FileDiff | undefined = diffData.files.find((f) => f.path === c.path);
    if (!fileDiff) return null;

    // Collect all lines from all hunks into a flat array
    const allLines: DiffLine[] = [];
    for (const hunk of fileDiff.hunks) {
      for (const line of hunk.lines) {
        allLines.push(line);
      }
    }

    // Find the target line index — match new_line first, fallback old_line
    let targetIdx = allLines.findIndex((l) => l.new_line === c.line);
    if (targetIdx < 0) targetIdx = allLines.findIndex((l) => l.old_line === c.line);
    if (targetIdx < 0) return null;

    const start = Math.max(0, targetIdx - CONTEXT_LINES);
    const end = Math.min(allLines.length - 1, targetIdx + CONTEXT_LINES);
    return allLines.slice(start, end + 1);
  }

  function isDiffExpandedDefault(c: ReviewComment): boolean {
    return c.state === 'draft';
  }

  function toggleDiff(id: string, defaultExpanded: boolean): void {
    const current = id in diffExpanded ? diffExpanded[id] : defaultExpanded;
    diffExpanded = { ...diffExpanded, [id]: !current };
  }

  function isDiffExpanded(id: string, defaultExpanded: boolean): boolean {
    return id in diffExpanded ? diffExpanded[id] : defaultExpanded;
  }

  const approvedCount = $derived.by(() => {
    const cs = review?.comments ?? [];
    return cs.filter((c: ReviewComment) => c.state === 'approved').length;
  });
  const draftCount = $derived.by(() => {
    const cs = review?.comments ?? [];
    return cs.filter((c: ReviewComment) => c.state === 'draft').length;
  });
  const totalCount = $derived.by(() => review?.comments?.length ?? 0);
  const blockerCount = $derived.by(() => review?.blocker_count ?? 0);
  const mergeReady = $derived.by(() => {
    const r = review;
    return r !== null && r.status === 'done' && blockerCount === 0;
  });

  /** Format an ISO timestamp as "X ago" */
  function timeAgo(iso: string): string {
    const diff = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
    if (diff < 60) return `${diff}s ago`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  }

  function toggleHistoryRun(idx: number): void {
    historyExpanded = { ...historyExpanded, [idx]: !historyExpanded[idx] };
  }
</script>

<div class="rp">
  <!-- Non-blocking pre-check: review skills missing or with available updates.
       The user can still run a review with whatever is installed. -->
  {#if showPrecheck}
    <div class="rp-precheck" role="status">
      <span class="rp-precheck-icon">&#9888;</span>
      <span class="rp-precheck-msg">
        {#if missingReviewSkills > 0}{missingReviewSkills} review skill{missingReviewSkills === 1 ? " isn't" : "s aren't"} installed{/if}{#if missingReviewSkills > 0 && outdatedReviewSkills > 0} · {/if}{#if outdatedReviewSkills > 0}{outdatedReviewSkills} {outdatedReviewSkills === 1 ? 'has an update' : 'have updates'}{/if}
      </span>
      <button class="btn small ghost rp-precheck-btn" onclick={openSkillSettings}>
        Settings → Skills
      </button>
      <button
        class="rp-precheck-dismiss"
        onclick={() => (precheckDismissed = true)}
        aria-label="Dismiss"
      >&#10005;</button>
    </div>
  {/if}
  {#if loading}
    <div style="padding: 16px"><Skeleton rows={4} height={36} /></div>
  {:else if !review}
    <div class="rp-no-review">
      <EmptyState
        icon="zap"
        title="No review yet"
        body="Run AI agents to analyze the diff and generate draft comments."
        actionLabel={starting ? 'Starting…' : 'Send to review agents'}
        onaction={startReview}
      />
      <div class="rp-jira-row">
        {#if attachedIssue}
          <span class="chip rp-jira-chip">
            JIRA: {attachedIssue.key} — {attachedIssue.summary}
            <button class="rp-jira-remove" onclick={removeAttachedIssue} aria-label="Remove Jira story">&#10005;</button>
          </span>
        {:else}
          <button class="btn small ghost" onclick={openJiraPicker}>+ Attach Jira story</button>
        {/if}
      </div>
      <div class="rp-context-row">
        <textarea
          class="rp-context-input"
          rows={2}
          placeholder="What should the reviewers focus on? (optional)"
          bind:value={reviewContext}
        ></textarea>
      </div>
      <button class="btn small ghost rp-cfg-btn" onclick={openConfig}>
        &#9881; Configure agents
      </button>
    </div>
  {:else if review.status === 'running'}
    <div class="rp-running-header">
      <div class="spinner"></div>
      <span class="rp-running-title">Reviewing…</span>
      <span class="grow"></span>
      <button class="btn small ghost" onclick={openConfig}>&#9881; Configure</button>
    </div>
    <!-- Live agent cards (shared with the local review) -->
    {#if review.agents && review.agents.length > 0}
      <ReviewAgents {review} view="running" onretried={onAgentRetried} />
    {:else}
      <p class="dim" style="font-size:12px;padding:8px 0">Agents starting…</p>
    {/if}
  {:else if review.status === 'error'}
    <div class="rp-error card">
      <Icon name="zap" size={14} />
      <span class="rp-error-msg">{review.error ?? 'An unknown error occurred.'}</span>
      <button class="btn small" disabled={starting} onclick={startReview}>
        {starting ? 'Starting…' : 'Try again'}
      </button>
    </div>
    <div class="rp-jira-row">
      {#if attachedIssue}
        <span class="chip rp-jira-chip">
          JIRA: {attachedIssue.key} — {attachedIssue.summary}
          <button class="rp-jira-remove" onclick={removeAttachedIssue} aria-label="Remove Jira story">&#10005;</button>
        </span>
      {:else}
        <button class="btn small ghost" onclick={openJiraPicker}>+ Attach Jira story</button>
      {/if}
    </div>
  {:else}
    <!-- status === 'done' -->
    <div class="rp-header">
      <span class="rp-stats">
        <span class="rp-stat">{totalCount} comment{totalCount === 1 ? '' : 's'}</span>
        {#if approvedCount > 0}
          <span class="chip ok">{approvedCount} approved</span>
        {/if}
        {#if draftCount > 0}
          <span class="chip">{draftCount} draft</span>
        {/if}
      </span>
      <button class="btn small ghost" onclick={openConfig}>&#9881; Configure</button>
      <button class="btn small ghost" disabled={starting} onclick={startReview}>
        <Icon name="refresh" size={11} /> {starting ? 'Starting…' : 'Re-run review'}
      </button>
    </div>
    <div class="rp-jira-row">
      {#if attachedIssue}
        <span class="chip rp-jira-chip">
          JIRA: {attachedIssue.key} — {attachedIssue.summary}
          <button class="rp-jira-remove" onclick={removeAttachedIssue} aria-label="Remove Jira story">&#10005;</button>
        </span>
      {:else}
        <button class="btn small ghost" onclick={openJiraPicker}>+ Attach Jira story</button>
      {/if}
    </div>
    <div class="rp-context-row">
      <textarea
        class="rp-context-input"
        rows={2}
        placeholder="What should the reviewers focus on? (optional)"
        bind:value={reviewContext}
      ></textarea>
    </div>

    <!-- Merge-readiness banner (item 13) -->
    {#if review.blocker_count != null || review.verdict != null}
      <div class="rp-merge-gate" class:rp-merge-ok={mergeReady} class:rp-merge-blocked={!mergeReady}>
        {#if mergeReady}
          <Icon name="check" size={13} /> Merge-ready — no bug-severity blockers
        {:else}
          <Icon name="zap" size={13} /> {blockerCount} blocker{blockerCount === 1 ? '' : 's'} before merge
        {/if}
        {#if review.verdict}
          <span class="rp-verdict">Verdict: {review.verdict}</span>
        {/if}
      </div>
    {/if}

    <!-- Per-agent breakdown: open each agent's (archived) session + its own
         findings. Shared with the local review; excludes the summarizer. -->
    {#if review.agents.length > 1}
      <ReviewAgents {review} view="done" onretried={onAgentRetried} />
    {/if}

    {#if review.comments.length === 0}
      <p class="dim" style="font-size: 12.5px; padding: 16px 0">No comments generated.</p>
    {:else}
      <div class="rp-list">
        {#each review.comments as c (c.id)}
          {@const snippetLines = getSnippetLines(c)}
          {@const defaultExpanded = isDiffExpandedDefault(c)}
          {@const expanded = isDiffExpanded(c.id, defaultExpanded)}
          <div class="rp-comment card">
            <div class="rp-comment-head">
              <span class="severity-chip sev-{c.severity}">{c.severity}</span>
              {#if c.path !== null}
                <span class="mono rp-loc">{c.path}{c.line !== null ? `:${c.line}` : ''}</span>
              {/if}
              <span class="grow"></span>
              {#if c.state === 'draft'}
                <button
                  class="btn small primary"
                  disabled={!!actionBusy[c.id]}
                  onclick={() => approveComment(c)}
                >
                  {actionBusy[c.id] === 'approve' ? 'Posting…' : 'Approve'}
                </button>
                <button
                  class="btn small ghost"
                  disabled={!!actionBusy[c.id]}
                  onclick={() => declineComment(c)}
                >
                  {actionBusy[c.id] === 'decline' ? 'Declining…' : 'Decline'}
                </button>
              {:else if c.state === 'approved'}
                <span class="chip ok rp-badge">
                  <Icon name="check" size={10} /> posted
                </span>
              {:else}
                <span class="chip rp-badge dim">declined</span>
              {/if}
            </div>
            <p class="rp-comment-body">{c.body}</p>

            <!-- Diff snippet -->
            {#if snippetLines !== null}
              <div class="rp-diff-toggle-row">
                <button
                  class="rp-diff-toggle"
                  onclick={() => toggleDiff(c.id, defaultExpanded)}
                  aria-expanded={expanded}
                >
                  {expanded ? 'hide diff ▾' : 'show diff ▸'}
                </button>
              </div>
              {#if expanded}
                <div class="rp-diff-snippet">
                  {#each snippetLines as dl}
                    {@const isTarget = (dl.new_line !== null && dl.new_line === c.line) || (dl.new_line === null && dl.old_line !== null && dl.old_line === c.line)}
                    <div
                      class="rp-diff-line rp-diff-{dl.origin}{isTarget ? ' rp-diff-target' : ''}"
                    >
                      <span class="rp-diff-gutter">
                        {dl.origin === 'add' ? '+' : dl.origin === 'del' ? '-' : ' '}
                      </span>
                      <span class="rp-diff-linenum">{dl.new_line ?? dl.old_line ?? ''}</span>
                      <span class="rp-diff-content">{dl.content}</span>
                    </div>
                  {/each}
                </div>
              {/if}
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}

  <!-- Past reviews history (runs index 1+) -->
  {#if history.length > 1}
    {@const headerOpen = !!historyExpanded['_header' as unknown as number]}
    <div class="rp-history">
      <button
        class="rp-history-toggle"
        onclick={() => { historyExpanded = { ...historyExpanded, ['_header' as unknown as number]: !headerOpen }; }}
        aria-expanded={headerOpen}
      >
        Past reviews ({history.length - 1}){headerOpen ? ' ▾' : ' ▸'}
      </button>
      {#if headerOpen}
        <div class="rp-history-list">
          {#each history.slice(1) as run, i (run.id)}
            {@const isOpen = !!historyExpanded[i]}
            <div class="rp-history-run card">
              <button
                class="rp-history-run-header"
                onclick={() => toggleHistoryRun(i)}
                aria-expanded={isOpen}
              >
                <span class="dim" style="font-size:11px">{timeAgo(run.created_at)}</span>
                <span class="chip rp-status-{run.status}" style="font-size:10px;padding:1px 5px">{run.status}</span>
                {#if run.agents && run.agents.length > 0}
                  <span class="dim" style="font-size:10.5px">{run.agents.filter(a => a.status === 'done').length}/{run.agents.length} agents</span>
                {/if}
                <span class="dim" style="font-size:10.5px">{run.comments.length} comment{run.comments.length === 1 ? '' : 's'}</span>
                <span class="grow"></span>
                <span class="dim" style="font-size:10px">{isOpen ? '▾' : '▸'}</span>
              </button>
              {#if isOpen}
                <div class="rp-history-run-body">
                  {#if run.comments.length === 0}
                    <p class="dim" style="font-size:11.5px;padding:4px 0">No comments for this run.</p>
                  {:else}
                    {#each run.comments as c (c.id)}
                      <div class="rp-comment card rp-history-comment">
                        <div class="rp-comment-head">
                          <span class="severity-chip sev-{c.severity}">{c.severity}</span>
                          {#if c.path !== null}
                            <span class="mono rp-loc">{c.path}{c.line !== null ? `:${c.line}` : ''}</span>
                          {/if}
                          <span class="grow"></span>
                          {#if c.state === 'approved'}
                            <span class="chip ok rp-badge"><Icon name="check" size={10} /> posted</span>
                          {:else if c.state === 'declined'}
                            <span class="chip rp-badge dim">declined</span>
                          {:else}
                            <span class="chip rp-badge dim">draft</span>
                          {/if}
                        </div>
                        <p class="rp-comment-body">{c.body}</p>
                      </div>
                    {/each}
                  {/if}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<!-- Configure agents modal -->
{#if showConfig}
  <Modal title="Configure review agents" width={580} onclose={closeConfig}>
    {#snippet children()}
      {#if configLoading}
        <Skeleton rows={3} height={36} />
      {:else}
        <p class="dim cfg-note">
          Each agent reviews the diff with its own lens. The summarizer merges all results into the final comment list.
        </p>

        <h3 class="cfg-section-title">Review agents</h3>
        {#each editAgents as agent, i (i)}
          {@const agentProviders = agent.providers && agent.providers.length > 0 ? agent.providers : [agent.provider || 'claude']}
          <div class="cfg-agent-row card">
            <div class="cfg-agent-fields">
              <div class="cfg-field">
                <span class="cfg-label">Name</span>
                <input class="cfg-input" bind:value={editAgents[i].name} />
              </div>
              <div class="cfg-field">
                <span class="cfg-label">Run on (CLIs)</span>
                <div class="cfg-provider-checks">
                  {#each PROVIDER_OPTIONS as p}
                    <label class="cfg-check-label">
                      <input
                        type="checkbox"
                        checked={agentProviders.includes(p)}
                        onchange={() => toggleProvider(i, p)}
                      />
                      {p}
                    </label>
                  {/each}
                </div>
              </div>
              <div class="cfg-field">
                <span class="cfg-label">Lens / instructions</span>
                <textarea class="cfg-textarea" rows={3} bind:value={editAgents[i].prompt}></textarea>
              </div>
            </div>
            <div class="cfg-agent-actions">
              <button
                class="btn small ghost cfg-save-preset"
                title="Save as preset"
                onclick={() => saveAsPreset(i)}
              >&#9734; Save as preset</button>
              <button class="btn small ghost cfg-remove" onclick={() => removeAgent(i)} aria-label="Remove agent">&#10005;</button>
            </div>
          </div>
        {/each}
        <div class="cfg-add-row">
          <button class="btn small ghost" onclick={addAgent}>+ Add agent</button>
          <button class="btn small ghost" onclick={openPresetMenu}>+ Add preset ▾</button>
        </div>

        <!-- Your presets section -->
        {#if editPresets.length > 0}
          <h3 class="cfg-section-title cfg-presets-title">Your presets</h3>
          <div class="cfg-presets-list">
            {#each editPresets as preset, i (i)}
              <div class="cfg-preset-chip">
                <span class="cfg-preset-name">{preset.name}</span>
                <button
                  class="cfg-preset-action"
                  title="Add to agents"
                  onclick={() => addPresetToAgents(preset)}
                  aria-label="Add preset to agents"
                >&#43;</button>
                <button
                  class="cfg-preset-action cfg-preset-del"
                  title="Delete preset"
                  onclick={() => removePreset(i)}
                  aria-label="Delete preset"
                >&#10005;</button>
              </div>
            {/each}
          </div>
        {/if}

        <h3 class="cfg-section-title" style="margin-top:18px">Summarizer</h3>
        <div class="cfg-agent-row card">
          <div class="cfg-agent-fields">
            <div class="cfg-field">
              <span class="cfg-label">Name</span>
              <input class="cfg-input" bind:value={editSummarizer.name} />
            </div>
            <div class="cfg-field">
              <span class="cfg-label">Provider <span class="dim">(uses its default model)</span></span>
              <select class="cfg-select" bind:value={editSummarizer.provider}>
                {#each PROVIDER_OPTIONS as p}
                  <option value={p}>{p}</option>
                {/each}
              </select>
            </div>
            <div class="cfg-field">
              <span class="cfg-label">Merge / dedupe instructions</span>
              <textarea class="cfg-textarea" rows={3} bind:value={editSummarizer.prompt}></textarea>
            </div>
          </div>
        </div>
      {/if}
    {/snippet}
    {#snippet footer()}
      <button class="btn ghost" onclick={closeConfig}>Cancel</button>
      <button class="btn primary" disabled={configSaving || configLoading} onclick={saveConfig}>
        {configSaving ? 'Saving…' : 'Save'}
      </button>
    {/snippet}
  </Modal>
{/if}

<!-- Jira story picker modal -->
{#if showJiraPicker}
  <Modal title="Attach Jira story" width={520} onclose={closeJiraPicker}>
    {#snippet children()}
      <JiraIssuePicker onpick={(iss) => { attachedIssue = iss; closeJiraPicker(); }} />
    {/snippet}
    {#snippet footer()}
      <button class="btn ghost" onclick={closeJiraPicker}>Cancel</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .rp {
    padding: 4px 0 32px;
  }

  .rp-merge-gate {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    margin: 0 0 8px;
    border-radius: var(--radius-s, 4px);
    font-size: 11.5px;
    font-weight: 500;
  }
  .rp-merge-ok {
    background: color-mix(in srgb, #22c55e 12%, transparent);
    border: 1px solid color-mix(in srgb, #22c55e 40%, var(--border));
    color: #15803d;
  }
  .rp-merge-blocked {
    background: color-mix(in srgb, #ef4444 10%, transparent);
    border: 1px solid color-mix(in srgb, #ef4444 35%, var(--border));
    color: #b91c1c;
  }
  .rp-verdict {
    margin-left: auto;
    font-weight: 400;
    font-size: 11px;
    opacity: 0.8;
  }

  /* Pre-check banner: missing / outdated review skills */
  .rp-precheck {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    margin: 0 0 10px;
    border: 1px solid color-mix(in srgb, #e0a000 35%, var(--border));
    background: color-mix(in srgb, #e0a000 10%, transparent);
    border-radius: var(--radius-s, 4px);
    font-size: 11.5px;
    line-height: 1.4;
  }
  .rp-precheck-icon {
    color: #b07d00;
    flex-shrink: 0;
  }
  .rp-precheck-msg {
    flex: 1;
    color: var(--text);
    min-width: 0;
  }
  .rp-precheck-btn {
    flex-shrink: 0;
    font-size: 11px;
    white-space: nowrap;
  }
  .rp-precheck-dismiss {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0 2px;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1;
    flex-shrink: 0;
  }
  .rp-precheck-dismiss:hover {
    color: var(--text);
  }

  /* No-review state with configure button */
  .rp-no-review {
    position: relative;
  }
  .rp-cfg-btn {
    position: absolute;
    top: 0;
    right: 0;
    font-size: 11px;
  }

  /* Running state */
  .rp-running-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 0 10px;
  }
  .rp-running-title {
    font-size: 13px;
    font-weight: 600;
  }
  .spinner {
    width: 18px;
    height: 18px;
    border: 2.5px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    vertical-align: middle;
    margin-right: 3px;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Live agent cards */
  .rp-agents {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 4px;
  }
  .rp-agent {
    padding: 8px 12px;
  }
  .rp-agent-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .rp-agent-name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .rp-agent-chip {
    font-size: 10.5px;
  }
  .rp-agent-note {
    margin: 4px 0 0;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.4;
  }
  .rp-agent-count {
    font-size: 11px;
    display: block;
    margin-top: 3px;
  }

  /* Status pills */
  .rp-status-pill {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 6px;
    border-radius: var(--radius-s, 4px);
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .rp-status-pending {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }
  .rp-status-running {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .rp-status-done {
    background: color-mix(in srgb, var(--status-idle, #6bbf6b) 15%, transparent);
    color: var(--status-idle, #3a8c3a);
  }
  .rp-status-error {
    background: color-mix(in srgb, var(--status-exited) 15%, transparent);
    color: var(--status-exited);
  }
  .rp-status-waiting {
    background: color-mix(in srgb, #e0a000 20%, transparent);
    color: #b07d00;
  }

  /* Per-agent: "waiting for input" callout + expandable findings */
  .rp-agent-waiting {
    margin: 6px 0 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: #b07d00;
  }
  .rp-term {
    height: 360px;
    margin: 8px 0 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: #1b1b1b;
  }
  .rp-agent-findings {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .rp-finding {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 11.5px;
    line-height: 1.4;
  }
  .rp-finding-body {
    flex: 1;
    min-width: 0;
  }

  /* Error state */
  .rp-error {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    color: var(--status-exited);
    margin-top: 8px;
  }
  .rp-error-msg {
    flex: 1;
    font-size: 12.5px;
  }

  /* Done: header row */
  .rp-header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
  }
  .rp-stats {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
  }
  .rp-stat {
    font-size: 12.5px;
    font-weight: 600;
  }

  /* Comment list */
  .rp-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .rp-comment {
    padding: 10px 14px;
  }
  .rp-comment-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
    flex-wrap: wrap;
  }
  .rp-comment-body {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.55;
    white-space: pre-wrap;
  }
  .rp-loc {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 280px;
  }
  .rp-badge {
    font-size: 11px;
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  /* Severity chips */
  .severity-chip {
    display: inline-block;
    padding: 2px 7px;
    border-radius: var(--radius-s, 4px);
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .sev-info {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .sev-warn {
    background: color-mix(in srgb, var(--status-idle, #e6a817) 15%, transparent);
    color: var(--status-idle, #c8920a);
  }
  .sev-bug {
    background: color-mix(in srgb, var(--status-exited) 15%, transparent);
    color: var(--status-exited);
  }

  /* Diff snippet */
  .rp-diff-toggle-row {
    margin-top: 6px;
  }
  .rp-diff-toggle {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 10.5px;
    color: var(--text-dim);
    padding: 0;
    line-height: 1.4;
  }
  .rp-diff-toggle:hover {
    color: var(--text);
  }
  .rp-diff-snippet {
    margin-top: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    overflow-x: auto;
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    line-height: 1.45;
  }
  .rp-diff-line {
    display: flex;
    align-items: baseline;
    gap: 0;
    padding: 0 6px;
    white-space: pre;
  }
  .rp-diff-add {
    background: color-mix(in srgb, #3a8c3a 12%, transparent);
    color: var(--text);
  }
  .rp-diff-del {
    background: color-mix(in srgb, var(--status-exited, #c0392b) 12%, transparent);
    color: var(--text);
  }
  .rp-diff-context {
    color: var(--text-dim);
  }
  .rp-diff-target {
    outline: 1px solid color-mix(in srgb, var(--accent) 50%, transparent);
    outline-offset: -1px;
    font-weight: 600;
  }
  .rp-diff-gutter {
    width: 12px;
    flex-shrink: 0;
    user-select: none;
    opacity: 0.6;
  }
  .rp-diff-linenum {
    width: 32px;
    flex-shrink: 0;
    text-align: right;
    padding-right: 8px;
    opacity: 0.45;
    user-select: none;
  }
  .rp-diff-content {
    flex: 1;
  }

  /* Config modal */
  .cfg-note {
    font-size: 12px;
    margin: 0 0 14px;
  }
  .cfg-add-row {
    display: flex;
    gap: 8px;
  }
  .cfg-section-title {
    font-size: 12.5px;
    font-weight: 600;
    margin: 0 0 8px;
  }
  .cfg-agent-row {
    padding: 10px 12px;
    margin-bottom: 8px;
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }
  .cfg-agent-fields {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cfg-agent-actions {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex-shrink: 0;
    align-items: flex-end;
  }
  .cfg-save-preset {
    font-size: 10.5px;
    padding: 2px 6px;
    white-space: nowrap;
  }
  .cfg-field {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .cfg-label {
    font-size: 11px;
    color: var(--text-dim);
    font-weight: 500;
  }
  .cfg-input,
  .cfg-select,
  .cfg-textarea {
    width: 100%;
    background: var(--input-bg, var(--surface-raised));
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    color: var(--text);
    font-size: 12.5px;
    padding: 4px 8px;
    box-sizing: border-box;
  }
  .cfg-textarea {
    resize: vertical;
    font-family: var(--font-mono, monospace);
    font-size: 11.5px;
    line-height: 1.5;
  }
  .cfg-remove {
    flex-shrink: 0;
    font-size: 11px;
    padding: 2px 6px;
  }
  .cfg-provider-checks {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    padding: 4px 0;
  }
  .cfg-check-label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12.5px;
    cursor: pointer;
    user-select: none;
  }
  .grow {
    flex: 1;
  }

  /* Custom presets section */
  .cfg-presets-title {
    margin-top: 16px;
  }
  .cfg-presets-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 4px;
  }
  .cfg-preset-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: 20px;
    padding: 3px 8px 3px 10px;
    font-size: 11.5px;
  }
  .cfg-preset-name {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cfg-preset-action {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0 2px;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1;
    display: inline-flex;
    align-items: center;
  }
  .cfg-preset-action:hover {
    color: var(--text);
  }
  .cfg-preset-del:hover {
    color: var(--status-exited);
  }

  /* Jira attachment row */
  .rp-jira-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
    margin-bottom: 4px;
  }
  .rp-jira-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rp-jira-remove {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0 2px;
    font-size: 10px;
    color: var(--text-dim);
    line-height: 1;
  }
  .rp-jira-remove:hover {
    color: var(--text);
  }

  /* Free-text reviewer-guidance input */
  .rp-context-row {
    margin-top: 6px;
    margin-bottom: 4px;
  }
  .rp-context-input {
    width: 100%;
    background: var(--input-bg, var(--surface-raised));
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    color: var(--text);
    font-size: 12px;
    line-height: 1.5;
    padding: 6px 8px;
    box-sizing: border-box;
    resize: vertical;
  }
  .rp-context-input::placeholder {
    color: var(--text-dim);
  }

  /* History section */
  .rp-history {
    margin-top: 20px;
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }
  .rp-history-toggle {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-dim);
    padding: 0;
    line-height: 1.4;
  }
  .rp-history-toggle:hover {
    color: var(--text);
  }
  .rp-history-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }
  .rp-history-run {
    padding: 0;
    overflow: hidden;
  }
  .rp-history-run-header {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    background: none;
    border: none;
    cursor: pointer;
    padding: 8px 12px;
    text-align: left;
    flex-wrap: wrap;
  }
  .rp-history-run-header:hover {
    background: var(--surface-hover, var(--surface-raised));
  }
  .rp-history-run-body {
    padding: 4px 8px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rp-history-comment {
    padding: 8px 12px;
    opacity: 0.85;
  }

</style>
