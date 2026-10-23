<script lang="ts">
  // The run's human-approval gate above the canvas: the "resumed after a
  // restart" note, the publication preview a product gate must show before it
  // can be approved, the Approve / Deny banner, and the recorded outcome. The
  // state and the write live in runSync.svelte.ts.
  import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
  import ApprovalOutcome from '../../lib/components/ApprovalOutcome.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import type { WorkflowGraph } from '../../lib/api/types';
  import type { RunMonitor } from './runSync.svelte';

  interface Props {
    mon: RunMonitor;
    graph: WorkflowGraph;
    nodeName: (id: string) => string;
  }
  let { mon, graph, nodeName }: Props = $props();
  const run = $derived(mon.run);

  /** "“Title”" for a parent page picked by name in the publish node (PC-9 keeps
   *  its title beside the id), else "page {id}". */
  function parentPageLabel(parentId: string): string {
    const n = graph.nodes.find((x) => x.kind === 'product_publish' && (x.params as { parent_id?: unknown } | null)?.parent_id === parentId);
    const t = (n?.params as { parent_title?: unknown } | null)?.parent_title;
    return typeof t === 'string' && t ? `“${t}”` : `page ${parentId}`;
  }
  /** What a paused run's approval gates: the step(s) that fed it (whose work
   *  is being approved), the node's own prompt, and the step(s) an approval
   *  lets run next. */
  function approvalGate(nodeId: string): { after: string[]; ask: string; next: string[] } {
    const params = graph.nodes.find((n) => n.id === nodeId)?.params as { prompt?: unknown } | null | undefined;
    return {
      after: graph.edges.filter((e) => e.target === nodeId).map((e) => nodeName(e.source)),
      ask: typeof params?.prompt === 'string' ? params.prompt.trim() : '',
      next: graph.edges.filter((e) => e.source === nodeId).map((e) => nodeName(e.target)),
    };
  }
  /** "you" for the signed-in approver, a name as given, nothing for a bare id. */
  function approverLabel(by: string | null | undefined): string | null {
    if (!by) return null;
    if (by === auth.me?.id) return 'you';
    return /^[0-9A-HJKMNP-TV-Z]{26}$/i.test(by) ? null : by;
  }
</script>

<!-- Human-approval banner: shown when a run is paused at a human_approval node -->
{#if run && (run.resume_attempts ?? 0) > 0}
  <div class="resumed-banner" title="A daemon restart interrupted this run; it picked up from the step it was on">
    <Icon name="refresh" size={13} />
    <span>Resumed after a daemon restart (attempt {run.resume_attempts})</span>
  </div>
{/if}

{#if run?.waiting_approval && run.approval_node_id}
  {@const gate = approvalGate(run.approval_node_id)}
  {#if mon.approvalNeedsPreview}
    <section aria-label="Publication review">
      {#if mon.approvalPreviewLoading}<LoadState what="the content and destination for review" variant="compact" loading empty />
      {:else if mon.approvalPreviewError}
        <LoadState what="the publication preview" variant="compact" error={mon.approvalPreviewError} empty onretry={() => mon.retryApprovalPreview()} />
      {:else if mon.approvalPreview}
        {@const preview = mon.approvalPreview}
        <h3>{preview.kind === 'rfc' ? (preview.request.title ?? preview.request.reviewed_content.title) : preview.request.reviewed_content.title}</h3>
        <p>Publish via {preview.account_label} ({preview.account_url}).
          {#if preview.kind === 'jira'}Create {preview.request.issue_type} in project {preview.request.project_key}; visible to people with access to that project.
          {:else}Create a page in space {preview.request.space_key}{preview.request.parent_id ? ` under ${parentPageLabel(preview.request.parent_id)}` : ''}; visible to people with access to that space.{/if}
        </p>
        <textarea dir="auto" class="np-prompt" aria-label="Full publication content" rows="12" readonly value={mon.approvalBody}></textarea>
      {/if}
    </section>
  {/if}
  <div class="approval-banner" role="group" aria-label="Approval needed">
    <Icon name="userCheck" size={14} />
    <span>
      Run paused at <strong title={run.approval_node_id}>{nodeName(run.approval_node_id)}</strong>{#if gate.after.length > 0}, after <strong>{gate.after.join(', ')}</strong>{/if} — waiting for your approval
      {#if gate.ask}<span class="approval-ask">“{gate.ask}”</span>{/if}
      <span class="approval-sub">Run started <RelTime iso={run.started_at} /> · approving runs {gate.next.length > 0 ? gate.next.join(', ') : 'the rest of the run'}; denying stops the run with an error.</span>
    </span>
    <ApprovalActions
      disabled={mon.approvalNeedsPreview && (!mon.approvalPreviewReady || mon.approvalPreviewLoading || !!mon.approvalPreviewError)}
      disabledReason="Load and review the publication preview before approving."
      busy={mon.approving ? mon.approvingKind : null}
      denyTarget={`the run at ${nodeName(run.approval_node_id)}`}
      denyTitle="Deny approval"
      onapprove={() => mon.approveRun(true)}
      ondeny={(reason) => mon.approveRun(false, reason)}
    />
  </div>
{:else if run?.approved_at && !run.waiting_approval}
  <div class="approval-outcome">
    <ApprovalOutcome
      outcome={run.approved_by ? 'approved' : 'denied'}
      by={approverLabel(run.approved_by)}
      at={run.approved_at}
      note={run.approval_note && run.approval_note !== 'rejected' ? run.approval_note : null}
    />
  </div>
{/if}

<style>
  .resumed-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .approval-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--warning-soft);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-m);
    color: var(--text);
  }
  .approval-banner strong {
    font-weight: 600;
  }
  .approval-banner > span {
    flex: 1;
    min-width: 0;
  }
  .approval-ask {
    display: block;
    margin-block-start: 2px;
    font-style: italic;
    overflow-wrap: anywhere;
  }
  .approval-sub {
    display: block;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .approval-outcome {
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
  }
</style>
