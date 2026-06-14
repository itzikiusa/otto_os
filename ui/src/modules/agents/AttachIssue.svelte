<script lang="ts">
  // Modal to search and attach a Jira issue to a session.
  import type { IssueSummary } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import JiraIssuePicker from './JiraIssuePicker.svelte';

  interface Props {
    sessionId: string;
    onclose: () => void;
  }
  let { sessionId, onclose }: Props = $props();

  let attaching = $state(false);

  async function attach(issue: { account_id: string; key: string; summary: string }): Promise<void> {
    attaching = true;
    try {
      await ws.attachIssue(sessionId, {
        provider: 'jira',
        account_id: issue.account_id,
        key: issue.key,
        summary: issue.summary,
        url: '',
        status: '',
      });
      toasts.success('Issue attached', `${issue.key}: ${issue.summary}`);
      onclose();
    } catch (e) {
      toasts.error('Attach failed', e instanceof Error ? e.message : String(e));
    } finally {
      attaching = false;
    }
  }
</script>

<Modal title="Attach Jira Issue" width={520} {onclose}>
  <JiraIssuePicker onpick={(iss) => { void attach(iss); }} />

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
  {/snippet}
</Modal>
