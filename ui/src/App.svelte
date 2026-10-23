<script lang="ts">
  import { onMount, untrack } from 'svelte';
  // Boot flow: GET /meta → onboarding wizard | login | main shell.
  import Toasts from './lib/components/Toasts.svelte';
  import Announcer from './lib/components/Announcer.svelte';
  import BarHost from './modules/desktop/BarHost.svelte';
  import TrayPage from './modules/desktop/TrayPage.svelte';
  import { auth } from './lib/stores/auth.svelte';
  import { appLive } from './lib/live';
  import { router } from './lib/router.svelte';
  import { ui } from './lib/stores/ui.svelte';
  import { isEmbedded, isTauri } from './lib/desktop';
  import {
    daemonSupervisor,
    installFailed,
    offlineDetail,
    supervisorErrorText,
    type DaemonReport,
  } from './modules/desktop/daemonState';
  import { preloadRoute } from './shell/pages.svelte';

  ui.applyTheme();

  // The shell (the whole app: ~every module + store) and the boot screens load
  // as their own chunks, on demand. The desktop's hidden, always-alive `bar`
  // and `tray` webviews render only BarHost / TrayPage, so they no longer parse
  // and compile the entire app — and start its global timers — at launch.
  // One promise per chunk, so re-renders never re-import.
  let shellChunk: Promise<typeof import('./shell/App.svelte')> | null = null;
  let onboardingChunk: Promise<typeof import('./modules/settings/Onboarding.svelte')> | null = null;
  let loginChunk: Promise<typeof import('./modules/settings/Login.svelte')> | null = null;
  let gameGuestChunk: Promise<typeof import('./modules/rooms/games/GameGuest.svelte')> | null = null;
  let roomChunk: Promise<typeof import('./modules/rooms/RoomGuest.svelte')> | null = null;
  let hostRoomChunk: Promise<typeof import('./modules/rooms/RoomHost.svelte')> | null = null;
  let shareChunk: Promise<typeof import('./modules/share/SharePage.svelte')> | null = null;
  // The shell resolves together with the page its route shows (each page is
  // its own chunk, shell/pages.svelte.ts), so the first shell paint has its
  // page. `untrack`: the template's `{#await shell()}` must not depend on the
  // route — a sidebar switch re-running it is how the shell used to remount.
  const shell = () =>
    (shellChunk ??= Promise.all([import('./shell/App.svelte'), preloadRoute(untrack(() => router.parts))]).then(([m]) => m));
  const onboarding = () => (onboardingChunk ??= import('./modules/settings/Onboarding.svelte'));
  const login = () => (loginChunk ??= import('./modules/settings/Login.svelte'));
  const gameGuest = () => (gameGuestChunk ??= import('./modules/rooms/games/GameGuest.svelte'));
  const room = () => (roomChunk ??= import('./modules/rooms/RoomGuest.svelte'));
  const hostRoom = () => (hostRoomChunk ??= import('./modules/rooms/RoomHost.svelte'));
  const share = () => (shareChunk ??= import('./modules/share/SharePage.svelte'));
  // The main window fetches the shell chunk while /meta is still in flight.
  if (!['bar', 'tray', 's', 'room', 'room-host', 'game-room'].includes(router.module)) void shell().catch(() => {});

  // Boot once per entry into an authenticated route, not per navigation:
  // reading `router.module` inside the effect re-ran boot() on every sidebar
  // switch, which reset the phase to 'loading' and remounted the whole shell.
  const needsAuth = $derived(router.module !== 'room' && router.module !== 'game-room');
  $effect(() => {
    if (needsAuth) untrack(() => void auth.boot());
  });

  // Device telemetry is administrator opt-in. Capture no guest/member activity.
  $effect(() => {
    if (router.module === 'game-room' || auth.phase !== 'ready' || !auth.isRoot) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    const module = untrack(() => router.module);
    void import('./lib/telemetryBoot').then(({ bootTelemetry }) => {
      if (!disposed) stop = bootTelemetry(module);
    });
    return () => { disposed = true; stop?.(); };
  });

  // First launch installs + starts the daemon in the background; poll until
  // it answers instead of parking on a manual Retry button.
  $effect(() => {
    if (router.module === 'room' || router.module === 'game-room' || auth.phase !== 'offline') return;
    const timer = setInterval(() => void auth.boot(true), 2000);
    return () => clearInterval(timer);
  });

  // The first ~15 s offline is a normal first launch ("starting…"); past that
  // the daemon is probably not coming up on its own, so say so and point at
  // the log instead of promising a start forever.
  const OFFLINE_GIVE_UP_MS = 15_000;
  let offlineLong = $state(false);
  $effect(() => {
    if (auth.phase !== 'offline') {
      offlineLong = false;
      return;
    }
    const t = setTimeout(() => (offlineLong = true), OFFLINE_GIVE_UP_MS);
    return () => clearTimeout(t);
  });
  // A failed `/auth/capabilities` read is retried with backoff inside the
  // store; a (re)connected events socket — the daemon answers again, maybe a
  // restarted one — retries at once (error-handling-1-05).
  $effect(() => appLive.onConnection((up) => {
    if (up) auth.noteConnected();
  }));

  // The desktop supervisor's diagnosis (error-handling-5-01): the shell
  // publishes `otto://daemon-state` while it installs/starts ottod. Keep the
  // last report so the offline screen says WHY (a wedged daemon, a refused
  // bootstrap) and offers the recovery the shell can do. A window opened
  // after the boot event asks for the status (which carries the last
  // diagnosis); on a first run with nothing installed it starts the daemon
  // (single-flight in the shell).
  let daemonReport = $state<DaemonReport | null>(null);
  const daemonDetail = $derived(offlineDetail(daemonReport));
  onMount(() => {
    if (!isTauri || ['bar', 'tray', 's', 'room', 'game-room'].includes(untrack(() => router.module))) return;
    let disposed = false;
    let stop: () => void = () => {};
    void daemonSupervisor
      .onState((report) => (daemonReport = report))
      .then((fn) => (disposed ? fn() : (stop = fn)))
      .catch(() => {});
    void daemonSupervisor
      .status()
      .then(async (report) => {
        if (!report || disposed) return;
        daemonReport ??= report;
        if (!report.healthy && !report.installed) await daemonSupervisor.start();
      })
      .catch(() => {});
    return () => {
      disposed = true;
      stop();
    };
  });

  // Restart replaces a live but wedged daemon (`launchctl kickstart -k`; a
  // reinstall when the job isn't loaded) — the case quitting and reopening
  // Otto never fixes. Reinstall is offered after a failed install. Either one
  // then re-probes at once; the 2 s poll keeps running while offline.
  let daemonAction = $state<'restart' | 'reinstall' | null>(null);
  let daemonActionError = $state('');
  async function runDaemonAction(kind: 'restart' | 'reinstall'): Promise<void> {
    if (daemonAction) return;
    daemonAction = kind;
    daemonActionError = '';
    try {
      if (kind === 'restart') await daemonSupervisor.restart();
      else await daemonSupervisor.reinstall();
      // The old diagnosis no longer applies; a new one arrives as an event.
      daemonReport = null;
      await auth.boot(true);
    } catch (e) {
      daemonActionError = `${kind === 'restart' ? 'Restart' : 'Reinstall'} failed: ${supervisorErrorText(e)}`;
    } finally {
      daemonAction = null;
    }
  }

  // "Retry now" is inert while its own attempt is in flight (the 2 s poll
  // already dedupes inside auth.boot, but the button gave no feedback).
  let retrying = $state(false);
  async function retryNow(): Promise<void> {
    if (retrying) return;
    retrying = true;
    try {
      await auth.boot(true);
    } finally {
      retrying = false;
    }
  }
