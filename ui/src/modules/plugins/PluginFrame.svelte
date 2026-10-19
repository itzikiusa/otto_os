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
  // inline "Couldn’t load" + Retry instead of an empty/raw daemon page in the
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
  /** Plugin slugs are plain identifiers. Anything else (`..`, `/`, `%2e`) is
   *  never interpolated into a URL: `/plugins/../ui/` resolves to Otto's own
   *  SPA (served 200), which would then load inside the frame and receive
   *  `otto:init` with the viewer's token. */
  const slugOk = $derived(/^[a-z][a-z0-9-]{0,63}$/.test(slug)); // = plugins.rs `valid_slug`
  const src = $derived(`${origin}/plugins/${encodeURIComponent(slug)}/ui/`);
  let frame = $state<HTMLIFrameElement | undefined>();

  // 'missing' = the daemon has no UI for this slug (disabled, uninstalled or
  // UI-less) — not a failure to retry, so it gets an empty state with a way
  // to the plugin settings instead of "Couldn’t load" + Retry.
  let probe = $state<'loading' | 'ok' | 'error' | 'missing'>('loading');
  let probeError = $state<string | null>(null);
  let frameLoaded = $state(false);
  let probeSeq = 0;

  async function check(): Promise<void> {
    const seq = ++probeSeq;
    const url = src;
    probe = 'loading';
    frameLoaded = false;
    if (!slugOk) {
      probeError = null;
      probe = 'missing';
      return;
    }
    try {
      // HEAD (V8): the probe only needs the status; a GET downloaded the whole
      // entry once here and again when the iframe loaded it. Axum answers HEAD
      // on GET routes; anything that refuses it (405) falls back to a GET.
      let r = await fetch(url, { method: 'HEAD', cache: 'no-store' });
      if (r.status === 405 || r.status === 501) r = await fetch(url, { cache: 'no-store' });
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
    const out: Record<string, string> = {};
    for (const k of 'bg surface surface-2 surface-3 hover text text-dim border accent accent-solid accent-contrast accent-text accent-soft danger danger-soft warning warning-soft success success-soft info info-soft radius-s radius-m font-ui font-mono fs-xs fs-s fs-m fs-l'.split(' ')) {
      const v = `--${k}`;
      out[v] = cs.getPropertyValue(v).trim();
    }
    return out;
  }

  function relayKey(init: KeyboardEventInit) {
    window.dispatchEvent(new KeyboardEvent('keydown', { ...init, bubbles: true, cancelable: true }));
  }

  function postTheme(type: 'otto:init' | 'otto:theme', extra?: Record<string, unknown>) {
    const dir = ui.direction || document.documentElement.dir || 'ltr';
    try {
      if (frame?.contentDocument?.documentElement) frame.contentDocument.documentElement.dir = dir;
    } catch { /* cross-origin */ }
    frame?.contentWindow?.postMessage(
      {
        type,
        theme: themeVars(),
        themeName: ui.theme,
        scheme: ui.resolvedScheme,
        dir,
        reducedMotion: typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches,
        ...extra,
      },
      origin,
    );
  }

  function onload() {
    frameLoaded = true;
    if (!slugOk) return;
    try {
      frame?.contentWindow?.addEventListener('keydown', (e: KeyboardEvent) => {
        if (e.key === 'Escape' && !e.defaultPrevented) relayKey({ key: 'Escape' });
      });
    } catch { /* cross-origin */ }
    postTheme('otto:init', {
      slug,
      apiBase: `${baseUrl()}/api/v1/plugins/${encodeURIComponent(slug)}`,
      token: getToken(),
      providers: agentProviders(),
    });
  }

  $effect(() => {
    void ui.resolvedScheme;
    void ui.theme;
    void ui.direction;
    void ui.accent;
    if (!frameLoaded) return;
    untrack(() => postTheme('otto:theme'));
  });

  function onMessage(ev: MessageEvent) {
    const m = ev.data;
    if (!m || !frame || ev.source == null || ev.source !== frame.contentWindow) return;
    if (m.type === 'otto:focus-leave') {
      document.querySelector<HTMLElement>('.plugin-page .icon-btn')?.focus();
    } else if (m.type === 'otto:keydown' && typeof m.key === 'string') {
      relayKey(m);
    }
  }
</script>

<svelte:window onmessage={onMessage} />

<!-- Same chrome as every built-in module: the plugin's name in the shared
     header bar, its own UI below. -->
<div class="plugin-page">
  <PageHeader title={name}>
    {#snippet actions()}
      {#if probe === 'ok'}
        <!-- A plugin page can wedge or go stale like any web page: restart it
             in place (re-probes, reloads the frame, re-sends otto:init). -->
        <button class="icon-btn" data-icon="refresh" onclick={() => void check()} aria-label="Restart {name}" title="Restart {name}">
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
