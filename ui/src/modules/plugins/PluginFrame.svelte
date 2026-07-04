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
      },
      origin,
    );
  }

  function onMessage(ev: MessageEvent) {
    if (ev.source !== frame?.contentWindow || ev.origin !== origin) return;
    const m = ev.data;
    if (!m || m.type !== 'otto:keydown' || typeof m.key !== 'string') return;
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: m.key,
        code: typeof m.code === 'string' ? m.code : undefined,
        metaKey: !!m.metaKey,
        ctrlKey: !!m.ctrlKey,
        altKey: !!m.altKey,
        shiftKey: !!m.shiftKey,
        bubbles: true,
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
