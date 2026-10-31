///<reference types="svelte" />
;
import PageHeader from '../../lib/components/PageHeader.svelte';
import { focusOnMount } from '../../lib/focusOnMount';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
// Settings → Snipping: the system-wide capture shortcut (desktop app only —
// the chord is registered by the Tauri shell via tauri-plugin-global-shortcut
// and persisted in the app config dir, so it works while Otto runs in the
// background). In a plain browser only the in-app triggers exist.
import { isTauri } from '../../lib/stores/ui.svelte';
import { toasts } from '../../lib/toast.svelte';
import { startSnip } from '../../lib/snip';
import SectionIntro from './SectionIntro.svelte';
import Icon from '../../lib/components/Icon.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  const DEFAULT_ACCEL = 'Cmd+Ctrl+Shift+2';

  let accel = $state('');
  let loading = $state(true);
  let recording = $state(false);
  let saveError = $state('');
  /** The chord being saved ('' = turning it off); null when idle. Every
   *  control disables meanwhile and the one that fired reads "…ing". */
  let saving = $state<string | null>(null);
  // The shell couldn't report the chord (an older app build): '' would read
  // as "disabled" and offer Reset/Disable against a value we never saw.
  let unavailable = $state(false);

  $effect(() => {
    if (!isTauri) {
      loading = false;
      return;
    }
    void (async () => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        accel = await invoke<string>('snip_get_shortcut');
      } catch {
        // Older shell without the command; leave the section read-only.
        unavailable = true;
      } finally {
        loading = false;
      }
    })();
  });

  async function save(next: string): Promise<void> {
    if (saving !== null) return;
    saveError = '';
    saving = next;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('snip_set_shortcut', { accel: next });
      accel = next;
      toasts.success(next ? `Snip shortcut set to ${pretty(next)}` : 'Snip shortcut disabled');
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = null;
    }
  }

  function onRecordKey(e: KeyboardEvent): void {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape') {
      recording = false;
      return;
    }
    // Wait for a non-modifier key; require at least one modifier so a bare
    // letter can't shadow normal typing system-wide.
    if (['Meta', 'Control', 'Alt', 'Shift'].includes(e.key)) return;
    const mods = [
      e.metaKey ? 'Cmd' : null,
      e.ctrlKey ? 'Ctrl' : null,
      e.altKey ? 'Alt' : null,
      e.shiftKey ? 'Shift' : null,
    ].filter(Boolean) as string[];
    if (!mods.length) return;
    // The PHYSICAL key: with ⇧ or ⌥ held, `e.key` is the shifted/option
    // character ("@", "™"), which the shell can't parse as a chord.
    const c = e.code;
    let key = /^Key[A-Z]$/.test(c)
      ? c.slice(3)
      : /^Digit[0-9]$/.test(c)
        ? c.slice(5)
        : /^F\d{1,2}$/.test(c)
          ? c
          : e.key.length === 1
            ? e.key.toUpperCase()
            : e.key;
    if (key === ' ' || c === 'Space') key = 'Space';
    recording = false;
    void save([...mods, key].join('+'));
  }

  /** "Cmd+Ctrl+Shift+2" → "⌘⌃⇧2" for display. */
  function pretty(a: string): string {
    if (!a) return '—';
    return a
      .split('+')
      .map((part) => ({ Cmd: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧' })[part] ?? part)
      .join('');
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('snipping'),"subtitle":`One-gesture screenshots: capture, annotate, paste`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});               { svelteHTML.createElement("strong", {});    }            SectionIntro}

  if(isTauri){
     { svelteHTML.createElement("div", { "class":`card snip-card`,});
       { svelteHTML.createElement("div", { "class":`snip-row`,});
         { svelteHTML.createElement("div", { "class":`row-text`,});
           { svelteHTML.createElement("span", { "class":`row-title`,});  }
           { svelteHTML.createElement("span", { "class":`row-desc`,});
                        
                        
            
           }
         }
         { svelteHTML.createElement("div", { "class":`row-controls`,});
          if(recording){
             {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {              "dir":`auto`,"class":`input recorder`,"readonly":true,"aria-label":`Press the new shortcut`,"placeholder":`Press keys… (Esc cancels)`,"onkeydown":onRecordKey,"onblur":() => (recording = false),});}}
          } else if (unavailable){
             { svelteHTML.createElement("span", { "class":`row-desc`,});            }
          }else{
             { svelteHTML.createElement("kbd", {     "class":`chord`,...__sveltets_2_empty({"data-accel":accel}),"title":accel || 'No shortcut',});loading ? '…' : accel ? pretty(accel) : 'Off'; }
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":saving !== null,"onclick":() => (recording = true),});
              saving !== null && saving !== '' && saving !== DEFAULT_ACCEL ? 'Saving…' : accel ? 'Change…' : 'Set…';
             }
            if(accel !== DEFAULT_ACCEL){
               { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":saving !== null,"onclick":() => void save(DEFAULT_ACCEL),});
                saving === DEFAULT_ACCEL ? 'Resetting…' : `Reset to ${pretty(DEFAULT_ACCEL)}`;
               }
            }
            if(accel){
               { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":saving !== null,"onclick":() => void save(''),});saving === '' ? 'Turning off…' : 'Turn off'; }
            }
          }
         }
       }
      if(saveError){
         { svelteHTML.createElement("div", {   "class":`error`,"role":`alert`,});    saveError;             }
      }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`card snip-card`,});
       { svelteHTML.createElement("div", { "class":`row-text`,});
         { svelteHTML.createElement("span", { "class":`row-title`,});  }
         { svelteHTML.createElement("span", { "class":`row-desc`,});
                       
           
         }
       }
     }
  }

   { svelteHTML.createElement("div", { "class":`card snip-card`,});
     { svelteHTML.createElement("div", { "class":`snip-row`,});
       { svelteHTML.createElement("div", { "class":`row-text`,});
         { svelteHTML.createElement("span", { "class":`row-title`,});  }
         { svelteHTML.createElement("span", { "class":`row-desc`,});
                         
                   
           { svelteHTML.createElement("span", { "class":`mono`,});  }      
         }
       }
       { svelteHTML.createElement("div", { "class":`row-controls`,});
         { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => void startSnip(),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":13,}});}    }
       }
     }
   }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SnipSettings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SnipSettings__SvelteComponent_ = ReturnType<typeof SnipSettings__SvelteComponent_>;
/*Ωignore_endΩ*/export default SnipSettings__SvelteComponent_;