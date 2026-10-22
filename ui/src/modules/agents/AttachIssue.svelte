<script lang="ts">
  // Modal to search and attach a tracker issue (Jira or Linear, per account) to a session.
  import type { IssueProviderKind } from '../../lib/api/types';
  import { toastError } from '../../lib/toastError';
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

  async function attach(issue: {
    account_id: string;
    provider: IssueProviderKind;
    key: string;
    summary: string;
    url: string;
    status: string;
  }): Promise<void> {
    // A second pick while the first is in flight attached the issue twice.
    if (attaching) return;
    attaching = true;
    try {
      // Store the tracker link + status the search returned, and the account's real provider,
      // so the Info panel's "Open in browser" works and a Linear issue isn't labelled Jira.
      await ws.attachIssue(sessionId, {
        provider: issue.provider,
        account_id: issue.account_id,
        key: issue.key,
        summary: issue.summary,
        url: issue.url,
        status: issue.status,
      });
      toasts.success('Issue attached', `${issue.key}: ${issue.summary}`);
      onclose();
    } catch (e) {
      toastError('Couldn’t attach the issue', e);
    } finally {
      attaching = false;
    }
  }
</script>

<Modal title="Attach issue" width={520} {onclose}>
  <JiraIssuePicker onpick={(iss) => { void attach(iss); }} />

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
  {/snippet}
</Modal>
