<script lang="ts">
  // Hosts a runtime plugin's UI in an iframe served by the daemon at
  // /plugins/<slug>/ui/. After load, we hand the iframe its API base + bearer
  // token + theme via postMessage (the plugin SDK listens for `otto:init`),
  // then re-sends the theme as `otto:theme` whenever the appearance changes.
  //
  // Keyboard chords: an focused iframe swallows keydown, so global shell
  // shortcuts (⌘⇧←, ⌘K …) die inside plugin pages. Plugins forward
  // modifier-chords back as `otto:keydown` messages; we re-dispatch them as
  // synthetic window keydowns so the shell's shortcut handlers fire normally.
  //
  // Load states: the UI entry is probed before the iframe mounts, so a
  // disabled/uninstalled plugin (404) or an unreachable daemon renders an
  // inline "Couldn't load" + Retry instead of an empty/raw daemon page in the
  // frame; a skeleton covers the frame until it fires `load`.
  import { untrack } from 'svelte';
  import { baseUrl, getToken } from '../../lib/api/client';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { agentProviders } from '../../lib/providers';
  import { plugins } from '../../lib/stores/plugins.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { router } from '../../lib/router.svelte';
  import { ui } from '../../lib/stores/ui.svelte';

  let { slug }: { slug: string } = $props();

  const origin = new URL(baseUrl()).origin;
  /** Display name: the manifest name once the nav list has it, else the slug. */
  const name = $derived(plugins.get(slug)?.name ?? slug);
  const src = $derived(`${origin}/plugins/${slug}/ui/`);
  let frame = $state<HTMLIFrameElement | undefined>();

  // 'missing' = the daemon has no UI for this slug (disabled, uninstalled or
  // UI-less) — not a failure to retry, so it gets an empty state with a way
  // to the plugin settings instead of "Couldn't load" + Retry.
  let probe = $state<'loading' | 'ok' | 'error' | 'missing'>('loading');
  let probeError = $state<string | null>(null);
  let frameLoaded = $state(false);
  let probeSeq = 0;

  async function check(): Promise<void> {
    const seq = ++probeSeq;
    const url = src;
    probe = 'loading';
    frameLoaded = false;
    try {
      const r = await fetch(url, { cache: 'no-store' });
      if (seq !== probeSeq) return;
      if (r.ok) {
        probeError = null;
        probe = 'ok';
        return;
      }
      if (r.status === 404) {
        probeError = null;
        probe = 'missing';
        return;
      }
      probeError = `Otto answered ${r.status} for the plugin’s page. Try again, or check the plugin’s health in Settings → Plugins.`;
      probe = 'error';
    } catch (e) {
      if (seq !== probeSeq) return;
      probeError = loadErrorText(e);
      probe = 'error';
    }
  }

  $effect(() => {
    void src;
    untrack(() => void check());
  });

  function themeVars(): Record<string, string> {
    const cs = getComputedStyle(document.documentElement);
    // The semantic set a plugin needs to look native: surfaces, text, the
    // accent trio, status tones, border/radius and the two font stacks.
    const pick = [
      '--bg', '--surface', '--surface-2', '--text', '--text-dim', '--border',
      '--accent', '--accent-solid', '--accent-contrast', '--accent-text', '--accent-soft',
      '--danger', '--warning', '--success', '--info',
      '--radius-s', '--radius-m', '--font-ui', '--font-mono',
    ];
    const out: Record<string, string> = {};
    for (const v of pick) out[v] = cs.getPropertyValue(v).trim();
    return out;
  }

  function onload() {
    frameLoaded = true;
    frame?.contentWindow?.postMessage(
      {
        type: 'otto:init',
        slug,
        apiBase: `${baseUrl()}/api/v1/plugins/${slug}`,
        token: getToken(),
        theme: themeVars(),
        scheme: ui.resolvedScheme,
        // The live agent-provider registry (built-ins + custom, e.g. grok) so a
        // plugin's provider pickers stay in sync with Otto — never hardcoded.
        providers: agentProviders(),
      },
      origin,
    );
  }

  // Follow the app's appearance live: a light/dark, theme or accent change
  // after load re-sends the vars as `otto:theme` so an open plugin never stays
  // in the old scheme. (The attributes/vars are applied synchronously by the
  // ui store, so computed style is already current here.)
  $effect(() => {
    const scheme = ui.resolvedScheme;
    void ui.theme;
    void ui.accent;
    if (!frameLoaded) return;
    untrack(() => {
      frame?.contentWindow?.postMessage({ type: 'otto:theme', theme: themeVars(), scheme }, origin);
    });
  });

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

<!-- Same chrome as every built-in module: the plugin's name in the shared
     header bar, its own UI below. -->
<div class="plugin-page">
  <PageHeader title={name}>
    {#snippet actions()}
      {#if probe === 'ok'}
        <!-- A plugin page can wedge or go stale like any web page: reload it
             in place (re-probes, re-sends otto:init) without leaving. -->
        <button class="icon-btn" data-icon="refresh" onclick={() => void check()} aria-label="Reload {name}" title="Reload {name}">
          <Icon name="refresh" size={14} />
        </button>
      {/if}
      <!-- Not while the page is missing: its empty state already offers it. -->
      {#if auth.isRoot && probe !== 'missing'}
        <button class="icon-btn" data-icon="gear" data-overflow="-1" onclick={() => router.go('settings/plugins')} aria-label="Plugin settings" title="Plugin settings">
          <Icon name="gear" size={14} />
        </button>
      {/if}
    {/snippet}
  </PageHeader>
  {#if probe === 'ok'}
    <div class="pf-host">
      <iframe
        bind:this={frame}
        title={`${name} plugin`}
        data-plugin={slug}
        {src}
        onload={onload}
        allow="clipboard-write"
      ></iframe>
      {#if !frameLoaded}
        <div class="pf-loading"><LoadState what={name} variant="page" loading empty /></div>
      {/if}
    </div>
  {:else if probe === 'missing'}
    {#if auth.isRoot}
      <EmptyState
        variant="page"
        icon="box"
        title="{name} isn’t available"
        body="It’s disabled, uninstalled, or ships no page of its own. Enable or reinstall it in Settings → Plugins."
        actionLabel="Open plugin settings"
        actionIcon="gear"
        onaction={() => router.go('settings/plugins')}
      />
    {:else}
      <EmptyState
        variant="page"
        icon="box"
        title="{name} isn’t available"
        body="It’s disabled, uninstalled, or ships no page of its own. Ask an Otto admin to enable it."
      />
    {/if}
  {:else}
    <LoadState
      what={name}
      variant="page"
      loading={probe === 'loading'}
      error={probeError}
      empty
      onretry={() => void check()}
    />
  {/if}
</div>

<style>
  .plugin-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .pf-host {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }
  iframe {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: 0;
    display: block;
    background: var(--bg);
  }
  .pf-loading {
    position: absolute;
    inset: 0;
    background: var(--bg);
  }
</style>
