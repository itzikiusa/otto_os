<script lang="ts">
  // The Proof Pack modal: assembles GET /reviews/{id}/proof-pack (summary counts +
  // every finding's evidence/reasoning/artifacts + its event timeline + the repo
  // rules generated from this review), and offers Export which persists a markdown
  // snapshot (POST .../proof-pack/export) and lets the user copy it.
  import { getProofPack, exportProofPack } from '../../lib/api/client';
  import type { ReviewProofPack, Finding, FindingEvent } from '../../lib/api/types';
  import Modal from '../../lib/components/Modal.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    reviewId: string;
    onclose: () => void;
  }
  let { reviewId, onclose }: Props = $props();

  let pack: ReviewProofPack | null = $state(null);
  let loading = $state(true);
  let loadErr = $state('');
  let exporting = $state(false);
  let exportedMd = $state('');

  $effect(() => {
    void load(reviewId);
  });

  async function load(rid: string): Promise<void> {
    loading = true;
    loadErr = '';
    try {
      pack = await getProofPack(rid);
    } catch (e) {
      loadErr = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function doExport(): Promise<void> {
    if (exporting) return;
    exporting = true;
    try {
      const res = await exportProofPack(reviewId);
      exportedMd = res.markdown;
      toasts.success('Proof Pack exported', 'A markdown snapshot was saved.');
    } catch (e) {
      toasts.error('Could not export Proof Pack', e instanceof Error ? e.message : String(e));
    } finally {
      exporting = false;
    }
  }

  async function copyMarkdown(): Promise<void> {
    try {
      await navigator.clipboard.writeText(exportedMd);
      toasts.success('Copied to clipboard');
    } catch {
      toasts.warn('Could not copy', 'Select the text and copy manually.');
    }
  }

  /** A short `path:Lstart–Lend` location label. */
  function loc(f: Finding): string {
    if (!f.path) return '';
    if (f.line == null) return f.path;
    if (f.line_end != null && f.line_end !== f.line) return `${f.path}:L${f.line}–L${f.line_end}`;
    return `${f.path}:L${f.line}`;
  }

  function sortedStatus(by: Record<string, number>): [string, number][] {
    return Object.entries(by).sort((a, b) => b[1] - a[1]);
  }

  /** " · from → to" suffix for an event that carried a status transition. */
  function transitionLabel(ev: FindingEvent): string {
    if (ev.from_status && ev.to_status) return ` · ${ev.from_status} → ${ev.to_status}`;
    return '';
  }
</script>

<Modal title="Proof Pack" width={720} {onclose}>
  {#snippet children()}
    {#if loading}
      <Skeleton rows={4} height={32} />
    {:else if loadErr}
      <p class="pp-err">{loadErr}</p>
    {:else if pack}
      <!-- Summary counts -->
      <div class="pp-summary">
        <span class="pp-stat"><strong>{pack.summary.total}</strong> total</span>
        <span class="pp-stat pp-ok"><strong>{pack.summary.verified}</strong> verified</span>
        <span class="pp-stat"><strong>{pack.summary.fixed}</strong> fixed</span>
        <span class="pp-stat"><strong>{pack.summary.open}</strong> open</span>
        <span class="pp-stat"><strong>{pack.summary.with_commit}</strong> with commit</span>
        <span class="pp-stat"><strong>{pack.summary.with_test}</strong> with test</span>
      </div>
      <div class="pp-breakdown">
        {#each sortedStatus(pack.summary.by_status) as [st, n] (st)}
          <span class="chip pp-chip status-{st}">{st.replace('_', ' ')}: {n}</span>
        {/each}
        {#each sortedStatus(pack.summary.by_severity) as [sv, n] (sv)}
          <span class="chip pp-chip sev2-{sv}">{sv}: {n}</span>
        {/each}
      </div>

      <!-- Per-finding evidence + timeline -->
      <div class="pp-list">
        {#each pack.findings as entry (entry.finding.id)}
          {@const f = entry.finding}
          <div class="pp-finding card">
            <div class="pp-finding-head">
              <span class="chip sev2-{f.severity}">{f.severity}</span>
              <span class="chip status-{f.status}">{f.status.replace('_', ' ')}</span>
              <span class="pp-title">{f.title || f.body.split('\n')[0]}</span>
            </div>
            {#if loc(f)}<div class="pp-loc mono">{loc(f)}</div>{/if}
            {#if f.evidence}
              <pre class="pp-evidence">{f.evidence}</pre>
            {/if}
            {#if f.agent_reasoning_summary}
              <p class="pp-reason"><strong>Reasoning:</strong> {f.agent_reasoning_summary}</p>
            {/if}
            <div class="pp-artifacts">
              {#if f.linked_commit}<span class="chip pp-artifact">commit {f.linked_commit.slice(0, 9)}</span>{/if}
              {#if f.linked_test}<span class="chip pp-artifact">test {f.linked_test}</span>{/if}
              {#if f.jira_key}<span class="chip pp-artifact">{f.jira_key}</span>{/if}
            </div>
            {#if entry.events.length > 0}
              <ul class="pp-timeline">
                {#each entry.events as ev (ev.id)}
                  <li class="pp-event">
                    <span class="pp-event-kind">{ev.kind.replace(/_/g, ' ')}</span>
                    <span class="pp-event-meta dim">{ev.actor}{transitionLabel(ev)}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {/each}
        {#if pack.findings.length === 0}
          <p class="dim" style="font-size:12.5px">No findings in this review.</p>
        {/if}
      </div>

      <!-- Repo rules generated from this review -->
      {#if pack.repo_rules.length > 0}
        <h3 class="pp-section">Repo rules ({pack.repo_rules.length})</h3>
        <ul class="pp-rules">
          {#each pack.repo_rules as r (r.id)}
            <li class="pp-rule"><strong>{r.title}</strong> — {r.body}</li>
          {/each}
        </ul>
      {/if}

      <!-- Exported markdown (after Export) -->
      {#if exportedMd}
        <h3 class="pp-section">Exported snapshot</h3>
        <pre class="pp-export">{exportedMd}</pre>
      {/if}
    {/if}
  {/snippet}
  {#snippet footer()}
    {#if exportedMd}
      <button class="btn ghost" onclick={() => void copyMarkdown()}>Copy markdown</button>
    {/if}
    <button class="btn primary" disabled={exporting || loading} onclick={() => void doExport()}>
      {exporting ? 'Exporting…' : 'Export'}
    </button>
    <button class="btn ghost" onclick={onclose}>Close</button>
  {/snippet}
</Modal>

<style>
  .pp-summary {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    padding: 4px 0 10px;
    font-size: 12.5px;
  }
  .pp-stat strong { font-size: 14px; }
  .pp-ok { color: #7ee787; }
  .pp-breakdown {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin-bottom: 12px;
  }
  .pp-chip { font-size: 10.5px; }
  .pp-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pp-finding { padding: 10px 12px; }
  .pp-finding-head {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: 4px;
  }
  .pp-title { font-size: 12.5px; font-weight: 600; }
  .pp-loc { font-size: 11px; color: var(--text-dim); margin-bottom: 6px; }
  .pp-evidence {
    margin: 4px 0;
    padding: 6px 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s, 4px);
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-x: auto;
    max-height: 180px;
  }
  .pp-reason { margin: 4px 0; font-size: 12px; line-height: 1.5; }
  .pp-artifacts { display: flex; flex-wrap: wrap; gap: 5px; margin: 4px 0; }
  .pp-artifact {
    font-size: 10.5px;
    background: color-mix(in srgb, #7ee787 16%, transparent);
    color: var(--text);
  }
  .pp-timeline { list-style: none; margin: 6px 0 0; padding: 0; }
  .pp-event { font-size: 11px; line-height: 1.5; display: flex; gap: 6px; flex-wrap: wrap; }
  .pp-event-kind { font-weight: 600; }
  .pp-section { font-size: 12.5px; font-weight: 600; margin: 14px 0 6px; }
  .pp-rules { margin: 0; padding-inline-start: 18px; }
  .pp-rule { font-size: 12px; line-height: 1.5; margin-bottom: 3px; }
  .pp-export {
    margin: 0;
    padding: 8px 10px;
    background: var(--surface-2);
    border-radius: var(--radius-s, 4px);
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-x: auto;
    max-height: 220px;
  }
  .pp-err { color: var(--status-exited); font-size: 12.5px; }
  .dim { color: var(--text-dim); }
  .mono { font-family: var(--font-mono, monospace); }

  /* Status chips (shared vocabulary; high-contrast for verified). */
  .status-open { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }
  .status-accepted { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .status-fixed { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); }
  .status-verified { background: #7ee787; color: #000; }
  .status-false_positive { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }
  .status-waived { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }

  /* Severity chips (red for blocker severities). */
  .sev2-critical { background: var(--status-exited); color: #fff; }
  .sev2-high { background: color-mix(in srgb, var(--status-exited) 18%, transparent); color: var(--status-exited); }
  .sev2-medium { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); }
  .sev2-low { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent); }
  .sev2-info { background: color-mix(in srgb, var(--text-dim) 16%, transparent); color: var(--text-dim); }
</style>
