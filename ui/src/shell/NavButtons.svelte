<script lang="ts">
  // On-screen Back / Forward buttons wired to the hash router's history stack.
  // Rendered in the mobile top bar (App.svelte) so touch users can navigate
  // without the browser chrome. On desktop the router already exposes ⌘⇧←/→
  // keyboard shortcuts (keys.ts → App.svelte navBack/navForward actions), so
  // these buttons are optional there; they are included but can be placed
  // anywhere in the shell layout.
  import Icon from '../lib/components/Icon.svelte';
  import { router } from '../lib/router.svelte';
</script>

<div class="nav-btns" role="group" aria-label="Navigation history">
  <button
    class="nav-btn"
    onclick={() => router.back()}
    disabled={!router.canBack}
    aria-label="Go back"
    title="Back (⌘⇧←)"
  >
    <Icon name="chevronLeft" size={16} />
  </button>
  <button
    class="nav-btn"
    onclick={() => router.forward()}
    disabled={!router.canForward}
    aria-label="Go forward"
    title="Forward (⌘⇧→)"
  >
    <Icon name="chevronRight" size={16} />
  </button>
</div>

<style>
  .nav-btns {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }
  .nav-btn {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
    transition: background 100ms ease-out, color 100ms ease-out;
  }
  .nav-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  .nav-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
</style>
