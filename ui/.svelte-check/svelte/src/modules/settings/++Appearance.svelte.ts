///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
import SettingToggle from './SettingToggle.svelte';
// Theme (native / pro-dark / warm), scheme (auto / light / dark), accent.
import {
    ui,
    TERM_FONT_OPTIONS,
    type SchemePref,
    type ThemeName,
    type Direction,
  } from '../../lib/stores/ui.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { radioKey } from '../../lib/radioKey';
import { AUTO_VERTICAL_ENGINES } from '../../lib/db-view-prefs';
import { dbPrefs } from '../../lib/stores/dbPrefs.svelte';
import { clipHistory } from '../../lib/stores/clipHistory.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import { AMBIENT_MODES, ambientImage, type AmbientMode } from '../../lib/ambient';
import { processWallpaper } from '../../lib/wallpaper';
import { barStore } from '../../lib/stores/bar.svelte';
import type { BarPref } from '../../lib/floatingBar';
import { plugins } from '../../lib/stores/plugins.svelte';
import {
    FAVORITES_ID,
    availableModules,
    moveAmong,
    moveWithinGroup,
    resolveGroupOrder,
    resolveOrder,
    sidebarSections,
    type SidebarPluginEntry,
  } from '../../lib/sidebar';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // The "Type or speak… ⌘K" bar over the content column (FloatingBar.svelte).
  const barPrefs: { id: BarPref; label: string; hint: string }[] = [
    { id: 'auto', label: 'Auto', hint: 'A short pill at rest, full on Home; it docks into the status bar while you scroll or type in a terminal or editor.' },
    { id: 'pinned', label: 'Always full', hint: 'The whole pill (model, spaces) stays up; it only shrinks while a terminal or editor has the keyboard.' },
    { id: 'docked', label: 'Docked', hint: 'A small chip in the status bar that never covers content; ⌘K opens the full bar.' },
    { id: 'hidden', label: 'Hidden', hint: 'No bar; ⌘K opens the command palette sheet instead.' },
  ];

  // The full resolved sidebar list (same logic as the Navigator/Rail): built-ins
  // the user may see + permitted plugins, in the saved order, including hidden
  // ones (so they can be toggled back on here).
  const sidebarPlugins = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p): SidebarPluginEntry => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const sidebarResolved = $derived(
    resolveOrder(availableModules((f) => auth.can(f, 'view'), sidebarPlugins), ui.sidebarOrder),
  );
  // Section by section exactly as the sidebar shows them: Favorites first
  // (while it has any), then the sections in the user's order.
  const sidebarSecs = $derived(sidebarSections(sidebarResolved, ui.sidebarFavorites, ui.sidebarGroupOrder));
  const sidebarFavIds = $derived(
    sidebarSecs[0]?.group.id === FAVORITES_ID ? sidebarSecs[0].modules.map((m) => m.id) : [],
  );
  const sidebarMovableSecs = $derived(
    sidebarSecs.filter((s) => s.group.id !== FAVORITES_ID).map((s) => s.group.id as string),
  );
  /** Up/down within the module's sidebar section (same rule as the Navigator);
   *  a favorite moves among the favorites. */
  function moveSidebarItem(id: string, delta: -1 | 1): void {
    if (sidebarFavIds.includes(id)) {
      const favs = moveAmong(ui.sidebarFavorites, id, delta, (x) => sidebarFavIds.includes(x));
      if (favs) ui.setSidebarFavorites(favs);
      return;
    }
    const next = moveWithinGroup(sidebarResolved, id, delta);
    if (next) ui.setSidebarOrder(next);
  }
  /** Up/down for a whole section (Favorites always stays first). */
  function moveSidebarSection(id: string, delta: -1 | 1): void {
    const all = resolveGroupOrder(ui.sidebarGroupOrder).map((g) => g.id as string);
    const next = moveAmong(all, id, delta, (g) => sidebarMovableSecs.includes(g));
    if (next) ui.setSidebarGroupOrder(next);
  }

  const themes: { id: ThemeName; name: string; desc: string }[] = [
    { id: 'native', name: 'Native', desc: 'macOS vibrancy, blue accent' },
    { id: 'pro-dark', name: 'Pro Dark', desc: 'Always-dark, violet accent' },
    { id: 'warm', name: 'Warm', desc: 'Paper tones, green accent' },
  ];
  const schemes: { id: SchemePref; label: string }[] = [
    { id: 'auto', label: 'Auto' },
    { id: 'light', label: 'Light' },
    { id: 'dark', label: 'Dark' },
  ];
  const directions: { id: Direction; label: string }[] = [
    { id: 'ltr', label: 'Left-to-right' },
    { id: 'rtl', label: 'Right-to-left' },
  ];

  // Backdrop previews: the real generated art for the accent + scheme in
  // effect (re-derived when either changes), drawn over the window colour.
  let accentNow = $state('');
  $effect(() => {
    void ui.theme;
    void ui.accent;
    void ui.resolvedScheme;
    accentNow = ui.accent || getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
  });
  function previewOf(mode: AmbientMode): string {
    const photo = ui.ambientPhoto ? ui.ambientPhoto[ui.resolvedScheme] : null;
    return ambientImage(mode, accentNow, ui.resolvedScheme, photo);
  }

  let photoBusy = $state(false);
  async function pickPhoto(e: Event & { currentTarget: HTMLInputElement }): Promise<void> {
    const input = e.currentTarget;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    photoBusy = true;
    try {
      const photo = await processWallpaper(file);
      if (ui.setAmbientPhoto(photo)) toasts.success('Wallpaper set', 'Stored on this device only.');
      else toasts.error('Couldn’t store the photo', 'This browser refused the storage — try a smaller image.');
    } catch (err) {
      toastError('Couldn’t use that image', err);
    } finally {
      photoBusy = false;
    }
  }

  const swatches: Record<ThemeName, { bg: string; fg: string; acc: string }> = {
    native: { bg: '#1e1e23', fg: '#f2f2f5', acc: '#0a84ff' },
    'pro-dark': { bg: '#16161c', fg: '#e8e8ee', acc: '#6c5ce7' },
    warm: { bg: '#211f1b', fg: '#e8e4da', acc: '#2bb673' },
  };
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('appearance'),"subtitle":`Themes apply instantly and persist per device`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", { "class":`theme-grid`,});
       for(let t of __sveltets_2_ensureArray(themes)){t.id;
       { svelteHTML.createElement("button", {      "class":`theme-card`,"aria-pressed":ui.theme === t.id,"onclick":() => ui.setTheme(t.id),});ui.theme === t.id;
         { svelteHTML.createElement("div", {   "class":`theme-preview`,"style":`background: ${swatches[t.id].bg}`,});
           { svelteHTML.createElement("div", {   "class":`tp-bar`,"style":`background: ${swatches[t.id].acc}`,}); }
           { svelteHTML.createElement("div", {   "class":`tp-line`,"style":`background: ${swatches[t.id].fg}; opacity: 0.8`,}); }
           { svelteHTML.createElement("div", {   "class":`tp-line short`,"style":`background: ${swatches[t.id].fg}; opacity: 0.4`,}); }
         }
         { svelteHTML.createElement("div", { "class":`theme-name`,});t.name; }
         { svelteHTML.createElement("div", { "class":`theme-desc`,});t.desc; }
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
  
   { svelteHTML.createElement("div", {      "class":`segmented`,"role":`group`,"aria-label":`Scheme`,});ui.theme === 'pro-dark';
       for(let s of __sveltets_2_ensureArray(schemes)){s.id;
       { svelteHTML.createElement("button", {         "aria-pressed":ui.scheme === s.id,"disabled":ui.theme === 'pro-dark',"title":ui.theme === 'pro-dark' ? 'Pro Dark is always dark' : undefined,"onclick":() => ui.setScheme(s.id),});ui.scheme === s.id;s.label; }
    }
   }
   { svelteHTML.createElement("p", { "class":`hint-line`,});
    ui.theme === 'pro-dark'
      ? 'Pro Dark is always dark. Pick Native or Warm to use a light scheme.'
      : 'Auto follows the system light/dark preference.';
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Direction`,});
       for(let d of __sveltets_2_ensureArray(directions)){d.id;
       { svelteHTML.createElement("button", {     "aria-pressed":ui.direction === d.id,"onclick":() => ui.setDirection(d.id),});ui.direction === d.id;d.label; }
    }
   }
   { svelteHTML.createElement("p", { "class":`hint-line`,});         }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", { "class":`row`,});
     { svelteHTML.createElement("input", {           "type":`color`,"class":`accent-input`,"value":ui.accent || (/^#[0-9a-f]{6}$/i.test(accentNow) ? accentNow : '#0a84ff'),"oninput":(e) => ui.setAccent(e.currentTarget.value),"aria-label":`Accent color`,});}
     { svelteHTML.createElement("span", { "class":`accent-val`,});ui.accent ? ui.accent.toUpperCase() : 'Theme default'; }
    if(ui.accent){
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => ui.setAccent(''),});  }
    }
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", {     "class":`theme-grid`,"role":`radiogroup`,"aria-label":`Backdrop`,});
       for(let m of __sveltets_2_ensureArray(AMBIENT_MODES)){m.id;
       { svelteHTML.createElement("button", {                "class":`theme-card`,"role":`radio`,"aria-checked":ui.ambient === m.id,"tabindex":ui.ambient === m.id ? 0 : -1,"onkeydown":radioKey,...__sveltets_2_empty({"data-ambient-option":m.id}),"onclick":() => ui.setAmbient(m.id),});ui.ambient === m.id;
         { svelteHTML.createElement("div", {  "class":`theme-preview ambient-preview`,});__sveltets_2_ensureType(String, Number, previewOf(m.id));
           { svelteHTML.createElement("span", { "class":`ap-side`,}); }
           { svelteHTML.createElement("span", { "class":`ap-card`,}); }
         }
         { svelteHTML.createElement("div", { "class":`theme-name`,});m.label; }
         { svelteHTML.createElement("div", { "class":`theme-desc`,});m.id === 'wallpaper' && ui.ambientPhoto ? 'Your photo' : m.desc; }
       }
    }
   }
   { svelteHTML.createElement("div", { "class":`row photo-row`,});
     { svelteHTML.createElement("label", {  "class":`btn small`,});photoBusy;
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":12,}});}
      ui.ambientPhoto ? 'Change photo…' : 'Use your own photo…';
       { svelteHTML.createElement("input", {          "type":`file`,"accept":`image/*`,"class":`visually-hidden`,"onchange":pickPhoto,"disabled":photoBusy,});}
     }
    if(ui.ambientPhoto){
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => ui.setAmbientPhoto(null),});  }
    }
   }
   { svelteHTML.createElement("p", { "class":`hint-line`,});
                  
                 
                   
   }
   { svelteHTML.createElement("div", { "class":`toggle-block`,});
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {         "label":`Reduce transparency`,"hint":`Solid sidebar, toolbar and menus with no backdrop. Otto also follows the macOS “Reduce transparency” accessibility setting.`,"checked":ui.reduceTransparency,"onchange":(v) => ui.setReduceTransparency(v),}});}
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Terminal font`,});
       for(let f of __sveltets_2_ensureArray(TERM_FONT_OPTIONS)){f.id;
       { svelteHTML.createElement("button", {       "aria-pressed":ui.termFontFamily === f.id,"title":f.desc,"onclick":() => ui.setTermFontFamily(f.id),});ui.termFontFamily === f.id;f.name; }
    }
   }
   { svelteHTML.createElement("p", { "class":`hint-line`,});
                 
          
   }
  
   { svelteHTML.createElement("div", {     "class":`row term-size-row`,"role":`group`,"aria-label":`Terminal font size`,});
     { svelteHTML.createElement("span", { "class":`term-size-label`,});  }
     { svelteHTML.createElement("button", {         "class":`sb-btn`,"onclick":() => ui.termZoomOut(),"disabled":ui.termFontSize <= 8,"title":`Smaller (⌘− in a terminal)`,"aria-label":`Terminal font smaller`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`minus`,"size":12,}});} }
     { svelteHTML.createElement("span", {   "class":`mono term-size-val`,"aria-live":`polite`,});ui.termFontSize;  }
     { svelteHTML.createElement("button", {         "class":`sb-btn`,"onclick":() => ui.termZoomIn(),"disabled":ui.termFontSize >= 28,"title":`Larger (⌘+ in a terminal)`,"aria-label":`Terminal font larger`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} }
    if(ui.termFontSize !== 13){
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => ui.termZoomReset(),});  }
    }
   }
   { svelteHTML.createElement("div", { "class":`toggle-block`,});
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {         "label":`Copy on select`,"hint":`Selecting text in a terminal copies it to the clipboard.`,"checked":ui.termCopyOnSelect,"onchange":(v) => ui.setTermCopyOnSelect(v),}});}
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {         "label":`Use Option as Meta key`,"hint":`⌥ sends Meta, so ⌥← / ⌥→ jump words and ⌥⌫ deletes a word in shells and agents. Turn it off if your keyboard layout types characters such as @, braces, brackets, | or accents with ⌥.`,"checked":ui.termOptionAsMeta,"onchange":(v) => ui.setTermOptionAsMeta(v),}});}
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {         "label":`Screen reader support`,"hint":`Announces terminal output to VoiceOver and lets you read the screen line by line. Uses the slower non-GPU renderer; toggling reloads open terminals.`,"checked":ui.termScreenReader,"onchange":(v) => ui.setTermScreenReader(v),}});}
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {         "label":`Terminal toolbar`,"hint":`Show the font-size and copy-on-select controls on each terminal.`,"checked":ui.termToolbar,"onchange":(v) => ui.setTermToolbar(v),}});}
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});   { svelteHTML.createElement("span", { "class":`chip exp-tag`,});  } }
   { svelteHTML.createElement("div", { "class":`toggle-block`,});
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },"label":`Right-to-left text in the terminal`,"checked":ui.rtlBidi,"onchange":(v) => ui.setRtlBidi(v),}});
                 
                     
                   
             
     SettingToggle}
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", {     "class":`segmented`,"role":`radiogroup`,"aria-label":`Floating bar`,});
       for(let b of __sveltets_2_ensureArray(barPrefs)){b.id;
       { svelteHTML.createElement("button", {            "role":`radio`,"aria-checked":barStore.pref === b.id,"tabindex":barStore.pref === b.id ? 0 : -1,"onkeydown":radioKey,"onclick":() => barStore.setPref(b.id),});barStore.pref === b.id;b.label; }
    }
   }
   { svelteHTML.createElement("p", { "class":`hint-line`,});
    barPrefs.find((b) => b.id === barStore.pref)?.hint;       
          
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});    }
   { svelteHTML.createElement("div", { "class":`toggle-block`,});
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {           "label":`Isolate sessions to this device`,"hint":`Only show sessions started on this device. Other devices' sessions stay hidden here (they still run on the daemon).`,"checked":ui.sessionIsolation,"onchange":(v) => ui.setSessionIsolation(v),"testid":`session-isolation-toggle`,}});}
   }
   { svelteHTML.createElement("div", { "class":`section-title`,});    }
   { svelteHTML.createElement("p", { "class":`hint-line`,});
                      
                   
                 
             
   }
   { svelteHTML.createElement("div", {     "class":`radio-col`,"role":`radiogroup`,"aria-label":`When closing a session tab`,});
     { svelteHTML.createElement("label", { "class":`switch-row`,});
       { svelteHTML.createElement("input", {        "type":`radio`,"name":`close-tab-pref`,"checked":ui.closeTabPref === '',"onchange":() => ui.setCloseTabPref(''),});}
       { svelteHTML.createElement("span", {});      }
     }
     { svelteHTML.createElement("label", { "class":`switch-row`,});
       { svelteHTML.createElement("input", {        "type":`radio`,"name":`close-tab-pref`,"checked":ui.closeTabPref === 'archive',"onchange":() => ui.setCloseTabPref('archive'),});}
       { svelteHTML.createElement("span", {});           }
     }
     { svelteHTML.createElement("label", { "class":`switch-row`,});
       { svelteHTML.createElement("input", {        "type":`radio`,"name":`close-tab-pref`,"checked":ui.closeTabPref === 'delete',"onchange":() => ui.setCloseTabPref('delete'),});}
       { svelteHTML.createElement("span", {});              }
     }
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("p", { "class":`hint-line`,});
                    
                   
                    
                     
    
   }
   { svelteHTML.createElement("div", {       "class":`av-table`,"role":`group`,"aria-label":`Auto-Vertical by engine`,...__sveltets_2_empty({"data-testid":`db-auto-vertical`}),});
       for(let eng of __sveltets_2_ensureArray(AUTO_VERTICAL_ENGINES)){eng.id;
      const n = ui.dbAutoVertical[eng.id];
       { svelteHTML.createElement("div", { "class":`av-row`,});
         { svelteHTML.createElement("label", { "class":`switch-row av-toggle`,});
           { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":n > 0,"onchange":(e) => ui.setDbAutoVertical(eng.id, e.currentTarget.checked ? 10 : 0),});}
           { svelteHTML.createElement("span", {});eng.label; }
         }
         { svelteHTML.createElement("label", {  "class":`av-num`,});n === 0;
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {                       "type":`number`,"class":`input num-input mono`,"min":`1`,"max":`500`,"step":`1`,"disabled":n === 0,"value":n === 0 ? '' : n,"placeholder":`—`,"oninput":(e) => {
              // A cleared box is mid-edit, not "off" — keep the setting until a number lands.
              const v = e.currentTarget.value;
              if (v !== '' && Number(v) > 0) ui.setDbAutoVertical(eng.id, Number(v));
            },"onchange":(e) => {
              // Leaving the box: show what was actually saved (a cleared box,
              // 0, a decimal or a value past 500 would otherwise keep showing
              // a number that isn’t in effect).
              e.currentTarget.value = String(ui.dbAutoVertical[eng.id] || '');
            },"aria-label":`${eng.label}: column threshold`,});}
           { svelteHTML.createElement("span", {});  }
         }
       }
    }
   }
   { svelteHTML.createElement("p", { "class":`hint-line`,});
                       
    
   }
   { const $$_elggoTgnitteS2C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS2C({ target: __sveltets_2_any(), props: {           "label":`Connect restored tabs on click only`,"hint":`After a reload only the active connection connects; the other tabs wait, marked “Not connected yet”, until you open one. Off: they connect in the background, three at a time.`,"checked":dbPrefs.warmRestored === 'on-click',"onchange":(on) => dbPrefs.setWarmRestored(on ? 'on-click' : 'background'),"testid":`db-warm-on-click`,}});}
   { const $$_elggoTgnitteS2C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS2C({ target: __sveltets_2_any(), props: {           "label":`Keep open connections alive`,"hint":`Pings each open, connected database every 4 minutes so its pool and SSH tunnel don’t close from inactivity, and a dropped one shows red before your next query.`,"checked":dbPrefs.keepAlive,"onchange":(on) => dbPrefs.setKeepAlive(on),"testid":`db-keep-alive`,}});}
   { const $$_elggoTgnitteS2C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS2C({ target: __sveltets_2_any(), props: {           "label":`Clipboard history`,"hint":`Remember the last 50 copies made in Otto (result cells, the query editor) so ⌥⌘V in the query editor can paste them again. Only copies made inside Otto, never from masked tabs. Off clears it.`,"checked":clipHistory.enabled,"onchange":(on) => clipHistory.setEnabled(on),"testid":`clip-history-enabled`,}});}

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("p", { "class":`hint-line`,});
                      
                      
                  
       
   }
   { svelteHTML.createElement("div", {   "class":`sidebar-list`,...__sveltets_2_empty({"data-testid":`settings-sidebar-list`}),});
    
       for(let sec of __sveltets_2_ensureArray(sidebarSecs)){sec.group.id;
      const isFavSec = sec.group.id === FAVORITES_ID;
      const si = sidebarMovableSecs.indexOf(sec.group.id);
       { svelteHTML.createElement("div", { "class":`sidebar-group-label`,});
        if(isFavSec){ { svelteHTML.createElement("span", { "class":`sb-group-star`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":12,}});} }}
         { svelteHTML.createElement("span", { "class":`grow`,});sec.group.label; }
        if(!isFavSec){
           { svelteHTML.createElement("button", {           "class":`sb-btn`,"onclick":() => moveSidebarSection(sec.group.id, -1),"disabled":si <= 0,"title":`Move section up`,"aria-label":`Move ${sec.group.label} section up`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}
           }
           { svelteHTML.createElement("button", {           "class":`sb-btn`,"onclick":() => moveSidebarSection(sec.group.id, 1),"disabled":si >= sidebarMovableSecs.length - 1,"title":`Move section down`,"aria-label":`Move ${sec.group.label} section down`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}
           }
        }
       }
          for(let m of __sveltets_2_ensureArray(sec.modules)){let i = 1;m.id;
        const fav = sidebarFavIds.includes(m.id);
         { svelteHTML.createElement("div", {  "class":`sidebar-row`,});ui.sidebarHidden.includes(m.id);
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":m.icon,"size":14,}});}
           { svelteHTML.createElement("span", { "class":`grow`,});m.label; }
           { svelteHTML.createElement("button", {            "class":`sb-btn sb-star`,"onclick":() => ui.toggleSidebarFavorite(m.id),"title":fav ? 'Remove from Favorites' : 'Add to Favorites',"aria-label":`Favorite ${m.label}`,"aria-pressed":fav,});fav;
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":13,}});}
           }
           { svelteHTML.createElement("button", {           "class":`sb-btn`,"onclick":() => moveSidebarItem(m.id, -1),"disabled":i === 0,"title":`Move up`,"aria-label":`Move ${m.label} up`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}
           }
           { svelteHTML.createElement("button", {           "class":`sb-btn`,"onclick":() => moveSidebarItem(m.id, 1),"disabled":i === sec.modules.length - 1,"title":`Move down`,"aria-label":`Move ${m.label} down`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}
           }
           { svelteHTML.createElement("label", {   "class":`sb-toggle`,"title":ui.sidebarHidden.includes(m.id) ? 'Hidden' : 'Shown',});
             { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":!ui.sidebarHidden.includes(m.id),"onchange":() => ui.toggleSidebarHidden(m.id),"aria-label":`Show ${m.label}`,});}
           }
         }
      }
    }
   }
   { svelteHTML.createElement("div", { "class":`row`,});
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => ui.resetSidebar(),});   }
   }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Appearance__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Appearance__SvelteComponent_ = ReturnType<typeof Appearance__SvelteComponent_>;
/*Ωignore_endΩ*/export default Appearance__SvelteComponent_;