<script lang="ts">
  // The per-finding action bar for the Findings workflow board: the two headline
  // actions (ask an agent to fix / verify resolved) inline, and everything else
  // (jira / false-positive / require-approval / repo-rule / regression-test,
  // then accept / waive / approve / reject) in one ⋯ menu — seven equal buttons
  // on every card was a wall nobody could scan.
  // Each button is disabled per the legal status transitions (mirrors
  // FindingStatus::can_transition in crates/otto-core/src/finding.rs); on click it
  // calls the client method and reports the updated finding back to the board (the
  // finding_updated WS event also drives a refetch).
  import { ApiError } from '../../lib/api/client';
  import {
    acceptFinding,
    waiveFinding,
    falsePositiveFinding,
    requireApprovalFinding,
    approveFinding,
    findingToJira,
    findingToRepoRule,
    fixFinding,
    verifyFinding,
    regressionTestFinding,
  } from '../../lib/api/client';
  import type { Finding } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmOutward } from '../../lib/confirmOutward';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    finding: Finding;
    /** Patch the board's copy with the server's updated finding. */
    onupdated: (f: Finding) => void;
  }
  let { finding, onupdated }: Props = $props();

  // Which action is in flight (disables the whole bar). Empty = idle.
  let busy = $state('');
  // Inline "Convert to Jira" project-key input.
  let jiraOpen = $state(false);
  let jiraKey = $state('');

  // --- legal-transition gates (mirror the Rust transition table) -------------
  const canAccept = $derived(finding.status === 'open');
  const canFix = $derived(finding.status === 'open' || finding.status === 'accepted');
  const canVerify = $derived(
    finding.status === 'accepted' || finding.status === 'fixed' || finding.status === 'verified',
  );
  const canFalsePositive = $derived(
    ['open', 'accepted', 'fixed', 'verified'].includes(finding.status),
  );
  const canWaive = $derived(['open', 'accepted', 'fixed'].includes(finding.status));
  const canRequireApproval = $derived(!finding.requires_human_approval);
  const canApprove = $derived(finding.requires_human_approval && !finding.approved_at);
  const canJira = $derived(!finding.jira_key);
  const canRepoRule = $derived(!finding.repo_rule_id);
  const canRegressionTest = $derived(!finding.linked_test);

  /** Run an action that returns the updated finding; report it back + toast. */
  async function run(
    label: string,
    fn: () => Promise<Finding>,
    ok: string,
  ): Promise<void> {
    if (busy) return;
    busy = label;
    try {
      const f = await fn();
      onupdated(f);
      toasts.success(ok);
    } catch (e) {
      const msg = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
      toasts.error(`Could not ${label}`, msg);
    } finally {
      busy = '';
    }
  }

  // Agent-backed actions return { finding, session_id? } — unwrap the finding.
  async function runAgent(
    label: string,
    fn: () => Promise<{ finding: Finding; session_id?: string | null }>,
    ok: string,
  ): Promise<void> {
    if (busy) return;
    busy = label;
    try {
      const resp = await fn();
      onupdated(resp.finding);
      toasts.success(ok, resp.session_id ? 'Agent session started — watch it in Agents.' : undefined);
    } catch (e) {
      const msg = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
      toasts.error(`Could not ${label}`, msg);
    } finally {
      busy = '';
    }
  }

  function doFix(): void {
    void runAgent('ask agent to fix', () => fixFinding(finding.id), 'Fix agent started');
  }
  function doVerify(): void {
    // Verify is judged synchronously and is evidence-based: say plainly when
    // it did NOT verify (and why) instead of a generic "started" toast.
    if (busy) return;
    busy = 'verify';
    verifyFinding(finding.id)
      .then((resp) => {
        onupdated(resp.finding);
        if (resp.finding.status === 'verified') {
          toasts.success('Verified', resp.note ?? undefined);
        } else {
          toasts.warn('Not verified', resp.note ?? undefined);
        }
      })
      .catch((e: unknown) => {
        const msg = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
        toasts.error('Could not verify', msg);
      })
      .finally(() => {
        busy = '';
      });
  }
  function doRegressionTest(): void {
    void runAgent(
      'add regression test',
      () => regressionTestFinding(finding.id),
      'Regression-test agent started',
    );
  }
  function doFalsePositive(): void {
    void run('mark false positive', () => falsePositiveFinding(finding.id), 'Marked false positive');
  }
  function doRequireApproval(): void {
    void run('require approval', () => requireApprovalFinding(finding.id), 'Approval required');
  }
  function doRepoRule(): void {
    // The repo-rule endpoint returns the RepoRule, not the Finding; refetch is
    // driven by the WS finding_updated. We just toast + note success here.
    if (busy) return;
    busy = 'add to repo rule';
    findingToRepoRule(finding.id, {})
      .then(() => {
        // Reflect the link locally so the button disables until the WS refetch.
        onupdated({ ...finding, repo_rule_id: 'pending' });
        toasts.success('Added to repo rules', 'It will be injected into future agent sessions.');
      })
      .catch((e: unknown) => {
        const msg = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
        toasts.error('Could not add to repo rule', msg);
      })
      .finally(() => {
        busy = '';
      });
  }

  // --- Convert to Jira (inline project-key input) ----------------------------
  function openJira(): void {
    jiraOpen = true;
    jiraKey = '';
  }
  function cancelJira(): void {
    jiraOpen = false;
    jiraKey = '';
  }
  async function submitJira(): Promise<void> {
    const key = jiraKey.trim().toUpperCase();
    if (!key) {
      toasts.warn('Enter a Jira project key');
      return;
    }
    if (busy) return;
    // Creating an issue is outward-facing: say where it lands and who sees it.
    const ok = await confirmOutward({
      verb: 'Create Jira issue',
      where: `Jira project ${key}`,
      what: finding.title || finding.body.split('\n')[0],
      who: 'Everyone with access to the project; watchers may be notified.',
    });
    if (!ok) return;
    busy = 'convert to Jira';
    try {
      const f = await findingToJira(finding.id, { project_key: key });
      onupdated(f);
      toasts.success('Jira issue created', f.jira_key ?? undefined);
      jiraOpen = false;
      jiraKey = '';
    } catch (e) {
      // 400 {code:'invalid'} when no Jira account is configured — show its message.
      const msg = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
      toasts.error('Could not convert to Jira', msg);
    } finally {
      busy = '';
    }
  }

  // --- Overflow menu (accept / waive / approve / reject) ---------------------
  function openOverflow(e: MouseEvent): void {
    const items: MenuItem[] = [
      {
        label: finding.jira_key ? `In Jira as ${finding.jira_key}` : 'Convert to Jira issue…',
        icon: 'ticket',
        disabled: !canJira || !!busy,
        action: openJira,
      },
      {
        label: finding.linked_test ? 'Regression test added' : 'Add regression test',
        icon: 'check',
        disabled: !canRegressionTest || !!busy,
        action: doRegressionTest,
      },
      {
        label: finding.repo_rule_id ? 'In repo rules' : 'Add to repo rules',
        icon: 'book',
        disabled: !canRepoRule || !!busy,
        action: doRepoRule,
      },
      {
        label: finding.requires_human_approval ? 'Approval required' : 'Require human approval',
        icon: 'userCheck',
        disabled: !canRequireApproval || !!busy,
        action: doRequireApproval,
      },
      {
        label: 'Mark false positive',
        icon: 'eyeOff',
        disabled: !canFalsePositive || !!busy,
        action: doFalsePositive,
      },
      { separator: true },
    ];
    items.push({
      label: 'Accept',
      disabled: !canAccept || !!busy,
      action: () => void run('accept', () => acceptFinding(finding.id), 'Accepted'),
    });
    items.push({
      label: 'Waive',
      disabled: !canWaive || !!busy,
      action: () => void run('waive', () => waiveFinding(finding.id), 'Waived'),
    });
    if (finding.requires_human_approval && !finding.approved_at) {
      items.push({
        label: 'Approve',
        disabled: !!busy,
        action: () =>
          void run('approve', () => approveFinding(finding.id, 'approve'), 'Approved'),
      });
      items.push({
        label: 'Reject',
        disabled: !!busy,
        action: () =>
          void run('reject', () => approveFinding(finding.id, 'reject'), 'Rejected'),
      });
    }
    ctxMenu.show(e, items);
  }
