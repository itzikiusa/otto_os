<script lang="ts">
  // Settings → Browser: how live tabs render, and the daemon's Chromium
  // (api.md "Browser — remote live view"). Engine choices are daemon-wide and
  // Browser Admin; the renderer choice is per device (desktop app only).
  //
  // The Chromium download is never silent: choosing a build that isn't
  // installed only offers a Download button that says how big it is.
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { browserLive } from '../../lib/stores/browserLive.svelte';
  import { nativeBrowserAvailable } from '../../lib/nativeBrowser';
  import { formatBytes } from '../../lib/metric-format';
  import { toasts } from '../../lib/toast.svelte';
  import type { BrowserChromeBuild, BrowserLiveSettings } from '../../lib/api/types';

  $effect(() => {
    if (!browserLive.status && !browserLive.loading && browserLive.supported !== false) void browserLive.load();
  });

  const st = $derived(browserLive.status);
  const isAdmin = $derived(auth.can('browser', 'admin'));
  let saving = $state(false);
  let saveError = $state('');
  let installError = $state('');

  const BUILDS: { build: BrowserChromeBuild; name: string; blurb: string }[] = [
    { build: 'chrome', name: 'Chrome for Testing', blurb: 'The full browser in new headless mode. Best fidelity; the default.' },
    { build: 'chrome-headless-shell', name: 'Lighter engine', blurb: 'chrome-headless-shell. Smaller and faster to start; a few sites render differently.' },
  ];

  async function save(patch: Partial<BrowserLiveSettings>): Promise<void> {
    saving = true;
    saveError = '';
    try {
      await browserLive.updateSettings(patch);
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  async function install(build: BrowserChromeBuild): Promise<void> {
    installError = '';
    try {
      await browserLive.install(build);
      toasts.info('Download started', 'Otto is downloading the live browser engine. It keeps going if you leave this page.');
    } catch (e) {
      installError = e instanceof Error ? e.message : String(e);
    }
  }

  const job = $derived(st?.install ?? null);
  const running = $derived(browserLive.engineState === 'installing');
  const pct = $derived(
    job && job.total_bytes ? Math.min(100, Math.round((job.received_bytes / job.total_bytes) * 100)) : 0,
  );
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('browser')} subtitle="Live tabs and the browser engine that runs them" />
  <PageBody width="readable">
    <p class="section-intro">
      A live tab is a real browser. In the desktop app it can use this Mac's web view; everywhere
      else (a remote session, your phone, and whenever an agent drives a page) it runs in a
      Chromium the Otto daemon manages and streams to you.
    </p>

    {#if nativeBrowserAvailable}
      <section class="card" aria-labelledby="bs-renderer">
        <h2 id="bs-renderer" class="row-title">Live tabs on this device</h2>
        <div class="choices" role="radiogroup" aria-labelledby="bs-renderer">
          <label class="choice">
            <input
              type="radio"
              name="renderer"
              checked={browserLive.pref === 'native'}
              onchange={() => browserLive.setPref('native')}
            />
            <span>
              <span class="choice-title">This Mac's web view</span>
              <span class="row-desc">Fastest. Only in the desktop app; agents can't drive it.</span>
            </span>
          </label>
          <label class="choice">
            <input
              type="radio"
              name="renderer"
              checked={browserLive.pref === 'remote'}
              onchange={() => browserLive.setPref('remote')}
            />
            <span>
              <span class="choice-title">Otto's Chromium</span>
              <span class="row-desc">Streams to any device, uses Otto's network guard, and agents can drive it while you watch.</span>
            </span>
          </label>
        </div>
      </section>
    {/if}

    {#if browserLive.supported === false}
      <section class="card">
        <p class="row-desc">This Otto daemon doesn't include the live browser engine. Update Otto to use it.</p>
      </section>
    {:else if browserLive.loadError && !st}
      <section class="card" role="alert">
        <p class="error">Couldn't load the browser engine settings: {browserLive.loadError}</p>
        <button class="btn" onclick={() => void browserLive.load()}>Retry</button>
      </section>
    {:else if !st}
      <section class="card"><p class="row-desc" role="status">Loading browser engine settings…</p></section>
    {:else}
      <section class="card" aria-labelledby="bs-engine">
        <h2 id="bs-engine" class="row-title">Browser engine</h2>
        {#if !st.platform_supported}
          <p class="row-desc">The live browser engine runs on Apple silicon Macs only for now.</p>
        {:else}
          <p class="row-desc">
            Downloaded once into Otto's data folder and checked against a pinned checksum. It is never
            bundled or updated silently.
          </p>
          <div class="choices" role="radiogroup" aria-labelledby="bs-engine">
            {#each BUILDS as b (b.build)}
              {@const info = browserLive.build(b.build)}
              <div class="choice">
                <input
                  id={`bs-build-${b.build}`}
                  type="radio"
                  name="build"
                  checked={st.settings.build === b.build}
                  disabled={!isAdmin || saving || (b.build === 'chrome-headless-shell' && st.settings.headed)}
                  onchange={() => void save({ build: b.build })}
                />
                <label for={`bs-build-${b.build}`} class="choice-text">
                  <span class="choice-title">
                    {b.name}
                    {#if info?.installed}<span class="chip ok">Installed{info.version ? ` · ${info.version}` : ''}</span>{/if}
                  </span>
                  <span class="row-desc">{b.blurb} About {formatBytes(browserLive.downloadBytes(b.build))}.</span>
                  {#if info && !info.sha256_pinned}
                    <span class="row-desc">This Otto build has no checksum for it, so it can't be downloaded.</span>
                  {/if}
                </label>
                {#if isAdmin && info && !info.installed && info.sha256_pinned}
                  <button
                    class="btn small"
                    disabled={running || browserLive.starting}
                    onclick={() => void install(b.build)}
                  >
                    Download ({formatBytes(browserLive.downloadBytes(b.build))})
                  </button>
                {/if}
              </div>
            {/each}
          </div>
          {#if running && job}
            <div class="progress" role="status" aria-live="polite">
              <div class="bar" role="progressbar" aria-label="Engine download" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct}>
                <span style:width="{job.state === 'downloading' ? pct : 100}%"></span>
              </div>
              <span class="row-desc">
                {#if job.state === 'downloading'}
                  Downloading… {formatBytes(job.received_bytes)}{job.total_bytes ? ` of ${formatBytes(job.total_bytes)}` : ''}
                {:else if job.state === 'verifying'}
                  Checking the download…
                {:else}
                  Unpacking…
                {/if}
              </span>
            </div>
          {:else if job?.state === 'failed'}
            <p class="error" role="alert">The last download failed: {job.error || 'unknown error'}</p>
          {/if}
          {#if installError}<p class="error" role="alert">Couldn't start the download: {installError}</p>{/if}
        {/if}
      </section>

      <section class="card" aria-labelledby="bs-headed">
        <div class="row">
          <div class="row-text">
            <h2 id="bs-headed" class="row-title">Show the window on this Mac</h2>
            <span class="row-desc">
              Opens the live browser as a visible Chrome window on the Mac running Otto, as well as
              streaming it. Needs Chrome for Testing. Off by default.
            </span>
          </div>
          <div class="row-controls">
            <input
              type="checkbox"
              role="switch"
              aria-labelledby="bs-headed"
              aria-checked={st.settings.headed}
              checked={st.settings.headed}
              disabled={!isAdmin || saving || st.settings.build !== 'chrome'}
              title={st.settings.build !== 'chrome' ? 'Needs Chrome for Testing' : undefined}
              onchange={(e) => void save({ headed: (e.currentTarget as HTMLInputElement).checked })}
            />
          </div>
        </div>
      </section>

      <section class="card" aria-labelledby="bs-downloads">
        <div class="row">
          <div class="row-text">
            <h2 id="bs-downloads" class="row-title">Files pages download</h2>
            <span class="row-desc">Kept in a quarantine folder on the Mac running Otto and never opened, or refused.</span>
          </div>
          <div class="row-controls">
            <select
              class="input"
              aria-labelledby="bs-downloads"
              value={st.settings.downloads}
              disabled={!isAdmin || saving}
              onchange={(e) => void save({ downloads: (e.currentTarget as HTMLSelectElement).value as 'block' | 'quarantine' })}
            >
              <option value="quarantine">Keep in quarantine</option>
              <option value="block">Block</option>
            </select>
          </div>
        </div>
      </section>

      {#if !isAdmin}
        <p class="row-desc">Only a Browser admin can change the engine settings.</p>
      {/if}
      {#if saveError}<p class="error" role="alert">Couldn't save: {saveError}</p>{/if}
    {/if}
  </PageBody>
</div>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .section-intro {
    margin: 0 0 16px;
    font-size: var(--fs-m);
    line-height: 1.5;
    color: var(--text-dim);
  }
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 14px 16px;
    margin-bottom: 12px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
  }
  .row-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    min-width: 240px;
  }
  .row-title {
    margin: 0;
    font-weight: 600;
    font-size: var(--fs-m);
  }
  .row-desc {
    display: block;
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    line-height: 1.45;
  }
  .row-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 12px;
  }
  .choice {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .choice input {
    margin-top: 3px;
  }
  .choice-text {
    flex: 1;
    min-width: 0;
  }
  .choice-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 500;
  }
  .progress {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 12px;
  }
  .bar {
    height: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .error {
    margin: 8px 0 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
</style>
