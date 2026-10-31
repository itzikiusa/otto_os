///<reference types="svelte" />
;// Hosts a runtime plugin's UI in an iframe served by the daemon at
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
import { api, baseUrl, getToken } from '../../lib/api/client';
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
import { pluginFrameAction, pluginSlugOk, postPluginInit } from './pluginFrame';

;type $$ComponentProps =  { slug: string };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { slug }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const origin = new URL(baseUrl()).origin;
  /** Display name: the manifest name once the nav list has it, else the slug. */
  const name = $derived(plugins.get(slug)?.name ?? slug);
  /** Plugin slugs are plain identifiers. Anything else (`..`, `/`, `%2e`) is
   *  never interpolated into a URL nor sent `otto:init` (pluginFrame.ts). */
  const slugOk = $derived(pluginSlugOk(slug));
  const src = $derived(`${origin}/plugins/${encodeURIComponent(slug)}/ui/`);
  let frame = $state<HTMLIFrameElement | undefined>();

  // 'missing' = the daemon has no UI for this slug (disabled, uninstalled or
  // UI-less) — not a failure to retry, so it gets an empty state with a way
  // to the plugin settings instead of "Couldn’t load" + Retry.
  // 'stopped' = the page is served but its service (sidecar) is down, so
  // every call it makes would 502: say so, with Start again for an admin.
  let probe = $state<'loading' | 'ok' | 'error' | 'missing' | 'stopped'>('loading');
  let starting = $state(false);
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
        // Fresh nav state: the page is static, the service may have died.
        await plugins.load();
        if (seq !== probeSeq) return;
        probeError = null;
        probe = plugins.get(slug)?.running === false ? 'stopped' : 'ok';
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

  /** Admin: start the stopped service again (re-runs enable), then re-probe. */
  async function startAgain(): Promise<void> {
    if (starting) return;
    starting = true;
    try {
      await api.post(`/plugin-admin/${encodeURIComponent(slug)}/enable`);
      await check();
    } catch (e) {
      probeError = loadErrorText(e);
    } finally {
      starting = false;
    }
  }

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
    postPluginInit(slug, (extra) => postTheme('otto:init', extra), {
      baseUrl: baseUrl(),
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
    const action = pluginFrameAction(ev, frame?.contentWindow);
    if (action?.kind === 'focus-leave') {
      document.querySelector<HTMLElement>('.plugin-page .icon-btn')?.focus();
    } else if (action?.kind === 'keydown') {
      relayKey(action.init);
    }
  }
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onmessage":onMessage,});}


 { svelteHTML.createElement("div", { "class":`plugin-page`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: { "title":name,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(probe === 'ok'){
        
         { svelteHTML.createElement("button", {         "class":`icon-btn`,...__sveltets_2_empty({"data-icon":`refresh`}),"onclick":() => void check(),"aria-label":`Restart ${name}`,"title":`Restart ${name}`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}
         }
      }
      
      if(auth.isRoot && probe !== 'missing'){
         { svelteHTML.createElement("button", {           "class":`icon-btn`,...__sveltets_2_empty({"data-icon":`gear`}),...__sveltets_2_empty({"data-overflow":`-1`}),"onclick":() => router.go('settings/plugins'),"aria-label":`Plugin settings`,"title":`Plugin settings`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":14,}});}
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
  if(probe === 'ok'){
     { svelteHTML.createElement("div", { "class":`pf-host`,});
       { const $$_iframe2 = svelteHTML.createElement("iframe", {           "title":`${name} plugin`,...__sveltets_2_empty({"data-plugin":slug}),src,"onload":onload,"allow":`clipboard-write`,});frame = $$_iframe2; }
      if(!frameLoaded){
         { svelteHTML.createElement("div", { "class":`pf-loading`,});  { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {    "what":name,"variant":`page`,"loading":true,"empty":true,}});} }
      }
     }
  } else if (probe === 'stopped'){
    if(auth.isRoot){
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`box`,"title":`${name}’s service stopped`,"body":probeError ?? 'Its page is here, but the service behind it isn’t running, so nothing in it would work. Start it again, or see why in Settings → Plugins.',"actionLabel":starting ? 'Starting…' : 'Start again',"actionIcon":`refresh`,"onaction":() => void startAgain(),}});}
    }else{
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`box`,"title":`${name}’s service stopped`,"body":`Its page is here, but the service behind it isn’t running. Ask an Otto admin to start it again in Settings → Plugins.`,}});}
    }
  } else if (probe === 'missing'){
    if(auth.isRoot){
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`box`,"title":`${name} isn’t available`,"body":`It’s disabled, uninstalled, or ships no page of its own. Enable or reinstall it in Settings → Plugins.`,"actionLabel":`Open plugin settings`,"actionIcon":`gear`,"onaction":() => router.go('settings/plugins'),}});}
    }else{
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`box`,"title":`${name} isn’t available`,"body":`It’s disabled, uninstalled, or ships no page of its own. Ask an Otto admin to enable it.`,}});}
    }
  }else{
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            "what":name,"variant":`page`,"loading":probe === 'loading',"error":probeError,"empty":true,"onretry":() => void check(),}});}
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PluginFrame__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PluginFrame__SvelteComponent_ = ReturnType<typeof PluginFrame__SvelteComponent_>;
/*Ωignore_endΩ*/export default PluginFrame__SvelteComponent_;