<script lang="ts">
  // The dim intro paragraph at the top of a Settings section body. It holds
  // the rules that don't fit a PageHeader subtitle: subtitles are one short
  // line, ellipsized when space runs out and hidden on phone, so anything a
  // user must know (where secrets live, what applies when, who can do what)
  // belongs here instead.
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
</script>

<p class="settings-intro">{@render children()}</p>

<style>
  .settings-intro {
    margin: 0 0 14px;
    max-width: var(--settings-col);
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  /* An inline cross-link to a related page (a real <button>, styled as a link). */
  .settings-intro :global(.intro-link) {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }
  .settings-intro :global(.intro-link:focus-visible) {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
    border-radius: var(--radius-s);
  }
  .settings-intro :global(code) {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
</style>