</script>

{#if router.module === 'game-room'}
  {#await gameGuest()}{@render chunkWait()}{:then m}{#key router.parts[1]}<m.default roomId={router.parts[1] ?? ''} />{/key}{:catch}{@render chunkError()}{/await}
{:else if router.module === 'room'}
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
    {#if offlineLong}
      <div class="boot-sub" role="status">Otto can’t reach the daemon on 127.0.0.1:7700.</div>
      <div class="boot-hint">
        Check the daemon log at <span class="mono">~/Library/Logs/Otto/ottod.log</span>, then retry. Otto keeps trying in the background.
      </div>
    {:else}
      <div class="boot-sub" role="status">
        Starting the Otto daemon — first launch can take a few seconds…
      </div>
    {/if}
    {#if daemonDetail}
      <div class="boot-hint mono">{daemonDetail}</div>
    {/if}
    {#if daemonActionError}
      <div class="boot-hint boot-error" role="alert">{daemonActionError}</div>
    {/if}
    <div class="boot-actions">
      <button class="btn primary" onclick={retryNow} disabled={retrying}>{retrying ? 'Retrying…' : 'Retry now'}</button>
      {#if isTauri}
        <button class="btn" onclick={() => void runDaemonAction('restart')} disabled={daemonAction !== null}>
          {daemonAction === 'restart' ? 'Restarting…' : 'Restart daemon'}
        </button>
        {#if installFailed(daemonReport)}
          <button class="btn" onclick={() => void runDaemonAction('reinstall')} disabled={daemonAction !== null}>
            {daemonAction === 'reinstall' ? 'Reinstalling…' : 'Reinstall daemon'}
          </button>
        {/if}
      {/if}
    </div>
  </div>
{:else if auth.phase === 'onboarding'}
  {#await onboarding()}{@render chunkWait()}{:then m}<m.default />{:catch}{@render chunkError()}{/await}
{:else if auth.phase === 'login'}
  {#await login()}{@render chunkWait()}{:then m}<m.default />{:catch}{@render chunkError()}{/await}
{:else if router.module === 'room-host'}
  <!-- Local authenticated host windows own their room lifecycle independently
       of the workspace shell. Remote guests still use the unprivileged route. -->
  {#await hostRoom()}{@render chunkWait()}{:then m}{#key router.parts[1]}<m.default roomId={router.parts[1] ?? ''} />{/key}{:catch}{@render chunkError()}{/await}
{:else}
  <!-- The permissions-failure strip lives in the shell (shell/CapsBanner),
       in flow above the status bar — never over the toolbar. -->
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
<Announcer />

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
    font-size: var(--fs-hero);
    font-weight: 600;
    letter-spacing: -0.01em;
    background: linear-gradient(120deg, var(--accent), color-mix(in srgb, var(--accent) 50%, var(--text)));
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .boot-sub {
    font-size: var(--fs-m);
    color: var(--text-dim);
  }
  .boot-hint {
    max-width: 420px;
    padding-inline: 24px;
    text-align: center;
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .boot-error {
    color: var(--danger);
  }
  .boot-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
</style>
