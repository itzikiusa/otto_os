<script lang="ts" module>
  import { SvelteSet } from 'svelte/reactivity';
  /** Files whose preview may run their own scripts — this window session only. */
  const scriptsOn = new SvelteSet<string>();
</script>

<script lang="ts">
  // An HTML file rendered in a sandboxed `srcdoc` frame, the Workbench preview's
  // policy (lib/render/previewText.ts: a strict CSP meta in front of the page):
  //   • never `allow-same-origin` — the page is an opaque origin: it cannot read
  //     Otto's DOM, storage or API token, and never navigates the app;
  //   • scripts OFF by default; "Enable for this file" adds `allow-scripts` only
  //     (inline scripts; the CSP still blocks every network fetch);
  //   • relative files (images, CSS, JS next to the page) are not loaded — the
  //     bar says so rather than showing a silently broken page.
  // The frame keeps a white page (`color-scheme: light`): an HTML file's own
  // colours are its author's, whatever Otto's theme.
  import { buildHtmlPreviewDoc } from '../../render/previewText';
  import { htmlSandboxFor } from '../filePreviewKind';

  interface Props {
    path: string;
    content: string;
    name: string;
  }
  let { path, content, name }: Props = $props();

  const scripts = $derived(scriptsOn.has(path));
  const doc = $derived(buildHtmlPreviewDoc(content, { scripts }));

  function toggleScripts(): void {
    if (scriptsOn.has(path)) scriptsOn.delete(path);
    else scriptsOn.add(path);
  }
</script>

<div class="fp-html">
  <div class="fp-html-bar" data-testid="file-preview-html-bar">
    <span class="fp-html-state" data-testid="file-preview-scripts-state">
      {scripts ? 'Scripts on · sandboxed, no access to Otto' : 'Scripts off'}
    </span>
    <button
      type="button"
      class="btn small ghost"
      onclick={toggleScripts}
      title={scripts ? 'Stop running this page’s scripts' : 'Run this page’s inline scripts in the sandbox (no network, no access to Otto)'}
      data-testid="file-preview-scripts-toggle"
    >
      {scripts ? 'Turn off' : 'Enable for this file'}
    </button>
    <span class="fp-html-note">Relative files (images, styles, scripts) aren’t loaded.</span>
  </div>
  <!-- A new frame per policy: a changed `sandbox` only applies on the next load. -->
  {#key scripts}
    <iframe
      class="fp-html-frame"
      sandbox={htmlSandboxFor(scripts)}
      referrerpolicy="no-referrer"
      title={`Preview of ${name}`}
      srcdoc={doc}
      data-testid="file-preview-html-frame"
    ></iframe>
  {/key}
</div>

<style>
  .fp-html {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
  }
  .fp-html-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 8px;
    padding: 4px 10px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .fp-html-state {
    color: var(--text);
    font-weight: 500;
  }
  .fp-html-note {
    margin-inline-start: auto;
  }
  .fp-html-frame {
    display: block;
    flex: 1;
    inline-size: 100%;
    min-block-size: 240px;
    border: 0;
    /* The page's own colours: a browser-default white page in any Otto theme. */
    color-scheme: light;
    background: Canvas;
  }
</style>
