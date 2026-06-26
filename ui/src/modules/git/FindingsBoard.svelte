<script lang="ts">
  // The Review Findings workflow board. For a completed review it lists the
  // persisted Finding rows (GET /reviews/{id}/findings) as expandable cards — a
  // status chip + severity chip + category + path:Lstart–Lend + reviewer +
  // artifact chips (commit/test/Jira), the 7 action buttons (FindingActions), and
  // on expand the evidence/reasoning/suggested-fix + the event timeline
  // (GET /findings/{id}). Filters by status + severity; a header with counts and a
  // Proof Pack button. Subscribes to the finding WS bus and refetches on match —
  // the same pattern ReviewPanel uses for review_changed.
  import { listFindings, getFinding } from '../../lib/api/client';
  import type {
    Finding,
    FindingDetail,
    FindingStatus,
    FindingSeverity,
  } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { findingBus } from '../../lib/events.svelte';
  import FindingActions from './FindingActions.svelte';
  import ProofPackView from './ProofPackView.svelte';

  interface Props {
    reviewId: string;
    workspaceId: string;
  }
  let { reviewId, workspaceId }: Props = $props();

  let findings: Finding[] = $state([]);
  let loading = $state(true);
  let expanded: Record<string, boolean> = $state({});
  let details: Record<string, FindingDetail> = $state({});
  let detailLoading: Record<string, boolean> = $state({});

  // Filters
  let statusFilter: FindingStatus | 'all' = $state('all');
  let sevFilter: FindingSeverity | 'all' = $state('all');

  let showProofPack = $state(false);

  const STATUSES: FindingStatus[] = ['open', 'accepted', 'fixed', 'verified', 'false_positive', 'waived'];
  const SEVERITIES: FindingSeverity[] = ['critical', 'high', 'medium', 'low', 'info'];

  // Initial + reviewId-change load.
  $effect(() => {
    void load(reviewId);
  });

  // WS bus: refetch when a finding under THIS review changed / an action started.
  // (Keep this read-only of findingBus + reviewId; reload mutates state in an
  // effect, never in a derived.)
  $effect(() => {
    const _tick = findingBus.tick;
    if (_tick > 0 && findingBus.reviewId && findingBus.reviewId === reviewId) {
      void reload();
    }
  });

  async function load(rid: string): Promise<void> {
    loading = true;
    try {
      findings = await listFindings(rid);
    } catch (e) {
      toasts.error('Could not load findings', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  /** Silent refetch (no loading flicker) — used by the WS bus. */
  async function reload(): Promise<void> {
    try {
      const next = await listFindings(reviewId);
      findings = next;
      // Refresh any open detail so its timeline reflects the new events.
      for (const id of Object.keys(expanded)) {
        if (expanded[id] && details[id]) void loadDetail(id, true);
      }
    } catch {
      // non-blocking
    }
  }

  function patchFinding(f: Finding): void {
    findings = findings.map((x) => (x.id === f.id ? f : x));
  }

  function toggle(id: string): void {
    const open = !expanded[id];
    expanded = { ...expanded, [id]: open };
    if (open && !details[id] && !detailLoading[id]) void loadDetail(id);
  }

  async function loadDetail(id: string, silent = false): Promise<void> {
    if (!silent) detailLoading = { ...detailLoading, [id]: true };
    try {
      const d = await getFinding(id);
      details = { ...details, [id]: d };
      patchFinding(d.finding);
    } catch {
      // non-blocking — the card still shows the summary fields it already has
    } finally {
      if (!silent) detailLoading = { ...detailLoading, [id]: false };
    }
  }

  // --- derived (pure) --------------------------------------------------------
  const statusCounts = $derived.by(() => {
    const m: Record<string, number> = {};
    for (const f of findings) m[f.status] = (m[f.status] ?? 0) + 1;
    return m;
  });
  const sevCounts = $derived.by(() => {
    const m: Record<string, number> = {};
    for (const f of findings) m[f.severity] = (m[f.severity] ?? 0) + 1;
    return m;
  });
  const filtered = $derived.by(() =>
    findings.filter(
      (f) =>
        (statusFilter === 'all' || f.status === statusFilter) &&
        (sevFilter === 'all' || f.severity === sevFilter),
    ),
  );

  // --- helpers ---------------------------------------------------------------
  function loc(f: Finding): string {
    if (!f.path) return '';
    if (f.line == null) return f.path;
    if (f.line_end != null && f.line_end !== f.line) return `${f.path}:L${f.line}–L${f.line_end}`;
    return `${f.path}:L${f.line}`;
  }
  function statusLabel(s: string): string {
    return s.replace(/_/g, ' ');
  }
  function transitionLabel(from: string | null, to: string | null): string {
    if (from && to) return ` · ${from} → ${to}`;
    return '';
  }
</script>

<div class="fb" data-workspace-id={workspaceId}>
  <!-- Header: counts + Proof Pack -->
  <div class="fb-header">
    <span class="fb-count">{findings.length} finding{findings.length === 1 ? '' : 's'}</span>
    {#if statusCounts['verified']}
      <span class="chip status-verified fb-hchip">{statusCounts['verified']} verified</span>
    {/if}
    {#if statusCounts['open']}
      <span class="chip status-open fb-hchip">{statusCounts['open']} open</span>
    {/if}
    <span class="grow"></span>
    <button class="btn small ghost" onclick={() => (showProofPack = true)} disabled={findings.length === 0}>
      Proof Pack
    </button>
  </div>

  {#if loading}
    <Skeleton rows={3} height={48} />
  {:else if findings.length === 0}
    <p class="dim fb-empty">No tracked findings for this review yet.</p>
  {:else}
    <!-- Filters -->
    <div class="fb-filters">
      <div class="fb-filter-row">
        <span class="fb-filter-label">Status</span>
        <button class="fb-pill" class:active={statusFilter === 'all'} onclick={() => (statusFilter = 'all')}>
          All
        </button>
        {#each STATUSES as s}
          {#if statusCounts[s]}
            <button
              class="fb-pill status-{s}"
              class:active={statusFilter === s}
              onclick={() => (statusFilter = statusFilter === s ? 'all' : s)}
            >
              {statusLabel(s)} {statusCounts[s]}
            </button>
          {/if}
        {/each}
      </div>
      <div class="fb-filter-row">
        <span class="fb-filter-label">Severity</span>
        <button class="fb-pill" class:active={sevFilter === 'all'} onclick={() => (sevFilter = 'all')}>
          All
        </button>
        {#each SEVERITIES as s}
          {#if sevCounts[s]}
            <button
              class="fb-pill sev2-{s}"
              class:active={sevFilter === s}
              onclick={() => (sevFilter = sevFilter === s ? 'all' : s)}
            >
              {s} {sevCounts[s]}
            </button>
          {/if}
        {/each}
      </div>
    </div>

    <!-- Finding cards -->
    <div class="fb-list">
      {#each filtered as f (f.id)}
        {@const isOpen = !!expanded[f.id]}
        {@const detail = details[f.id]}
        <div class="fb-card card" class:fb-regressed={f.regressed}>
          <button
            class="fb-card-head"
            onclick={() => toggle(f.id)}
            aria-expanded={isOpen}
          >
            <span class="chip sev2-{f.severity}">{f.severity}</span>
            <span class="chip status-{f.status}">{statusLabel(f.status)}</span>
            {#if f.regressed}<span class="chip fb-regress-chip">regressed</span>{/if}
            {#if f.requires_human_approval && !f.approved_at}
              <span class="chip fb-gate-chip">needs approval</span>
            {/if}
            <span class="fb-title">{f.title || f.body.split('\n')[0]}</span>
            <span class="grow"></span>
            <span class="dim fb-caret">{isOpen ? '▾' : '▸'}</span>
          </button>

          <div class="fb-meta">
            {#if f.category}<span class="fb-cat">{f.category}</span>{/if}
            {#if loc(f)}<span class="mono fb-loc">{loc(f)}</span>{/if}
            {#if f.reviewer}<span class="dim fb-reviewer">· {f.reviewer}</span>{/if}
            {#if f.occurrence_count > 1}<span class="dim">· seen ×{f.occurrence_count}</span>{/if}
            <span class="grow"></span>
            {#if f.linked_commit}<span class="chip fb-artifact">commit {f.linked_commit.slice(0, 9)}</span>{/if}
            {#if f.linked_test}<span class="chip fb-artifact" title={f.linked_test}>test</span>{/if}
            {#if f.jira_key}
              {#if f.jira_url}
                <a class="chip fb-artifact fb-jira" href={f.jira_url} target="_blank" rel="noreferrer">{f.jira_key}</a>
              {:else}
                <span class="chip fb-artifact">{f.jira_key}</span>
              {/if}
            {/if}
          </div>

          {#if isOpen}
            <div class="fb-detail">
              {#if f.evidence}
                <div class="fb-field">
                  <span class="fb-field-label">Evidence</span>
                  <pre class="fb-pre">{f.evidence}</pre>
                </div>
              {/if}
              {#if f.agent_reasoning_summary}
                <div class="fb-field">
                  <span class="fb-field-label">Agent reasoning</span>
                  <p class="fb-field-text">{f.agent_reasoning_summary}</p>
                </div>
              {/if}
              {#if f.suggested_fix}
                <div class="fb-field">
                  <span class="fb-field-label">Suggested fix</span>
                  <pre class="fb-pre">{f.suggested_fix}</pre>
                </div>
              {/if}
              {#if !f.evidence && !f.agent_reasoning_summary && !f.suggested_fix}
                <p class="fb-field-text">{f.body}</p>
              {/if}

              <!-- Timeline -->
              <div class="fb-field">
                <span class="fb-field-label">Timeline</span>
                {#if detailLoading[f.id] && !detail}
                  <p class="dim" style="font-size:11.5px">Loading…</p>
                {:else if detail && detail.events.length > 0}
                  <ul class="fb-timeline">
                    {#each detail.events as ev (ev.id)}
                      <li class="fb-event">
                        <span class="fb-event-kind">{statusLabel(ev.kind)}</span>
                        <span class="dim fb-event-meta">{ev.actor}{transitionLabel(ev.from_status, ev.to_status)}</span>
                      </li>
                    {/each}
                  </ul>
                {:else if detail}
                  <p class="dim" style="font-size:11.5px">No events yet.</p>
                {/if}
              </div>

              <!-- Action bar -->
              <FindingActions finding={f} onupdated={patchFinding} />
            </div>
          {/if}
        </div>
      {/each}
      {#if filtered.length === 0}
        <p class="dim fb-empty">No findings match the current filters.</p>
      {/if}
    </div>
  {/if}
</div>

{#if showProofPack}
  <ProofPackView {reviewId} onclose={() => (showProofPack = false)} />
{/if}

<style>
  .fb {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 4px 0 8px;
  }
  .fb-header {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .fb-count { font-size: 12.5px; font-weight: 600; }
  .fb-hchip { font-size: 10.5px; }
  .fb-empty { font-size: 12.5px; padding: 12px 0; }

  /* Filters */
  .fb-filters { display: flex; flex-direction: column; gap: 6px; }
  .fb-filter-row {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-wrap: wrap;
  }
  .fb-filter-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    width: 56px;
    flex-shrink: 0;
  }
  .fb-pill {
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
    border-radius: 999px;
    font-size: 11px;
    padding: 2px 9px;
    cursor: pointer;
    text-transform: capitalize;
    line-height: 1.6;
  }
  .fb-pill:hover { color: var(--text); }
  /* Active filter pill: high-contrast light-green + black (selection-contrast rule). */
  .fb-pill.active {
    background: #7ee787;
    color: #000;
    border-color: #7ee787;
    font-weight: 600;
  }

  /* Cards */
  .fb-list { display: flex; flex-direction: column; gap: 8px; }
  .fb-card { padding: 8px 12px; }
  .fb-regressed { border-color: color-mix(in srgb, var(--status-warn) 45%, var(--border)); }
  .fb-card-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    text-align: start;
    flex-wrap: wrap;
    color: var(--text);
  }
  .fb-title { font-size: 12.5px; font-weight: 600; min-width: 0; }
  .fb-caret { font-size: 11px; }
  .fb-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 5px;
    font-size: 11px;
  }
  .fb-cat {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    border-radius: var(--radius-s, 4px);
    padding: 1px 6px;
    text-transform: capitalize;
  }
  .fb-loc {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .fb-reviewer { font-size: 11px; }
  .fb-artifact {
    font-size: 10px;
    background: color-mix(in srgb, #7ee787 16%, transparent);
    color: var(--text);
  }
  .fb-jira { text-decoration: none; }
  .fb-jira:hover { text-decoration: underline; }
  .fb-regress-chip {
    font-size: 10px;
    background: color-mix(in srgb, var(--status-warn) 18%, transparent);
    color: var(--status-warn);
  }
  .fb-gate-chip {
    font-size: 10px;
    background: color-mix(in srgb, var(--status-warn) 20%, transparent);
    color: var(--status-warn);
  }

  /* Expanded detail */
  .fb-detail {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .fb-field { display: flex; flex-direction: column; gap: 3px; }
  .fb-field-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .fb-field-text { margin: 0; font-size: 12px; line-height: 1.5; white-space: pre-wrap; }
  .fb-pre {
    margin: 0;
    padding: 6px 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s, 4px);
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-x: auto;
    max-height: 200px;
  }
  .fb-timeline { list-style: none; margin: 0; padding: 0; }
  .fb-event { font-size: 11px; line-height: 1.55; display: flex; gap: 6px; flex-wrap: wrap; }
  .fb-event-kind { font-weight: 600; text-transform: capitalize; }

  .grow { flex: 1; }
  .dim { color: var(--text-dim); }
  .mono { font-family: var(--font-mono, monospace); }

  /* Status chips (shared vocabulary; high-contrast light-green + black for verified). */
  .chip.status-open { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }
  .chip.status-accepted { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .chip.status-fixed { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); }
  .chip.status-verified { background: #7ee787; color: #000; font-weight: 700; }
  .chip.status-false_positive { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }
  .chip.status-waived { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }

  /* Severity chips (red for blocker severities critical/high). */
  .chip.sev2-critical { background: var(--status-exited); color: #fff; font-weight: 700; }
  .chip.sev2-high { background: color-mix(in srgb, var(--status-exited) 20%, transparent); color: var(--status-exited); }
  .chip.sev2-medium { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); }
  .chip.sev2-low { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent); }
  .chip.sev2-info { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }

  /* The filter pills reuse status-/sev2- classes for their idle tint, but the
     .active rule above (green) must win — these are lower specificity by design. */
  .fb-pill.status-verified:not(.active) { color: var(--text); }

  @media (max-width: 1024px) {
    .fb-loc { max-width: 100%; }
    .fb-pill { min-height: 30px; padding-block: 4px; }
  }
</style>
