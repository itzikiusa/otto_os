<script lang="ts">
  // Inline preview failure: the message (and source position when known) in
  // the pane itself — never a toast — plus Retry when re-running can help.
  import Icon from '../../../lib/components/Icon.svelte';

  interface Props {
    title: string;
    message: string;
    line?: number;
    col?: number;
    onretry?: () => void;
  }
  let { title, message, line, col, onretry }: Props = $props();
</script>

<div class="perr" role="alert" data-testid="wb-preview-error">
  <div class="head">
    <Icon name="warning" size={14} />
    <span class="title">{title}</span>
    {#if line !== undefined}
      <span class="pos">Line {line}{col !== undefined ? `, column ${col}` : ''}</span>
    {/if}
  </div>
  <pre class="msg">{message}</pre>
  {#if onretry}
    <button type="button" class="btn small" onclick={onretry}>Retry</button>
  {/if}
</div>

<style>
  .perr {
    margin: 12px;
    padding: 10px 12px;
    border: 1px solid var(--danger);
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    color: var(--text);
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--danger);
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .pos {
    font-weight: 400;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .msg {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
</style>
