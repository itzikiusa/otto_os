<script lang="ts">
  // Hosts a runtime plugin's UI in an iframe served by the daemon at
  // /plugins/<slug>/ui/. After load, we hand the iframe its API base + bearer
  // token + theme via postMessage (the plugin SDK listens for `otto:init`).
  //
  // Keyboard chords: an focused iframe swallows keydown, so global shell
  // shortcuts (⌘⇧←, ⌘K …) die inside plugin pages. Plugins forward
  // modifier-chords back as `otto:keydown` messages; we re-dispatch them as
  // synthetic window keydowns so the shell's shortcut handlers fire normally.
  import { baseUrl, getToken } from '../../lib/api/client';
  import { agentProviders } from '../../lib/providers';

  let { slug }: { slug: string } = $props();

  const origin = new URL(baseUrl()).origin;
  const src = $derived(`${origin}/plugins/${slug}/ui/`);
  let frame = $state<HTMLIFrameElement | undefined>();

  function themeVars(): Record<string, string> {
    const cs = getComputedStyle(document.documentElement);
    const pick = ['--bg', '--text', '--text-dim', '--accent', '--border'];
    const out: Record<string, string> = {};
    for (const v of pick) out[v] = cs.getPropertyValue(v).trim();
    return out;
  }

  function onload() {
    frame?.contentWindow?.postMessage(
      {
        type: 'otto:init',
        slug,
        apiBase: `${baseUrl()}/api/v1/plugins/${slug}`,
        token: getToken(),
        theme: themeVars(),
        // The live agent-provider registry (built-ins + custom, e.g. grok) so a
        // plugin's provider pickers stay in sync with Otto — never hardcoded.
        providers: agentProviders(),
      },
      origin,
    );
  }

  function onMessage(ev: MessageEvent) {
    const m = ev.data;
    if (!m || m.type !== 'otto:keydown' || typeof m.key !== 'string') return;
    // Only accept from OUR plugin frame. Some webviews (Tauri/WKWebView)
    // deliver iframe messages with `source === null`, so we can't require a
    // strict source match — fall back to the same-origin check when it is.
    const bySource = ev.source != null && ev.source === frame?.contentWindow;
    if (!bySource && ev.origin !== origin) return;
    // Re-dispatch as a real keydown so the shell's global key map (keys.ts,
    // capture-phase window listener) handles it exactly as if the app itself
    // were focused — an external plugin inherits every app shortcut.
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: m.key,
        code: typeof m.code === 'string' ? m.code : undefined,
        keyCode: typeof m.keyCode === 'number' ? m.keyCode : 0,
        metaKey: !!m.metaKey,
        ctrlKey: !!m.ctrlKey,
        altKey: !!m.altKey,
        shiftKey: !!m.shiftKey,
        bubbles: true,
        cancelable: true,
      }),
    );
  }
</script>

<svelte:window onmessage={onMessage} />

<iframe
  bind:this={frame}
  title={slug}
  {src}
  onload={onload}
  allow="clipboard-write"
></iframe>

<style>
  iframe {
    width: 100%;
    height: 100%;
    border: 0;
    display: block;
    background: var(--bg);
  }
</style>
