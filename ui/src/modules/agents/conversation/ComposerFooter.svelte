<script lang="ts">
  // T3-Code-style calm metadata footer strip under the composer card:
  // harness/provider token, live status dot, model, git branch, CWD, and
  // unsent terminal input banner when present.
  import { plural } from '../../../lib/plural';
  import { activity } from '../../../lib/stores/activity.svelte';
  import type { sessionState } from '../../../lib/status';
  import StatusDot from '../../../lib/components/StatusDot.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import Icon from '../../../lib/components/Icon.svelte';

  type SessionStateResult = ReturnType<typeof sessionState>;

  interface Props {
    sessionId: string;
    agentName: string;
    provider?: string;
    st: SessionStateResult;
    cwd?: string;
    branch?: string | null;
    model?: string | null;
    termStatus?: string;
    termInput?: string;
    uploading?: number;
  }

  let {
    sessionId,
    agentName,
    provider = '',
    st,
    cwd = '',
    branch = null,
    model = null,
    termStatus = '',
    termInput = '',
    uploading = 0,
  }: Props = $props();

  const shortCwd = $derived(cwd.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~'));
  const pendingNudges = $derived(activity.tasks(sessionId).filter((t) => t.nudge_pending).length);
  const statusLabel = $derived(st.key === 'working' ? 'Working…' : st.label);
</script>

{#if termInput}
  <div
    class="term-input"
    data-term-input
    title="Typed in the terminal but not sent. Sending from here appends your text after it — the CLI submits both as one message."
  >
    <span class="dim">In terminal:</span> <span class="mono term-raw" dir="ltr">{termInput}</span>
  </div>
{/if}
<div class="status" data-status-line>
  <span class="meta-harness">
    {#if provider}<ProviderIcon {provider} size={12} />{/if}
    <span class="harness-name">{agentName}</span>
    <StatusDot state={st} />
    <span>{statusLabel}</span>
  </span>
  {#if model}<span class="meta-token mono model" dir="ltr" title="Model">{model}</span>{/if}
  {#if branch}
    <span class="meta-token mono branch" dir="ltr" title="Git branch">
      <Icon name="branch" size={12} /> {branch}
    </span>
  {/if}
  {#if shortCwd}
    <span class="meta-token mono cwd" dir="ltr" title={cwd}>
      <Icon name="folder" size={12} /> {shortCwd}
    </span>
  {/if}
  {#if uploading > 0}<span class="dim">· uploading {plural(uploading, 'image')}…</span>{/if}
  {#if pendingNudges > 0}
    <span class="dim" title="Board tasks waiting for the agent to go idle">· {plural(pendingNudges, 'board task')} pending</span>
  {/if}
  {#if termStatus}<span class="term-status" title="The agent’s own status line">{termStatus}</span>{/if}
</div>

<style>
  .status,
  .term-input {
    width: 100%;
    box-sizing: border-box;
    max-width: var(--chat-measure, none);
    margin-inline: auto;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 4px 4px 0;
    min-width: 0;
    flex-wrap: wrap;
    row-gap: 4px;
  }
  .meta-harness {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
  }
  .harness-name {
    font-weight: 500;
  }
  .meta-token {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 240px;
    unicode-bidi: isolate;
  }
  .branch {
    color: var(--accent-text);
    border-color: var(--accent-line);
    background: var(--accent-faint);
  }
  .term-status {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 0 1 auto;
    max-width: 48%;
    margin-inline-start: auto;
  }
  .term-raw {
    unicode-bidi: isolate;
  }
  .term-input {
    margin-block-start: 6px;
    font-size: var(--fs-xs);
    padding: 4px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    border: 1px dashed color-mix(in srgb, var(--warning) 50%, var(--border));
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  @container (max-width: 480px) {
    .cwd,
    .branch,
    .model {
      display: none;
    }
  }
</style>
