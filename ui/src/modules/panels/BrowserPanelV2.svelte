<script lang="ts">
  // Right-panel Browser tab, "v2": the Browser module (reader/live tabs,
  // persisted marks, ask bar) embedded beside the active agent session. The
  // ask bar targets that session, so marking an element here and asking
  // about it lands in the pane the user is already working in. Shares the
  // `browser` store (tabs/marks) with the Browser page — they are never
  // mounted at the same time (the panel only exists on the Agents module),
  // so the two never fight over a live tab's native webview.
  import BrowserView from '../browser/BrowserView.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';

  const target = $derived(ws.activeSession?.kind === 'agent' ? ws.activeSessionId : null);
</script>

<div class="v2">
  <BrowserView embedded targetSessionId={target} />
</div>

<style>
  .v2 {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .v2 > :global(.browser) {
    flex: 1;
    min-height: 0;
  }
</style>
