<script lang="ts">
  import { plural } from '../../lib/plural';
  // The Review Findings workflow board. For a completed review it lists the
  // persisted Finding rows (GET /reviews/{id}/findings) as expandable cards — a
  // status chip + severity chip + category + path:Lstart–Lend + reviewer +
  // artifact chips (commit/test/Jira), the 7 action buttons (FindingActions), and
  // on expand the evidence/reasoning/suggested-fix + the event timeline
  // (GET /findings/{id}). Filters by status + severity; a header with counts and a
  // Proof Pack button. Subscribes to the finding WS bus and refetches on match —
  // the same pattern ReviewPanel uses for review_changed.
  import { sentenceCase, severityLabel } from '../../lib/labels';
  import Badge from '../../lib/components/Badge.svelte';
  import type { BadgeTone } from '../../lib/status';
  import Icon from '../../lib/components/Icon.svelte';
  import { listFindings, getFinding } from '../../lib/api/client';
  import type {
    Finding,
    FindingDetail,
    FindingStatus,
    FindingSeverity,
  } from '../../lib/api/types';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
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
  /** Failed list load — inline with Retry, never "No tracked findings". */
  let loadError = $state<string | null>(null);
  /** Failed silent refetch (WS bus) — the findings stay, with a stale bar + Retry. */
  let reloadError = $state<string | null>(null);
  /** Per-finding failed detail/timeline load → "Couldn’t load the timeline · Retry". */
  let detailError: Record<string, boolean> = $state({});
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
      loadError = null;
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  /** Silent refetch (no loading flicker) — used by the WS bus. */
  async function reload(): Promise<void> {
    try {
      const next = await listFindings(reviewId);
      findings = next;
      loadError = null;
      reloadError = null;
      // Refresh any open detail so its timeline reflects the new events.
      for (const id of Object.keys(expanded)) {
        if (expanded[id] && details[id]) void loadDetail(id, true);
      }
    } catch (e) {
      // Non-blocking: keep what is on screen and say the refresh failed.
      reloadError = loadErrorText(e);
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
      if (detailError[id]) detailError = { ...detailError, [id]: false };
    } catch {
      // Non-blocking — the card still shows the summary fields it already has;
      // a silent (WS-driven) refresh failure keeps the last good timeline.
      if (!silent) detailError = { ...detailError, [id]: true };
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
  // Wire values (`false_positive`) never go on screen — lib/labels words them.
  const statusLabel = sentenceCase;
  function transitionLabel(from: string | null, to: string | null): string {
    if (from && to) return ` · ${statusLabel(from)} → ${statusLabel(to)}`;
    return '';
  }

  /** Finding status / severity → the shared Badge tone. */
  const STATUS_TONE: Record<string, BadgeTone> = { accepted: 'accent', fixed: 'warn', verified: 'ok' };
  function statusTone(st: string): BadgeTone {
    return STATUS_TONE[st] ?? 'neutral';
  }
  const SEV_TONE: Record<string, BadgeTone> = { critical: 'bad', high: 'bad', medium: 'warn', low: 'info' };
  function sevTone(sev: string): BadgeTone {
    return SEV_TONE[sev] ?? 'neutral';
  }
</script>

<div class="fb" data-workspace-id={workspaceId}>
  <!-- Header: counts + Proof Pack -->
  <div class="fb-header">
    <span class="fb-count">{plural(findings.length, 'finding')}</span>
    {#if statusCounts['verified']}
      <Badge tone="ok" label={`${statusCounts['verified']} verified`} />
    {/if}
    {#if statusCounts['open']}
      <Badge label={`${statusCounts['open']} open`} />
    {/if}
    <span class="grow"></span>
    <button class="btn small ghost" onclick={() => (showProofPack = true)} disabled={findings.length === 0}>
      Proof Pack
    </button>
  </div>

  <!-- error + nothing → inline error; error + findings → the board with a stale bar. -->
  <LoadState
    what="findings"
    {loading}
    error={loadError ?? reloadError}
    empty={findings.length === 0}
    rows={3}
    onretry={() => void load(reviewId)}
  >
    {#snippet emptyView()}
      <p class="dim fb-empty">No tracked findings for this review yet.</p>
    {/snippet}
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
            <Badge tone={sevTone(f.severity)} label={severityLabel(f.severity)} />
            <Badge tone={statusTone(f.status)} label={statusLabel(f.status)} />
            {#if f.regressed}<Badge tone="warn" label="Regressed" />{/if}
            {#if f.requires_human_approval && !f.approved_at}
              <Badge tone="warn" label="Needs approval" />
            {/if}
            <span class="fb-title" title={f.title || f.body.split('\n')[0]}>{f.title || f.body.split('\n')[0]}</span>
            <span class="grow"></span>
            <span class="dim fb-caret" aria-hidden="true"><Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} /></span>
          </button>

          <div class="fb-meta">
            {#if f.category}<span class="fb-cat">{f.category}</span>{/if}
            {#if loc(f)}<span class="mono fb-loc">{loc(f)}</span>{/if}
            {#if f.reviewer}<span class="dim fb-reviewer">· {f.reviewer}</span>{/if}
            {#if f.occurrence_count > 1}<span class="dim">· seen {f.occurrence_count} times</span>{/if}
            <span class="grow"></span>
            {#if f.linked_commit}<Badge tone="ok" label={`commit ${f.linked_commit.slice(0, 9)}`} />{/if}
            {#if f.linked_test}<Badge tone="ok" label="Test" title={f.linked_test} />{/if}
            {#if f.jira_key}
              {#if f.jira_url}
                <a class="chip fb-artifact fb-jira" href={f.jira_url} target="_blank" rel="noreferrer">{f.jira_key}</a>
              {:else}
                <Badge tone="ok" label={f.jira_key} />
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
                  <p class="dim" style="font-size: var(--fs-xs)" role="status">Loading the timeline…</p>
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
                  <p class="dim" style="font-size: var(--fs-xs)">No events yet.</p>
                {:else if detailError[f.id]}
                  <p class="fb-tl-error" role="alert">
                    Couldn’t load the timeline ·
                    <button class="btn small ghost" onclick={() => void loadDetail(f.id)}>Retry</button>
                  </p>
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
  </LoadState>
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
  .fb-count { font-size: var(--fs-s); font-weight: 600; }
  .fb-empty { font-size: var(--fs-s); padding: 12px 0; }

  /* Filters */
  .fb-filters { display: flex; flex-direction: column; gap: 6px; }
  .fb-filter-row {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }
  .fb-filter-label {
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
    padding: 2px 8px;
    cursor: pointer;
    text-transform: capitalize;
    line-height: 1.6;
  }
  .fb-pill:hover { color: var(--text); }
  /* Active filter pill: the accent tint every selection uses. */
  .fb-pill.active {
    background: var(--accent-soft);
    color: var(--accent-text);
    border-color: var(--accent-line);
    font-weight: 600;
  }

  /* Cards */
  .fb-list { display: flex; flex-direction: column; gap: 8px; }
  .fb-card { padding: 8px 12px; }
  .fb-regressed { border-color: color-mix(in srgb, var(--warning) 45%, var(--border)); }
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
  .fb-title { font-size: var(--fs-s); font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .fb-caret { display: inline-flex; flex-shrink: 0; }
  .fb-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 4px;
    font-size: var(--fs-xs);
  }
  .fb-cat {
    background: var(--accent-soft);
    color: var(--accent-text);
    border-radius: var(--radius-s);
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
  .fb-reviewer { font-size: var(--fs-xs); }
  .fb-artifact {
    font-size: var(--fs-xs);
    background: color-mix(in srgb, var(--success) 16%, transparent);
    color: var(--text);
  }
  .fb-jira { text-decoration: none; }
  .fb-jira:hover { text-decoration: underline; }

  /* Expanded detail */
  .fb-detail {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .fb-field { display: flex; flex-direction: column; gap: 2px; }
  .fb-field-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .fb-field-text { margin: 0; font-size: var(--fs-s); line-height: 1.5; white-space: pre-wrap; }
  .fb-pre {
    margin: 0;
    padding: 6px 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-x: auto;
    max-height: 200px;
  }
  .fb-tl-error { margin: 0; font-size: var(--fs-xs); color: var(--danger); display: flex; align-items: center; gap: 6px; }
  .fb-timeline { list-style: none; margin: 0; padding: 0; }
  .fb-event { font-size: var(--fs-xs); line-height: 1.55; display: flex; gap: 6px; flex-wrap: wrap; }
  .fb-event-kind { font-weight: 600; text-transform: capitalize; }

  .grow { flex: 1; }
  .dim { color: var(--text-dim); }
  .mono { font-family: var(--font-mono); }



  /* The filter pills reuse status-/sev2- classes for their idle tint, but the
     .active rule above (green) must win — these are lower specificity by design. */
  .fb-pill.status-verified:not(.active) { color: var(--text); }

  @media (max-width: 1024px) {
    .fb-loc { max-width: 100%; }
    .fb-pill { min-height: 30px; padding-block: 4px; }
  }
</style>
