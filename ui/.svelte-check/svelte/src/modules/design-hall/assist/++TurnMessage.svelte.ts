///<reference types="svelte" />
;
import { announceShown } from '../../../lib/announce.svelte';
// One exchange in the Otto thread: what the person asked (when this device
// remembers it, else the committed version's prompt summary) and Otto's
// answer — attributed (provider mark, "Otto", AGENT label, time), its live
// state in words, the one-line summary, the verified citations as chips
// (unverified ones flagged, never clickable), findings with "Fix" (which is
// a NEW turn the person starts), the team rules the turn applied, and — for
// a conflict — the side draft with Compare / Apply on top / Keep current.
// A committed turn is a new version; Compare offers Restore of the previous
// one, so nothing an agent did is ever final without a person.
import Icon, { asIcon } from '../../../lib/components/Icon.svelte';
import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
import AgentChip from '../../../lib/components/AgentChip.svelte';
import LiveWorkingDot from '../../../lib/components/LiveWorkingDot.svelte';
import { rel } from '../../../lib/stores/now.svelte';
import type { DesignAssistTurn, DesignVersion } from '../../../lib/api/types';
import type { Ask } from './asks.svelte';
import { findingsOf, provenanceChips, statusInfo, type Finding, type ProvChip } from './model';
;

  interface Props {
    turn: DesignAssistTurn;
    ask: Ask | null;
    /** The version this turn committed (`turn.version_id`), when loaded. */
    version: DesignVersion | null;
    /** The version it started from (for Compare / seq labels). */
    base: DesignVersion | null;
    headSeq: number | null;
    /** Team-rule key → its text (from the learned rules), when known. */
    ruleText: (key: string) => string | null;
    dismissed: ReadonlySet<string>;
    /** The conflict draft was set aside by the person. */
    keptCurrent: boolean;
    readonly: boolean;
    /** Another turn is running (fix / apply are disabled meanwhile). */
    busy: boolean;
    onopenref: (chip: ProvChip) => void;
    oncompare: (leftVersionId: string, rightVersionId: string) => void;
    onfix: (findings: Finding[], a11y: boolean) => void;
    ondismiss: (f: Finding) => void;
    onapplyconflict: () => void;
    onkeepcurrent: () => void;
    onopensession: (sessionId: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    turn,
    ask,
    version,
    base,
    headSeq,
    ruleText,
    dismissed,
    keptCurrent,
    readonly,
    busy,
    onopenref,
    oncompare,
    onfix,
    ondismiss,
    onapplyconflict,
    onkeepcurrent,
    onopensession,
  }: Props = $props();

  const st = $derived(statusInfo(turn.status, turn.mode));
  const chips = $derived(provenanceChips(turn, version?.provenance ?? null));
  const findings = $derived(findingsOf(turn).filter((f) => !dismissed.has(`${turn.turn_id}:${f.index}`)));
  const open = $derived(findings.filter((f) => !f.fixed));
  const fixedList = $derived(findingsOf(turn).filter((f) => f.fixed));
  const isA11y = $derived(ask?.intent === 'a11y_check' || turn.mode === 'a11y');
  const summary = $derived(
    typeof version?.provenance?.prompt_summary === 'string' ? (version.provenance.prompt_summary as string) : null,
  );
  const asked = $derived(ask ? (ask.display ?? ask.prompt) : summary);
  const modeLabel = $derived(
    ({ generate: 'Generate', refine: 'Refine', critique: 'Review', a11y: 'Accessibility fix', variant: 'Variant' } as Record<string, string>)[
      turn.mode
    ] ?? turn.mode,
  );
  const rules = $derived(
    turn.team_rules.map((k) => ({ key: k, text: (ruleText(k) ?? k.replace(/[_:]/g, ' ')).replace(/[.\s]+$/, '') })),
  );
  const provider = $derived(turn.provider || 'claude');
;
async () => {

 { svelteHTML.createElement("div", {     "class":`exchange`,...__sveltets_2_empty({"data-testid":`design-assist-turn`}),...__sveltets_2_empty({"data-status":turn.status}),});
  if(asked){
     { svelteHTML.createElement("div", { "class":`msg human`,});
       { svelteHTML.createElement("div", { "class":`who`,}); { svelteHTML.createElement("span", {   "class":`av`,"aria-hidden":`true`,});  }  { svelteHTML.createElement("strong", {});  }
        if(ask){ { svelteHTML.createElement("span", {   "class":`when`,"title":new Date(ask.at).toLocaleString(),});rel(ask.at); }} }
      if(ask?.selectionLabel){ { svelteHTML.createElement("span", { "class":`chip ctx`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`target`,"size":12,}});} ask.selectionLabel; }}
       { svelteHTML.createElement("p", {   "class":`bubble`,"title":ask && ask.display && ask.display !== ask.prompt ? ask.prompt : undefined,});asked; }
     }
  }

   { svelteHTML.createElement("div", {   "class":`msg agent`,"aria-live":st.working ? 'polite' : undefined,});
     { svelteHTML.createElement("div", { "class":`who`,});
       { const $$_nocIredivorP3C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP3C({ target: __sveltets_2_any(), props: {   provider,"size":16,}});}
       { svelteHTML.createElement("strong", {});  }
       { const $$_pihCtnegA3C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA3C({ target: __sveltets_2_any(), props: {}});}
       { svelteHTML.createElement("span", {   "class":`when`,"title":new Date(turn.started_at).toLocaleString(),});rel(turn.started_at); }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("span", {   "class":`st tone-${st.tone}`,...__sveltets_2_empty({"data-testid":`design-assist-status`}),});
        if(st.working){ { const $$_toDgnikroWeviL4C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL4C({ target: __sveltets_2_any(), props: {}});}}
        st.label;
       }
     }
     { svelteHTML.createElement("div", { "class":`meta`,});[modeLabel, turn.provider, base ? `from v${base.seq}` : ''].filter(Boolean).join(' · '); }

    if(st.working){
       { svelteHTML.createElement("p", { "class":`body dim`,});
        turn.status === 'starting' ? 'Starting an agent session…' : 'Working on the saved version. Each valid save shows live on the canvas.';
       }
    } else if (turn.status === 'failed'){
       { svelteHTML.createElement("p", { "class":`body err`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} turn.error ?? 'The turn failed.';         }
    } else if (turn.message){
       { svelteHTML.createElement("p", { "class":`body`,});turn.message; }
    }

    if(turn.session_id){
       { svelteHTML.createElement("button", {     "class":`linkbtn`,"onclick":() => onopensession(turn.session_id!),...__sveltets_2_empty({"data-testid":`design-assist-session`}),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});} st.working ? 'View live session' : 'Open session';
       }
    }

    if(turn.status === 'done' && turn.version_id){
       { svelteHTML.createElement("div", { "class":`result`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`commit`,"size":12,}});}
         { svelteHTML.createElement("span", {});   { svelteHTML.createElement("strong", {}); version?.seq ?? '…'; } base ? ` — v${base.seq} stays in history.` : '.'; }
        if(base){
           { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => oncompare(base.id, turn.version_id!),});  }
        }
       }
    }

    if(turn.status === 'conflict' && turn.version_id){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),('Someone saved a newer version while Otto worked, so Otto’s result was kept as a separate draft.')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`conflict`,...__sveltets_2_empty({"data-testid":`design-assist-conflict`}),});
         { svelteHTML.createElement("p", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}   headSeq != null ? `v${headSeq}` : 'a newer version';      
                     }
        if(keptCurrent){
           { svelteHTML.createElement("p", { "class":`dim`,});          }
        }else{
           { svelteHTML.createElement("div", { "class":`row`,});
            if(base){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => oncompare(base.id, turn.version_id!),});  }}
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":readonly || busy,"onclick":onapplyconflict,});   }
             { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":readonly,"onclick":onkeepcurrent,});  }
           }
        }
       }}
    }

    if(open.length){
       { svelteHTML.createElement("ul", {   "class":`findings`,"aria-label":`Findings`,});
           for(let f of __sveltets_2_ensureArray(open)){f.index;
           { svelteHTML.createElement("li", {});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":f.severity === 'error' || f.severity === 'high' || f.severity === 'warning' ? 'warning' : 'info',"size":12,}});}
             { svelteHTML.createElement("span", { "class":`ftext`,});
              if(f.rule){ { svelteHTML.createElement("span", { "class":`rule`,});f.rule; }}
              f.message;if(f.fix){  { svelteHTML.createElement("span", { "class":`dim`,}); f.fix; }}
             }
             { svelteHTML.createElement("button", {         "class":`icon-btn`,"disabled":readonly,"onclick":() => ondismiss(f),"aria-label":`Dismiss this finding`,"title":`Dismiss — Otto notes it`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }
        }
       }
      if(turn.mode === 'critique'){
         { svelteHTML.createElement("button", {       "class":`btn small primary-soft`,"disabled":readonly || busy,"onclick":() => onfix(open, isA11y),...__sveltets_2_empty({"data-testid":`design-assist-fix`}),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});} open.length === 1 ? 'Fix it' : `Fix all ${open.length}`;
         }
      }
    }
    if(fixedList.length && turn.mode !== 'critique'){
       { svelteHTML.createElement("ul", {   "class":`findings fixed`,"aria-label":`Fixed`,});
           for(let f of __sveltets_2_ensureArray(fixedList)){f.index;
           { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`ftext`,});if(f.rule){ { svelteHTML.createElement("span", { "class":`rule`,});f.rule; }}f.message; } }
        }
       }
    }

    if(chips.length){
       { svelteHTML.createElement("div", {   "class":`prov`,"aria-label":`Provenance`,});
         { svelteHTML.createElement("span", { "class":`k`,});  }
           for(let c of __sveltets_2_ensureArray(chips)){c.key;
          if(c.verified && c.artifactId){
             { svelteHTML.createElement("button", {        "class":`chip as-btn`,"title":c.title,"onclick":() => onopenref(c),...__sveltets_2_empty({"data-testid":`design-prov-chip`}),});c.kind === 'brand';
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":asIcon(c.kind === 'brand' ? 'palette' : 'link'),"size":12,}});}  { svelteHTML.createElement("span", { "class":`ct`,});c.label; }
             }
          }else{
             { svelteHTML.createElement("span", {     "class":`chip unverified`,"title":c.title,...__sveltets_2_empty({"data-testid":`design-prov-unverified`}),});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`ct`,});c.label;    }
             }
          }
        }
       }
    }

    if(rules.length){
       { svelteHTML.createElement("p", { "class":`rules`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`bulb`,"size":12,}});}    
            for(let r of __sveltets_2_ensureArray(rules)){let i = 1;r.key; { svelteHTML.createElement("a", { "href":`#/design/learned/rules`,});r.text; }i < rules.length - 1 ? ', ' : '.';}
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TurnMessage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TurnMessage__SvelteComponent_ = ReturnType<typeof TurnMessage__SvelteComponent_>;
/*Ωignore_endΩ*/export default TurnMessage__SvelteComponent_;