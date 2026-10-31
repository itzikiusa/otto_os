///<reference types="svelte" />
;
import { onMount, untrack } from 'svelte';
// Boot flow: GET /meta → onboarding wizard | login | main shell.
import Toasts from './lib/components/Toasts.svelte';
import Announcer from './lib/components/Announcer.svelte';
import { announce } from './lib/announce.svelte';
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
function $$render() {
  const chunkWait/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", {   "class":`boot`,"aria-busy":`true`,}); }
};return __sveltets_2_any(0)};  const chunkError/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", { "class":`boot`,});
     { svelteHTML.createElement("div", { "class":`boot-mark`,});  }
     { svelteHTML.createElement("div", {   "class":`boot-sub`,"role":`alert`,});    }
     { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => location.reload(),});  }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
    // The boot lines below are inserted with their text, which VoiceOver
    // doesn't read: speak each edge through the announcer instead.
    announce('Starting the Otto daemon…');
    const t = setTimeout(() => {
      offlineLong = true;
      announce('Otto can’t reach the daemon on 127.0.0.1:7700. Check the daemon log, then retry.', { assertive: true });
    }, OFFLINE_GIVE_UP_MS);
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
;
async () => {

if(router.module === 'game-room'){
      { ;__sveltets_2_ensureSnippet(chunkWait());try { const $$_value = await (gameGuest());{ const m = $$_value; router.parts[1]; { { const $$_tluafed_m0C = __sveltets_2_ensureComponent(m.default); new $$_tluafed_m0C({ target: __sveltets_2_any(), props: {  "roomId":router.parts[1] ?? '',}});}}}} catch($$_e) { ;__sveltets_2_ensureSnippet(chunkError());}}
} else if (router.module === 'room'){
      { ;__sveltets_2_ensureSnippet(chunkWait());try { const $$_value = await (room());{ const m = $$_value; router.parts[1]; { { const $$_tluafed_m0C = __sveltets_2_ensureComponent(m.default); new $$_tluafed_m0C({ target: __sveltets_2_any(), props: {  "roomId":router.parts[1] ?? '',}});}}}} catch($$_e) { ;__sveltets_2_ensureSnippet(chunkError());}}
} else if (router.module === 's'){
  
      { try { const $$_value = await (share());{ const m = $$_value;  { const $$_tluafed_m0C = __sveltets_2_ensureComponent(m.default); new $$_tluafed_m0C({ target: __sveltets_2_any(), props: {  "sessionId":router.parts[1] ?? '',}});}}} catch($$_e) { ;__sveltets_2_ensureSnippet(chunkError());}}
} else if (router.module === 'bar'){
  
   { const $$_tsoHraB0C = __sveltets_2_ensureComponent(BarHost); new $$_tsoHraB0C({ target: __sveltets_2_any(), props: {}});}
} else if (router.module === 'tray'){
  
   { const $$_egaPyarT0C = __sveltets_2_ensureComponent(TrayPage); new $$_egaPyarT0C({ target: __sveltets_2_any(), props: {}});}
} else if (auth.phase === 'loading' && isEmbedded){
  
   { svelteHTML.createElement("div", {   "class":`boot`,"aria-busy":`true`,}); }
} else if (auth.phase === 'loading'){
   { svelteHTML.createElement("div", { "class":`boot`,});
     { svelteHTML.createElement("div", { "class":`boot-mark`,});  }
     { svelteHTML.createElement("div", {   "class":`boot-sub`,"role":`status`,});     }
   }
} else if (auth.phase === 'offline'){
   { svelteHTML.createElement("div", { "class":`boot`,});
     { svelteHTML.createElement("div", { "class":`boot-mark`,});  }
    if(offlineLong){
       { svelteHTML.createElement("div", { "class":`boot-sub`,});       }
       { svelteHTML.createElement("div", { "class":`boot-hint`,});
              { svelteHTML.createElement("span", { "class":`mono`,});  }        
       }
    }else{
       { svelteHTML.createElement("div", { "class":`boot-sub`,});
                   
       }
    }
    if(daemonDetail){
       { svelteHTML.createElement("div", { "class":`boot-hint mono`,});daemonDetail; }
    }
    if(daemonActionError){
       { svelteHTML.createElement("div", {   "class":`boot-hint boot-error`,"role":`alert`,});daemonActionError; }
    }
     { svelteHTML.createElement("div", { "class":`boot-actions`,});
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":retryNow,"disabled":retrying,});retrying ? 'Retrying…' : 'Retry now'; }
      if(isTauri){
         { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => void runDaemonAction('restart'),"disabled":daemonAction !== null,});
          daemonAction === 'restart' ? 'Restarting…' : 'Restart daemon';
         }
        if(installFailed(daemonReport)){
           { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => void runDaemonAction('reinstall'),"disabled":daemonAction !== null,});
            daemonAction === 'reinstall' ? 'Reinstalling…' : 'Reinstall daemon';
           }
        }
      }
     }
   }
} else if (auth.phase === 'onboarding'){
      { ;__sveltets_2_ensureSnippet(chunkWait());try { const $$_value = await (onboarding());{ const m = $$_value;  { const $$_tluafed_m0C = __sveltets_2_ensureComponent(m.default); new $$_tluafed_m0C({ target: __sveltets_2_any(), props: {}});}}} catch($$_e) { ;__sveltets_2_ensureSnippet(chunkError());}}
} else if (auth.phase === 'login'){
      { ;__sveltets_2_ensureSnippet(chunkWait());try { const $$_value = await (login());{ const m = $$_value;  { const $$_tluafed_m0C = __sveltets_2_ensureComponent(m.default); new $$_tluafed_m0C({ target: __sveltets_2_any(), props: {}});}}} catch($$_e) { ;__sveltets_2_ensureSnippet(chunkError());}}
} else if (router.module === 'room-host'){
  
      { ;__sveltets_2_ensureSnippet(chunkWait());try { const $$_value = await (hostRoom());{ const m = $$_value; router.parts[1]; { { const $$_tluafed_m0C = __sveltets_2_ensureComponent(m.default); new $$_tluafed_m0C({ target: __sveltets_2_any(), props: {  "roomId":router.parts[1] ?? '',}});}}}} catch($$_e) { ;__sveltets_2_ensureSnippet(chunkError());}}
}else{
  
      { ;__sveltets_2_ensureSnippet(chunkWait());try { const $$_value = await (shell());{ const m = $$_value;  { const $$_tluafed_m0C = __sveltets_2_ensureComponent(m.default); new $$_tluafed_m0C({ target: __sveltets_2_any(), props: {}});}}} catch($$_e) { ;__sveltets_2_ensureSnippet(chunkError());}}
}





 { const $$_stsaoT0C = __sveltets_2_ensureComponent(Toasts); new $$_stsaoT0C({ target: __sveltets_2_any(), props: {}});}
 { const $$_recnuonnA0C = __sveltets_2_ensureComponent(Announcer); new $$_recnuonnA0C({ target: __sveltets_2_any(), props: {}});}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const App__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type App__SvelteComponent_ = ReturnType<typeof App__SvelteComponent_>;
/*Ωignore_endΩ*/export default App__SvelteComponent_;