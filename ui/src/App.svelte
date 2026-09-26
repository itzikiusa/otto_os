<script lang="ts">
  // Boot flow: GET /meta → onboarding wizard | login | main shell.
  import Shell from './shell/App.svelte';
  import Onboarding from './modules/settings/Onboarding.svelte';
  import Login from './modules/settings/Login.svelte';
  import Toasts from './lib/components/Toasts.svelte';
  import SharePage from './modules/share/SharePage.svelte';
  import BarHost from './modules/desktop/BarHost.svelte';
  import TrayPage from './modules/desktop/TrayPage.svelte';
  import { auth } from './lib/stores/auth.svelte';
  import { router } from './lib/router.svelte';
  import { ui } from './lib/stores/ui.svelte';
  import { isEmbedded } from './lib/desktop';

  ui.applyTheme();

  $effect(() => {
    void auth.boot();
  });

  // First launch installs + starts the daemon in the background; poll until
  // it answers instead of parking on a manual Retry button.
  $effect(() => {
    if (auth.phase !== 'offline') return;
    const timer = setInterval(() => void auth.boot(true), 2000);
    return () => clearInterval(timer);
  });
</script>

{#if router.module === 's'}
  <!-- Guest share view: a scoped share-link recipient has no account, so this
       route must bypass the login/onboarding gate entirely and render the
       single-session SharePage using the token captured from the URL fragment. -->
  <SharePage sessionId={router.parts[1] ?? ''} />
{:else if router.module === 'bar'}
  <!-- Desktop shell's assistant bar panel (`otto-bar`): a transparent
       chromeless window, so it never renders the boot screens or the Shell —
       the bar handles signed-out/offline states itself. -->
  <BarHost />
{:else if router.module === 'tray'}
  <!-- Desktop shell's menu-bar popover (`otto-tray`); same rules as the bar. -->
  <TrayPage />
{:else if auth.phase === 'loading' && isEmbedded}
  <!-- The side-by-side pane boots under its host's loading cover: no second
       "Otto" splash inside the pane. -->
  <div class="boot" aria-busy="true"></div>
{:else if auth.phase === 'loading'}
  <div class="boot">
    <div class="boot-mark">Otto</div>
    <div class="boot-sub" role="status">Connecting to the Otto daemon…</div>
  </div>
{:else if auth.phase === 'offline'}
  <div class="boot">
    <div class="boot-mark">Otto</div>
    <div class="boot-sub" role="status">
      Starting the Otto daemon — first launch can take a few seconds…
    </div>
    <button class="btn primary" onclick={() => auth.boot(true)}>Retry now</button>
  </div>
{:else if auth.phase === 'onboarding'}
  <Onboarding />
{:else if auth.phase === 'login'}
  <Login />
{:else}
  <Shell />
{/if}

<Toasts />

<style>
  .boot {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    background: var(--bg);
  }
  .boot-mark {
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.02em;
    background: linear-gradient(120deg, var(--accent), color-mix(in srgb, var(--accent) 50%, var(--text)));
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .boot-sub {
    font-size: 13px;
    color: var(--text-dim);
  }
</style>
