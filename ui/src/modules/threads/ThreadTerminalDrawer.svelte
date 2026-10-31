<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';

  interface Props {
    sessionId: string;
    title: string;
    onclose: () => void;
  }
  let { sessionId, title, onclose }: Props = $props();

  let tall = $state(false);
</script>

<section
  class="tt-drawer otto-force-dark"
  class:tall
  aria-label="Terminal for {title}"
  data-testid="thread-terminal-drawer-{sessionId}"
>
  <header class="tt-bar">
    <div class="tt-title">
      <Icon name="terminal" size={12} />
      <span>Raw PTY Terminal</span>
      <span class="tt-sub ellipsis" dir="auto">{title}</span>
    </div>
    <div class="tt-actions">
      <button
        type="button"
        class="icon-btn"
        aria-label={tall ? 'Restore terminal height' : 'Expand terminal height'}
        title={tall ? 'Restore height' : 'Expand height'}
        onclick={() => (tall = !tall)}
      >
        <Icon name={tall ? 'minimize' : 'maximize'} size={12} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Hide terminal drawer"
        title="Hide terminal"
        onclick={onclose}
      >
        <Icon name="x" size={12} />
      </button>
    </div>
  </header>
  <div class="tt-body">
    <Terminal {sessionId} forceDark autoFocus resumeOnOpen />
  </div>
</section>

<style>
  .tt-drawer {
    display: flex;
    flex-direction: column;
    block-size: 220px;
    flex-shrink: 0;
    border-block-start: 1px solid var(--border);
    background: var(--term-bg);
  }
  .tt-drawer.tall {
    block-size: 360px;
  }
  .tt-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    block-size: 30px;
    padding-inline: 10px;
    background: var(--surface);
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .tt-title {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text);
  }
  .tt-sub {
    color: var(--text-dim);
    font-weight: 400;
  }
  .tt-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }
  .tt-body {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
  }
</style>
