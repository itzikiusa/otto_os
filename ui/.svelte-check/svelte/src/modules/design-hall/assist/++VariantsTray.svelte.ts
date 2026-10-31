///<reference types="svelte" />
;// One variants run as equal cards (patterns.md §3): a live sandboxed preview
// of each variant version, its direction name and one line on how it
// differs, then the same three actions on every card — Apply (fast-forward
// main; the person's explicit pick), Compare (against the current version)
// and Reject, which asks why (the reason feeds the learning loop). Nothing
// is pre-applied: the head stays where it is until a person applies one.

import Icon from '../../../lib/components/Icon.svelte';
import { announceEdge } from '../../../lib/announce.svelte';
import { plural } from '../../../lib/plural';
import type { DesignArtifact, DesignVariantRun } from '../../../lib/api/types';
import ArtifactThumb from '../ArtifactThumb.svelte';
import { directionBlurb, directionName, variantCards, type VariantCard } from './model';
;

  interface Props {
    artifact: DesignArtifact;
    run: DesignVariantRun;
    /** version id → the reason chip a person rejected it with. */
    rejected: Record<string, string>;
    /** The variant version being applied right now. */
    applying: string | null;
    readonly: boolean;
    onapply: (card: VariantCard) => void;
    oncompare: (card: VariantCard) => void;
    onreject: (e: MouseEvent, card: VariantCard) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { artifact, run, rejected, applying, readonly, onapply, oncompare, onreject }: Props = $props();

  const cards = $derived(variantCards(run));
  const letter = (k: number) => String.fromCharCode(64 + Math.min(26, Math.max(1, k)));
  const decided = $derived(!!run.accepted_version_id);

  // The cards' “Drawing…” lines are plain text (inserted with their card, a
  // role would be silent); the tray speaks the run's progress at its edges
  // instead — once while drawing, once when the last card lands. A run that
  // opens already finished says nothing.
  const drawing = $derived(cards.filter((c) => !c.version && c.state === 'running').length);
  const drawn = $derived(cards.filter((c) => !!c.version).length);
  const sayProgress = announceEdge();
  const seen = { drawing: false };
  /** The run's progress line for a listener (null: nothing to say yet). */
  function variantProgress(nDrawing: number, nDrawn: number, s: { drawing: boolean }): string | null {
    if (nDrawing > 0) {
      s.drawing = true;
      return `Drawing ${plural(nDrawing, 'variant')}…`;
    }
    return s.drawing ? `${plural(nDrawn, 'variant')} ready to compare` : null;
  }
  $effect(() => sayProgress(variantProgress(drawing, drawn, seen)));
;
async () => {

 { svelteHTML.createElement("div", {       "class":`tray`,"role":`group`,"aria-label":`Variants`,...__sveltets_2_empty({"data-testid":`design-variants-tray`}),});
     for(let c of __sveltets_2_ensureArray(cards)){c.k;
    const rej = c.version ? rejected[c.version.id] : undefined;
    const name = directionName(c.direction);
     { svelteHTML.createElement("article", {           "class":`card`,"aria-label":`Variant ${letter(c.k)}: ${name}`,...__sveltets_2_empty({"data-testid":`design-variant-card`}),...__sveltets_2_empty({"data-state":c.state}),});c.state === 'accepted';c.state === 'passed' || !!rej;
       { svelteHTML.createElement("div", { "class":`pic`,});
         { svelteHTML.createElement("span", {   "class":`k`,"aria-hidden":`true`,});letter(c.k); }
        if(c.version){
            { const $$_bmuhTtcafitrA3C = __sveltets_2_ensureComponent(ArtifactThumb); new $$_bmuhTtcafitrA3C({ target: __sveltets_2_any(), props: {   artifact,"versionId":c.version.id,"live":true,"compact":true,}});}
        } else if (c.state === 'running'){
           { svelteHTML.createElement("div", { "class":`drawing`,}); { svelteHTML.createElement("span", {   "class":`pulse`,"aria-hidden":`true`,}); }  }
        }else{
           { svelteHTML.createElement("div", { "class":`drawing failed`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}   }
        }
       }
       { svelteHTML.createElement("div", { "class":`body`,});
         { svelteHTML.createElement("span", { "class":`name`,});name; }
         { svelteHTML.createElement("span", {   "class":`blurb`,"title":c.summary ?? directionBlurb(c.direction),});c.summary ?? directionBlurb(c.direction); }
        if(c.state === 'accepted'){
           { svelteHTML.createElement("span", { "class":`state ok`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
        } else if (rej){
           { svelteHTML.createElement("span", { "class":`state`,});  rej; }
        } else if (c.state === 'passed'){
           { svelteHTML.createElement("span", { "class":`state`,});  }
        } else if (c.state === 'failed'){
           { svelteHTML.createElement("span", { "class":`state bad`,});c.turn?.error ? 'Failed' : 'Couldn’t draw this one'; }
        }
       }
       { svelteHTML.createElement("div", { "class":`acts`,});
         { svelteHTML.createElement("button", {           "class":`btn small`,"disabled":readonly || decided || !c.version || applying !== null,"onclick":() => onapply(c),"title":decided ? 'A variant of this set was already applied' : `Apply ${name} as a new version`,...__sveltets_2_empty({"data-testid":`design-variant-apply`}),});
          applying && applying === c.version?.id ? 'Applying…' : 'Apply';
         }
         { svelteHTML.createElement("button", {           "class":`icon-btn`,"disabled":!c.version,"onclick":() => oncompare(c),"aria-label":`Compare ${name} with the current version`,"title":`Compare with the current version`,...__sveltets_2_empty({"data-testid":`design-variant-compare`}),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`columns`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {             "class":`icon-btn`,"disabled":readonly || decided || !c.version || !!rej,"onclick":(e) => onreject(e, c),"aria-label":`Reject ${name} — tell Otto why`,"title":`Not this one — tell Otto why`,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`design-variant-reject`}),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
       }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const VariantsTray__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type VariantsTray__SvelteComponent_ = ReturnType<typeof VariantsTray__SvelteComponent_>;
/*Ωignore_endΩ*/export default VariantsTray__SvelteComponent_;