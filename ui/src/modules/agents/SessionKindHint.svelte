<script lang="ts">
  // The one quiet line under the New session sheet's title: what a terminal
  // session is, and that Threads (a chat view with child threads) exists —
  // with a way there. Agents and Threads stay two pages; this only points.
  import { router } from '../../lib/router.svelte';

  interface Props {
    /** Close the sheet (it may ask to discard what was typed); true when it closed. */
    onclose: () => boolean | Promise<boolean>;
  }
  let { onclose }: Props = $props();

  async function startThread(): Promise<void> {
    if (await onclose()) router.go('threads/new');
  }
</script>

<p class="kind-hint" data-testid="session-kind-hint">
  Runs the agent’s own CLI in a terminal. For a chat view with child threads:
  <button type="button" class="link" onclick={() => void startThread()}>Start a thread</button>
</p>

<style>
  .kind-hint {
    margin-block: 0 var(--sp-5);
    margin-inline: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    line-height: 1.4;
  }
  .link {
    border: 0;
    padding: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
</style>
