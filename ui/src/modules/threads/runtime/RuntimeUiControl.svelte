<script lang="ts">
  // Agent UI control for a structured thread (docs/features/agent-ui-control.md):
  // an agent called an `otto.ui_*` tool without the grant, so ask here, under
  // the thread's header, exactly like a terminal session's pane does. Allow
  // lasts for the thread; Deny is remembered on this device. While on, a
  // quiet strip says the agent may drive Otto beside the thread (Stop = off).
  import ApprovalActions from '../../../lib/components/ApprovalActions.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { moduleLabel } from '../../../lib/sidebar';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { uiControl } from '../../../lib/stores/uiControl.svelte';
  import { commandLabel, providerName } from '../../../lib/uiCommands/frames';
  import { runtimeThreads } from './runtimeThreads.svelte';

  interface Props {
    sessionId: string;
  }
  let { sessionId }: Props = $props();

  const thread = $derived(runtimeThreads.get(sessionId));
  const canGrant = $derived(
    !!thread && thread.status !== 'closed' && !auth.isImpersonating && thread.created_by === auth.me?.id,
  );
  const prompt = $derived(canGrant ? uiControl.promptFor(sessionId) : null);
  const who = $derived(providerName(thread?.provider ?? ''));
  const what = $derived(
    prompt
      ? `“${commandLabel(prompt.command)}”${prompt.module && prompt.module !== 'shell' ? ` in ${moduleLabel(prompt.module)}` : ''}`
      : '',
  );
  let act = $state<'approve' | 'deny' | null>(null);
  async function run(kind: 'approve' | 'deny', fn: () => Promise<unknown>): Promise<void> {
    act = kind;
    try {
      await fn();
    } finally {
      act = null;
    }
  }
</script>

{#if prompt}
  <div class="rt-ui-ask" role="group" aria-label="UI control request" data-testid="runtime-ui-control-request">
    <span class="mark" aria-hidden="true"><Icon name="cursor" size={13} /></span>
    <span class="text" aria-live="polite">
      <strong>{who}{thread?.title ? ` · ${thread.title}` : ''} wants to drive Otto</strong> — {what}. Everything it does
      opens in the side pane beside Threads, and writes still ask you.
    </span>
    <span class="actions">
      <ApprovalActions
        approveLabel="Allow for this thread"
        approveBusyLabel="Allowing…"
        askReason={false}
        busy={act}
        disabled={uiControl.busy[sessionId] === true && act === null}
        onapprove={() => run('approve', () => uiControl.allow(sessionId))}
        ondeny={() => run('deny', () => uiControl.deny(sessionId))}
        testid="runtime-ui-control-actions"
      />
    </span>
  </div>
{/if}

<style>
  .rt-ui-ask {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 10px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--separator);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .mark {
    display: inline-flex;
    color: var(--warning);
    flex-shrink: 0;
  }
  .text {
    flex: 1 1 240px;
    min-width: 0;
    line-height: 1.4;
  }
  .actions {
    display: inline-flex;
    gap: 6px;
    flex-shrink: 0;
    margin-inline-start: auto;
  }
</style>