</script>

<div class="fa">
  <button
    class="btn small"
    disabled={!canFix || !!busy}
    onclick={doFix}
    title={canFix ? 'Start an agent session that fixes this finding' : 'Only open or accepted findings can be sent for a fix'}
  >
    <Icon name="zap" size={12} />
    {busy === 'ask agent to fix' ? 'Starting…' : 'Ask agent to fix'}
  </button>
  <button
    class="btn small"
    disabled={!canVerify || !!busy}
    onclick={doVerify}
    title={canVerify ? 'Check the evidence that this finding is resolved' : 'Accept or fix the finding first'}
  >
    <Icon name="check" size={12} />
    {busy === 'verify' ? 'Verifying…' : 'Verify resolved'}
  </button>
  {#if busy && busy !== 'ask agent to fix' && busy !== 'verify'}
    <span class="fa-busy dim" role="status">Working…</span>
  {/if}
  <button
    class="icon-btn fa-overflow"
    onclick={openOverflow}
    aria-label="More actions for this finding"
    title="More actions for this finding"
  ><Icon name="more" size={14} /></button>
</div>

{#if jiraOpen}
  <div class="fa-jira">
    <input
      class="fa-jira-input"
      placeholder="PROJ"
      aria-label="Jira project key"
      bind:value={jiraKey}
      onkeydown={(e) => {
        if (e.key === 'Enter') void submitJira();
        if (e.key === 'Escape') cancelJira();
      }}
    />
    <button class="btn small ghost" disabled={!!busy} onclick={cancelJira}>Cancel</button>
    <button class="btn small primary" disabled={!!busy || !jiraKey.trim()} onclick={() => void submitJira()}>
      {busy === 'convert to Jira' ? 'Creating…' : 'Create issue…'}
    </button>
  </div>
{/if}

<style>
  .fa {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin-top: 8px;
  }
  .fa {
    align-items: center;
  }
  .fa-busy {
    font-size: var(--fs-xs);
  }
  .fa-jira {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    flex-wrap: wrap;
  }
  .fa-jira-input {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 4px 8px;
    min-width: 200px;
    flex: 1 1 200px;
    max-width: 280px;
  }
  /* Touch targets on phone/tablet. */
  @media (max-width: 1024px) {
    .fa .btn.small,
    .fa .icon-btn { min-height: 36px; min-width: 36px; }
  }
</style>
