///<reference types="svelte" />
;// Settings → Browser: how live tabs render, and the daemon's Chromium
// (api.md "Browser — remote live view"). Engine choices are daemon-wide and
// Browser Admin; the renderer choice is per device (desktop app only).
//
// The Chromium download is never silent: choosing a build that isn't
// installed only offers a Download button that says how big it is.

import PageHeader from '../../lib/components/PageHeader.svelte';
import { announceShown } from '../../lib/announce.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { browserLive } from '../../lib/stores/browserLive.svelte';
import { nativeBrowserAvailable } from '../../lib/nativeBrowser';
import { formatBytes } from '../../lib/metric-format';
import { toasts } from '../../lib/toast.svelte';
import type { BrowserChromeBuild, BrowserLiveSettings } from '../../lib/api/types';
import SectionIntro from './SectionIntro.svelte';
import SettingToggle from './SettingToggle.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // A failed load stays failed until Retry (LoadState below): without the
  // `loadError` term the effect re-fired on every settle and hammered the
  // daemon in a tight loop (a 403, a 5xx, a restarting daemon).
  $effect(() => {
    if (!browserLive.status && !browserLive.loading && !browserLive.loadError && browserLive.supported !== false) void browserLive.load();
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
      toasts.success('Browser engine settings saved', 'Used the next time the live browser starts.');
    } catch (e) {
      saveError = loadErrorText(e);
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
      installError = loadErrorText(e);
    }
  }

  const job = $derived(st?.install ?? null);
  const running = $derived(browserLive.engineState === 'installing');
  const pct = $derived(
    job && job.total_bytes ? Math.min(100, Math.round((job.received_bytes / job.total_bytes) * 100)) : 0,
  );
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('browser'),"subtitle":`Live tabs and the browser engine that runs them`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
     { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
                        
                      
              
     SectionIntro}

     { svelteHTML.createElement("section", {   "class":`card bs-card`,"aria-labelledby":`bs-panel`,});
       { svelteHTML.createElement("h2", {   "id":`bs-panel`,"class":`row-title`,});    }
       { svelteHTML.createElement("p", { "class":`row-desc`,});             }
       { svelteHTML.createElement("div", {     "class":`choices`,"role":`radiogroup`,"aria-labelledby":`bs-panel`,});
         { svelteHTML.createElement("label", { "class":`choice`,});
           { svelteHTML.createElement("input", {         "type":`radio`,"name":`panel-browser`,"checked":ui.browserPanelVersion === 'v2',"onchange":() => ui.setBrowserPanelVersion('v2'),});}
           { svelteHTML.createElement("span", {});
             { svelteHTML.createElement("span", { "class":`choice-title`,});   }
             { svelteHTML.createElement("span", { "class":`row-desc`,});                  }
           }
         }
         { svelteHTML.createElement("label", { "class":`choice`,});
           { svelteHTML.createElement("input", {         "type":`radio`,"name":`panel-browser`,"checked":ui.browserPanelVersion === 'v1',"onchange":() => ui.setBrowserPanelVersion('v1'),});}
           { svelteHTML.createElement("span", {});
             { svelteHTML.createElement("span", { "class":`choice-title`,});   }
             { svelteHTML.createElement("span", { "class":`row-desc`,});          }
           }
         }
       }
     }

    if(nativeBrowserAvailable){
       { svelteHTML.createElement("section", {   "class":`card bs-card`,"aria-labelledby":`bs-renderer`,});
         { svelteHTML.createElement("h2", {   "id":`bs-renderer`,"class":`row-title`,});     }
         { svelteHTML.createElement("div", {     "class":`choices`,"role":`radiogroup`,"aria-labelledby":`bs-renderer`,});
           { svelteHTML.createElement("label", { "class":`choice`,});
             { svelteHTML.createElement("input", {         "type":`radio`,"name":`renderer`,"checked":browserLive.pref === 'native',"onchange":() => browserLive.setPref('native'),});}
             { svelteHTML.createElement("span", {});
               { svelteHTML.createElement("span", { "class":`choice-title`,});    }
               { svelteHTML.createElement("span", { "class":`row-desc`,});          }
             }
           }
           { svelteHTML.createElement("label", { "class":`choice`,});
             { svelteHTML.createElement("input", {         "type":`radio`,"name":`renderer`,"checked":browserLive.pref === 'remote',"onchange":() => browserLive.setPref('remote'),});}
             { svelteHTML.createElement("span", {});
               { svelteHTML.createElement("span", { "class":`choice-title`,});  }
               { svelteHTML.createElement("span", { "class":`row-desc`,});                }
             }
           }
         }
       }
    }

    if(browserLive.supported === false){
       { svelteHTML.createElement("section", { "class":`card bs-card`,});
         { svelteHTML.createElement("p", { "class":`row-desc`,});              }
       }
    } else if (browserLive.loadError && !st){
       { svelteHTML.createElement("section", { "class":`card bs-card`,});
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the browser engine settings`,"error":browserLive.loadError,"empty":true,"onretry":() => void browserLive.load(),}});}
       }
    } else if (!st){
       { svelteHTML.createElement("section", { "class":`card bs-card`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":2,"height":28,"label":`browser engine settings`,}});} }
    }else{
       { svelteHTML.createElement("section", {   "class":`card bs-card`,"aria-labelledby":`bs-engine`,});
         { svelteHTML.createElement("h2", {   "id":`bs-engine`,"class":`row-title`,});  }
        if(!st.platform_supported){
           { svelteHTML.createElement("p", { "class":`row-desc`,});            }
        }else{
           { svelteHTML.createElement("p", { "class":`row-desc`,});
                          
               
           }
           { svelteHTML.createElement("div", {     "class":`choices`,"role":`radiogroup`,"aria-labelledby":`bs-engine`,});
               for(let b of __sveltets_2_ensureArray(BUILDS)){b.build;
              const info = browserLive.build(b.build);
               { svelteHTML.createElement("div", { "class":`choice`,});
                 { svelteHTML.createElement("input", {               "id":`bs-build-${b.build}`,"type":`radio`,"name":`build`,"checked":st.settings.build === b.build,"disabled":!isAdmin || saving || (b.build === 'chrome-headless-shell' && st.settings.headed),"title":b.build === 'chrome-headless-shell' && st.settings.headed
                    ? 'Turn off “Show the window on this Mac” first — the lighter engine has no window'
                    : undefined,"onchange":() => void save({ build: b.build }),});}
                 { svelteHTML.createElement("label", {   "for":`bs-build-${b.build}`,"class":`choice-text`,});
                   { svelteHTML.createElement("span", { "class":`choice-title`,});
                    b.name;
                    if(info?.installed){ { svelteHTML.createElement("span", { "class":`chip ok`,}); info.version ? ` · ${info.version}` : ''; }}
                   }
                  
                   { svelteHTML.createElement("span", { "class":`row-desc`,});b.blurb;if(!(isAdmin && info && !info.installed && info.sha256_pinned)){' '; formatBytes(browserLive.downloadBytes(b.build)); } }
                  if(info && !info.sha256_pinned){
                     { svelteHTML.createElement("span", { "class":`row-desc`,});             }
                  }
                 }
                if(isAdmin && info && !info.installed && info.sha256_pinned){
                   { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":running || browserLive.starting,"onclick":() => void install(b.build),});
                     formatBytes(browserLive.downloadBytes(b.build));
                   }
                }
               }
            }
           }
          if(running && job){
             {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(job.state === 'downloading' ? 'Downloading the browser engine…' : job.state === 'verifying' ? 'Checking the download…' : 'Unpacking the browser engine…')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`progress`,});
               { svelteHTML.createElement("div", {           "class":`bar`,"role":`progressbar`,"aria-label":`Engine download`,"aria-valuemin":0,"aria-valuemax":100,"aria-valuenow":pct,});
                 { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${job.state === 'downloading' ? pct : 100} `); }
               }
               { svelteHTML.createElement("span", { "class":`row-desc`,});
                if(job.state === 'downloading'){ formatBytes(job.received_bytes);job.total_bytes ? ` of ${formatBytes(job.total_bytes)}` : '';
                } else if (job.state === 'verifying'){  }else{ }
               }
             }}
          } else if (job?.state === 'failed'){
             { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});    job.error || 'the download ended without a reason. Try again, or check Settings › Logs.'; }
          }
          if(installError){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});    installError; }}
        }
       }

       { svelteHTML.createElement("section", { "class":`card bs-card toggle-card`,});
         { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {             "label":`Show the window on this Mac`,"hint":`Opens the live browser as a visible Chrome window on the Mac running Otto, as well as streaming it. Needs Chrome for Testing. Off by default.`,"checked":st.settings.headed,"disabled":!isAdmin || saving || st.settings.build !== 'chrome',"title":!isAdmin ? 'Only a Browser admin can change this' : st.settings.build !== 'chrome' ? 'Needs Chrome for Testing' : undefined,"onchange":(v) => save({ headed: v }),}});}
       }

       { svelteHTML.createElement("section", {   "class":`card bs-card`,"aria-labelledby":`bs-downloads`,});
         { svelteHTML.createElement("div", { "class":`bs-row`,});
           { svelteHTML.createElement("div", { "class":`row-text`,});
             { svelteHTML.createElement("h2", {   "id":`bs-downloads`,"class":`row-title`,});    }
             { svelteHTML.createElement("span", { "class":`row-desc`,});                }
           }
           { svelteHTML.createElement("div", { "class":`row-controls`,});
             { svelteHTML.createElement("select", {           "class":`input`,"aria-labelledby":`bs-downloads`,"value":st.settings.downloads,"disabled":!isAdmin || saving,"onchange":(e) => void save({ downloads: (e.currentTarget as HTMLSelectElement).value as 'block' | 'quarantine' }),});
               { svelteHTML.createElement("option", { "value":`quarantine`,});   }
               { svelteHTML.createElement("option", { "value":`block`,});  }
             }
           }
         }
       }

      
       { svelteHTML.createElement("p", { "class":`row-desc note`,});
                       
                       
       }
      if(!isAdmin){
         { svelteHTML.createElement("p", { "class":`row-desc note`,});         }
      }
      if(saveError){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});  saveError; }}
    }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrowserSettings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrowserSettings__SvelteComponent_ = ReturnType<typeof BrowserSettings__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrowserSettings__SvelteComponent_;