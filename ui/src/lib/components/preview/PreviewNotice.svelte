<script lang="ts">
  // Inline preview failure (invalid JSON, a Mermaid syntax error, an image that
  // won't decode): the message — and the source position when known — in the
  // pane itself, never a toast; the source stays one click away. Spoken once
  // when it appears (announceShown), not on every keystroke while the author
  // fixes it, so it carries no live-region role.
  import Icon from '../Icon.svelte';
  import { announceShown } from '../../announce.svelte';

  interface Props {
    title: string;
    message: string;
    line?: number;
    col?: number;
  }
  let { title, message, line, col }: Props = $props();
</script>

<div class="fp-err" use:announceShown={title} data-testid="file-preview-error">
  <div class="fp-err-head">
    <Icon name="warning" size={14} />
    <span>{title}</span>
    {#if line !== undefined}
      <span class="fp-err-pos">Line {line}{col !== undefined ? `, column ${col}` : ''}</span>
    {/if}
  </div>
  <pre class="fp-err-msg" dir="ltr">{message}</pre>
</div>

<style>
  .fp-err {
    margin: 12px;
    padding: 10px 12px;
    border: 1px solid var(--danger);
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    color: var(--text);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .fp-err-head {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--danger);
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .fp-err-pos {
    font-weight: 400;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .fp-err-msg {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
</style>
