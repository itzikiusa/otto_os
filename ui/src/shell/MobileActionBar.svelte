<script lang="ts">
  // On-screen action bar for phone users who can't reach ⌘-key chords.
  // Each button invokes the SAME function the keyboard shortcut calls (passed in
  // as props from App.svelte). Rendered only on phone (viewport.isPhone gate in
  // App.svelte); desktop is completely unaffected.
  //
  // Actions exposed:
  //   palette    – ⌘K  open/close command palette (primary nav on mobile)
  //   newSession – ⌘T  open the New Session modal
  //   closeTab   – ⌘W  close the active tab/session
  //   find       – ⌘F  open find bar (terminal find if focused, else page find)
  //   broadcast  – ⌘⇧B broadcast a message to all sessions (shown when sessions exist)

  import Icon from '../lib/components/Icon.svelte';

  interface Props {
    onpalette: () => void;
    onnewSession: () => void;
    oncloseTab: () => void;
    onfind: () => void;
    /** Only rendered when the caller deems it relevant (e.g. sessions exist). */
    showBroadcast?: boolean;
    onbroadcast?: () => void;
  }

  let {
    onpalette,
    onnewSession,
    oncloseTab,
    onfind,
    showBroadcast = false,
    onbroadcast,
  }: Props = $props();
</script>

<div class="action-bar" role="toolbar" aria-label="Quick actions">
  <button
    class="ab-btn"
    onclick={onpalette}
    title="Command palette (⌘K)"
    aria-label="Open command palette"
  >
    <Icon name="command" size={18} />
    <span class="ab-label">Palette</span>
  </button>

  <button
    class="ab-btn"
    onclick={onnewSession}
    title="New session (⌘T)"
    aria-label="New session"
  >
    <Icon name="plus" size={18} />
    <span class="ab-label">New</span>
  </button>

  <button
    class="ab-btn"
    onclick={oncloseTab}
    title="Close session (⌘W)"
    aria-label="Close current session"
  >
    <Icon name="x" size={18} />
    <span class="ab-label">Close</span>
  </button>

  <button
    class="ab-btn"
    onclick={onfind}
    title="Find (⌘F)"
    aria-label="Find in terminal or page"
  >
    <Icon name="search" size={18} />
    <span class="ab-label">Find</span>
  </button>

  {#if showBroadcast && onbroadcast}
    <button
      class="ab-btn"
      onclick={onbroadcast}
      title="Broadcast (⌘⇧B)"
      aria-label="Broadcast message to all sessions"
    >
      <Icon name="send" size={18} />
      <span class="ab-label">Broadcast</span>
    </button>
  {/if}
</div>

<style>
  .action-bar {
    display: flex;
    align-items: center;
    /* Sits between the mobile top bar and the bottom nav — a compact horizontal
       strip. Fills the full width; buttons flex evenly. */
    height: 44px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    padding: 0 4px;
    gap: 2px;
  }

  .ab-btn {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
    /* iOS: ensure tap target is reachable and visually responds. */
    -webkit-tap-highlight-color: transparent;
    touch-action: manipulation;
    padding: 4px 2px;
    min-width: 44px; /* WCAG 2.5.5 minimum touch target */
  }

  .ab-btn:hover,
  .ab-btn:active {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }

  .ab-label {
    font-size: 9.5px;
    font-weight: 500;
    line-height: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
