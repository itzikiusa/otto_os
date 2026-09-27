<script lang="ts">
  // Boot flow: GET /meta → onboarding wizard | login | main shell.
  import Toasts from './lib/components/Toasts.svelte';
  import BarHost from './modules/desktop/BarHost.svelte';
  import TrayPage from './modules/desktop/TrayPage.svelte';
  import { auth } from './lib/stores/auth.svelte';
  import { router } from './lib/router.svelte';
  import { ui } from './lib/stores/ui.svelte';
  import { isEmbedded } from './lib/desktop';

  ui.applyTheme();

  // The shell (the whole app: ~every module + store) and the boot screens load
  // as their own chunks, on demand. The desktop's hidden, always-alive `bar`
  // and `tray` webviews render only BarHost / TrayPage, so they no longer parse
  // and compile the entire app — and start its global timers — at launch.
  // One promise per chunk, so re-renders never re-import.
  let shellChunk: Promise<typeof import('./shell/App.svelte')> | null = null;
  let onboardingChunk: Promise<typeof import('./modules/settings/Onboarding.svelte')> | null = null;
  let loginChunk: Promise<typeof import('./modules/settings/Login.svelte')> | null = null;
  let roomChunk: Promise<typeof import('./modules/rooms/RoomGuest.svelte')> | null = null;
  let shareChunk: Promise<typeof import('./modules/share/SharePage.svelte')> | null = null;
  const shell = () => (shellChunk ??= import('./shell/App.svelte'));
  const onboarding = () => (onboardingChunk ??= import('./modules/settings/Onboarding.svelte'));
  const login = () => (loginChunk ??= import('./modules/settings/Login.svelte'));
  const room = () => (roomChunk ??= import('./modules/rooms/RoomGuest.svelte'));
  const share = () => (shareChunk ??= import('./modules/share/SharePage.svelte'));
  // The main window fetches the shell chunk while /meta is still in flight.
  if (router.module !== 'bar' && router.module !== 'tray' && router.module !== 's' && router.module !== 'room') void shell().catch(() => {});

  $effect(() => {
    if (router.module !== 'room') void auth.boot();
  });

  // First launch installs + starts the daemon in the background; poll until
  // it answers instead of parking on a manual Retry button.
  $effect(() => {
    if (router.module === 'room' || auth.phase !== 'offline') return;
    const timer = setInterval(() => void auth.boot(true), 2000);
    return () => clearInterval(timer);
  });
</script>

{#if router.module === 'room'}
  {#await room()}{@render chunkWait()}{:then m}{#key router.parts[1]}<m.default roomId={router.parts[1] ?? ''} />{/key}{:catch}{@render chunkError()}{/await}
{:else if router.module === 's'}
  <!-- Guest share view: a scoped share-link recipient has no account, so this
       route must bypass the login/onboarding gate entirely and render the
       single-session SharePage using the token captured from the URL fragment. -->
  {#await share() then m}<m.default sessionId={router.parts[1] ?? ''} />{:catch}{@render chunkError()}{/await}
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
  {#await onboarding()}{@render chunkWait()}{:then m}<m.default />{:catch}{@render chunkError()}{/await}
{:else if auth.phase === 'login'}
  {#await login()}{@render chunkWait()}{:then m}<m.default />{:catch}{@render chunkError()}{/await}
{:else}
  {#await shell()}{@render chunkWait()}{:then m}<m.default />{:catch}{@render chunkError()}{/await}
{/if}

{#snippet chunkWait()}
  <div class="boot" aria-busy="true"></div>
{/snippet}

{#snippet chunkError()}
  <div class="boot">
    <div class="boot-mark">Otto</div>
    <div class="boot-sub" role="alert">Otto couldn’t finish loading.</div>
    <button class="btn primary" onclick={() => location.reload()}>Reload</button>
  </div>
{/snippet}

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
