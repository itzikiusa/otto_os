///<reference types="svelte" />
;
import { announce, announceShown } from '../../../lib/announce.svelte';
// The one-time "Enable live browsing" step. Outside the desktop app a live
// tab runs in a daemon-owned Chromium, downloaded once — never silently:
// this pane says what is downloaded, how big it is and where it runs, and
// nothing happens until someone clicks (api.md "Browser — remote live
// view": POST /browser/live/install is Browser Admin, sha256-pinned).
//
//   missing     → EmptyState + the one CTA ("Download Chrome (180 MB)"),
//                 with a "lighter engine" option underneath
//   installing  → progress (role=progressbar): downloading → verifying →
//                 extracting
//   failed      → error EmptyState + Retry
//   unsupported → honest note (this Mac can't run it) + Reader
//   no admin    → says who can enable it
import EmptyState from '../../../lib/components/EmptyState.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../../lib/loadError';
import Icon from '../../../lib/components/Icon.svelte';
import { formatBytes } from '../../../lib/metric-format';
import { auth } from '../../../lib/stores/auth.svelte';
import { browserLive } from '../../../lib/stores/browserLive.svelte';
import type { BrowserChromeBuild } from '../../../lib/api/types';
;

  interface Props {
    /** "Switch to Reader" — the escape hatch for this tab. */
    onreader?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onreader }: Props = $props();

  let lighter = $state(false);
  let startError = $state('');

  const st = $derived(browserLive.status);
  const engine = $derived(browserLive.engineState);
  const build: BrowserChromeBuild = $derived(lighter ? 'chrome-headless-shell' : 'chrome');
  const canInstall = $derived(auth.can('browser', 'admin'));
  const pinned = (b: BrowserChromeBuild) => browserLive.build(b)?.sha256_pinned !== false;
  const lighterAvailable = $derived(pinned('chrome-headless-shell'));

  async function enable(): Promise<void> {
    startError = '';
    try {
      await browserLive.install(build);
    } catch (e) {
      startError = loadErrorText(e);
    }
  }

  const job = $derived(st?.install ?? null);
  const received = $derived(job?.received_bytes ?? 0);
  const total = $derived(job?.total_bytes ?? (job ? browserLive.downloadBytes(job.build) : 0));
  const pct = $derived(total > 0 ? Math.min(100, Math.round((received / total) * 100)) : 0);
  const phase = $derived(
    job?.state === 'verifying' ? 'Checking the download…' : job?.state === 'extracting' ? 'Unpacking…' : null,
  );
  const jobName = $derived(job?.build === 'chrome-headless-shell' ? 'the lighter Chromium engine' : 'Chrome for Testing');

  const body = $derived(
    `Live tabs run in a Chromium browser on the Mac running Otto and stream here, so they work from any device, including your phone. ` +
      `Otto downloads ${lighter ? 'the lighter Chromium engine' : 'Chrome for Testing'} once (about ${formatBytes(browserLive.downloadBytes(build))}) and checks it against a pinned checksum. ` +
      `Every request the pages make goes through Otto’s network guard.`,
  );
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  const unpinned = $derived(!pinned(build));
  $effect(() => { if (unpinned) announce('This Otto build has no checksum for that engine, so it can’t be downloaded safely.', { assertive: true }); });
  $effect(() => { if (startError) announce(`Couldn’t start the download: ${startError}`, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", {   "class":`setup`,...__sveltets_2_empty({"data-testid":`live-engine-setup`}),});
  if((browserLive.loadError && !st) || engine === 'unknown'){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            "variant":`page`,"what":`the live browser engine’s status`,"loading":!browserLive.loadError,"error":browserLive.loadError && !st ? browserLive.loadError : null,"empty":true,"onretry":() => void browserLive.load(),}});}
  } else if (engine === 'installing'){
    
     { svelteHTML.createElement("div", { "class":`progress-card`,});
       { svelteHTML.createElement("div", { "class":`icon-tile`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":24,}});} }
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('h3'),(phase ?? `Downloading ${jobName}…`)));{ svelteHTML.createElement("h3", __sveltets_2_union($$action_0), {  });phase ?? `Downloading ${jobName}…`; }}
       { svelteHTML.createElement("div", {             "class":`bar`,"role":`progressbar`,"aria-label":`Download progress`,"aria-valuemin":0,"aria-valuemax":100,"aria-valuenow":phase ? 100 : pct,});
         { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${phase ? 100 : pct} `); }
       }
       { svelteHTML.createElement("p", { "class":`meta`,});
        if(phase){formatBytes(total); }else{formatBytes(received);  formatBytes(total);  pct; }
       }
       { svelteHTML.createElement("p", { "class":`hint`,});              }
     }
  } else if (engine === 'failed'){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {                 "variant":`page`,"tone":`error`,"icon":`warning`,"title":`Couldn’t install the live browser engine`,"body":job?.error || 'The download stopped before it finished.',"actionLabel":canInstall && !browserLive.starting ? 'Retry' : undefined,"actionIcon":`refresh`,"onaction":() => void browserLive.install(job?.build ?? 'chrome'),}});}
  } else if (engine === 'unsupported'){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`globe`,"title":`Live browsing isn’t available on this Mac`,"body":`The live browser engine runs on Apple silicon Macs only for now. Reader view still works for every page.`,"actionLabel":onreader ? 'Switch to Reader' : undefined,"onaction":onreader,}});}
  } else if (!canInstall){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`globe`,"title":`Live browsing isn’t enabled yet`,"body":`Live tabs need a one-time Chromium download on the Mac running Otto. Ask an Otto admin to enable it in Settings → Browser. Reader view works meanwhile.`,"actionLabel":onreader ? 'Switch to Reader' : undefined,"onaction":onreader,}});}
  }else{
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {              children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`globe`,"title":`Enable live browsing`,body,"actionLabel":browserLive.starting
        ? 'Starting the download…'
        : `${lighter ? 'Download lighter engine' : 'Download Chrome'} (${formatBytes(browserLive.downloadBytes(build))})`,"actionIcon":`download`,"onaction":pinned(build) ? () => void enable() : undefined,}});
      if(lighterAvailable){
         { svelteHTML.createElement("label", { "class":`checkbox-row lighter`,});
           { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":lighter,});/*Ωignore_startΩ*/() => lighter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {});
                  formatBytes(browserLive.downloadBytes('chrome-headless-shell'));
             { svelteHTML.createElement("span", { "class":`hint`,});           }
           }
         }
      }
      if(!pinned(build)){
         { svelteHTML.createElement("p", { "class":`field-error`,});               }
      }
      if(startError){
         { svelteHTML.createElement("p", { "class":`field-error`,});    startError; }
      }
      if(onreader){
         { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":onreader,});    }
      }
     EmptyState}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LiveEngineSetup__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LiveEngineSetup__SvelteComponent_ = ReturnType<typeof LiveEngineSetup__SvelteComponent_>;
/*Ωignore_endΩ*/export default LiveEngineSetup__SvelteComponent_;