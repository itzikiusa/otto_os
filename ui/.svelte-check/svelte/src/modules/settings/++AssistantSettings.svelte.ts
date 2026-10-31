///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
// Settings → Assistant: how Otto Assistant picks provider + model per turn
// (plan §2.4). It never switches silently — rules decide, an explicit pin or
// a leading @claude / @codex always wins, and a usage limit asks first
// unless you turn on automatic switching. One form, one Save.
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import { guardUnsaved } from '../../lib/leaveGuard';
import SettingToggle from './SettingToggle.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { router } from '../../lib/router.svelte';
import { toasts } from '../../lib/toast.svelte';
import { formatCount } from '../../lib/metric-format';
import { assistant, describeError } from '../../lib/stores/assistant.svelte';
import { PROVIDER_NAME, ROUTE_ROWS, loadShare, parseKeywords, type Provider } from '../../lib/assistant/model';
import { whenLabel } from '../assistant/format';
import Badge from '../../lib/components/Badge.svelte';
import type { AssistantRouteKind, AssistantRouteTarget, AssistantRoutingSettings } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  $effect(() => {
    void assistant.loadRouting();
    void assistant.loadLimits();
    void assistant.loadUsageWeek();
    void assistant.loadAccounts();
  });

  const routing = $derived(assistant.routing);
  const PROVIDERS: Provider[] = ['claude', 'codex'];

  // ── the draft form ─────────────────────────────────────────────────────────
  interface Draft {
    targets: Record<AssistantRouteKind, AssistantRouteTarget>;
    code: string;
    hard: string;
    auto_failover: boolean;
    memory_approval: boolean;
  }
  function toDraft(r: AssistantRoutingSettings): Draft {
    return {
      targets: $state.snapshot(r.targets),
      code: r.extra_keywords.code.join(', '),
      hard: r.extra_keywords.hard.join(', '),
      auto_failover: r.auto_failover,
      memory_approval: r.memory_approval,
    };
  }
  let draft = $state<Draft | null>(null);
  let base = $state('');
  // Seed (and re-seed after a save) from the loaded settings.
  $effect(() => {
    const r = routing.data;
    if (!r) return;
    const d = toDraft(r);
    draft = d;
    base = JSON.stringify(d);
  });
  const dirty = $derived(!!draft && JSON.stringify(draft) !== base);
  // Leaving Settings (or this page) with unsaved edits asks first.
  $effect(() => guardUnsaved(() => dirty, { what: 'the routing rules' }));
  let saving = $state(false);
  let saveError = $state('');

  async function save(): Promise<void> {
    if (!draft || !dirty) return;
    saving = true;
    saveError = '';
    try {
      await assistant.saveRouting({
        targets: $state.snapshot(draft.targets),
        extra_keywords: { code: parseKeywords(draft.code), hard: parseKeywords(draft.hard) },
        auto_failover: draft.auto_failover,
        memory_approval: draft.memory_approval,
      });
      // The Save button sits in the header and the form is long — confirm
      // with a toast (like every other Save-model Settings page).
      toasts.success('Routing saved', 'New turns use these rules.');
    } catch (e) {
      saveError = `Couldn’t save routing. ${describeError(e)}`;
      toasts.error('Couldn’t save routing', describeError(e));
    } finally {
      saving = false;
    }
  }
  function setProvider(kind: AssistantRouteKind, p: Provider): void {
    if (!draft) return;
    draft.targets[kind] = { provider: p, model: null, account_id: null };
  }

  // ── subscriptions ──────────────────────────────────────────────────────────
  const share = $derived(loadShare(assistant.usageWeek.data.filter((p) => p.provider === 'claude' || p.provider === 'codex')));
  function accountsFor(p: Provider) {
    return assistant.accounts.data.filter((a) => a.provider === p);
  }
  function limitFor(p: Provider) {
    return assistant.limits.data.find((l) => l.provider === p && l.limited) ?? null;
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (dirty && !saveError) announce('Unsaved changes — Save to apply them.'); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('assistant'),"subtitle":`Route each turn to Claude or Codex`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {     "class":`btn small ghost`,...__sveltets_2_empty({"data-icon":`assistant`}),"onclick":() => router.go('assistant'),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`assistant`,"size":12,}});}   }
      if(routing.data){
         { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => void save(),"disabled":!dirty || saving,});saving ? 'Saving…' : 'Save'; }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
    if(routing.state === 'unsupported'){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`assistant`,"title":`The assistant isn’t available yet`,"body":`This Otto daemon doesn’t include the assistant. Update Otto to configure routing.`,}});}
    } else if (!routing.data){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {            "what":`routing settings`,"loading":routing.state !== 'error',"error":routing.state === 'error' ? routing.error || 'The daemon didn’t answer.' : null,"empty":true,"rows":6,"onretry":() => void assistant.loadRouting(),}});}
    } else if (draft){
       { svelteHTML.createElement("div", {   "class":`form`,...__sveltets_2_empty({"data-testid":`assistant-routing`}),});
        if(saveError){ { svelteHTML.createElement("p", {   "class":`form-note err`,"role":`alert`,});saveError; }} else if (dirty){ { svelteHTML.createElement("p", { "class":`form-note unsaved`,});       }}

         { svelteHTML.createElement("section", {});
           { svelteHTML.createElement("h2", { "class":`section-title`,});  }
           { svelteHTML.createElement("p", { "class":`help`,});                }
           { svelteHTML.createElement("div", { "class":`subs`,});
               for(let p of __sveltets_2_ensureArray(PROVIDERS)){p;
              const lim = limitFor(p);
              const s = share.find((x) => x.provider === p);
               { svelteHTML.createElement("div", {   "class":`sub card`,...__sveltets_2_empty({"data-testid":`sub-${p}`}),});
                 { svelteHTML.createElement("div", { "class":`sub-head`,});
                   { const $$_nocIredivorP7C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP7C({ target: __sveltets_2_any(), props: {    "provider":p,"size":16,}});}
                   { svelteHTML.createElement("strong", {});PROVIDER_NAME[p]; }
                   { svelteHTML.createElement("span", { "class":`push`,});
                    if(lim){ { const $$_egdaB8C = __sveltets_2_ensureComponent(Badge); new $$_egdaB8C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":lim.until ? `Limit until ${whenLabel(lim.until)}` : 'Limit reached',}});}}else{ { const $$_egdaB8C = __sveltets_2_ensureComponent(Badge); new $$_egdaB8C({ target: __sveltets_2_any(), props: {    "tone":`ok`,"label":`No limit hit`,}});}}
                   }
                 }
                 { svelteHTML.createElement("div", {           "class":`meter`,"role":`meter`,"aria-label":`${PROVIDER_NAME[p]} share of this week’s load`,"aria-valuemin":0,"aria-valuemax":100,"aria-valuenow":s?.pct ?? 0,});
                   { svelteHTML.createElement("span", {  });!!lim;__sveltets_2_ensureType(String, Number, `${s?.pct ?? 0} `); }
                 }
                 { svelteHTML.createElement("p", { "class":`sm`,});
                  if(s){s.pct;      formatCount(s.tokens); } else if (assistant.usageWeek.state === 'unsupported'){     } else if (assistant.usageWeek.state === 'loading'){ }else{   }
                  if(lim?.message){ { svelteHTML.createElement("br", {});} { svelteHTML.createElement("span", { "class":`dim`,});lim.message; }}
                 }
                 { svelteHTML.createElement("ul", { "class":`accts`,});
                   { svelteHTML.createElement("li", {}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":12,}});}    }
                     for(let a of __sveltets_2_ensureArray(accountsFor(p))){a.id;
                     { svelteHTML.createElement("li", {});
                       { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":12,}});}
                       { svelteHTML.createElement("span", { "class":`grow`,});a.label; }
                      if(a.signed_in === true){ { svelteHTML.createElement("span", { "class":`dim`,});  }} else if (a.signed_in === false){ { svelteHTML.createElement("span", { "class":`warn-t`,});   }}else{ { svelteHTML.createElement("span", { "class":`dim`,});  }}
                     }
                  }
                 }
               }
            }
           }
         }

         { svelteHTML.createElement("section", {});
           { svelteHTML.createElement("h2", { "class":`section-title`,});  }
           { svelteHTML.createElement("p", { "class":`help`,});
                                 
               { svelteHTML.createElement("span", { "class":`mono`,});  }   { svelteHTML.createElement("span", { "class":`mono`,});  }    
                           
           }
           { svelteHTML.createElement("div", { "class":`table-wrap`,});
             { svelteHTML.createElement("table", { "class":`rules`,});
               { svelteHTML.createElement("thead", {});
                 { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});    } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } }
               }
               { svelteHTML.createElement("tbody", {});
                   for(let row of __sveltets_2_ensureArray(ROUTE_ROWS)){row.kind;
                  const t = draft.targets[row.kind];
                   { svelteHTML.createElement("tr", { ...__sveltets_2_empty({"data-testid":`rule-${row.kind}`}),});
                     { svelteHTML.createElement("td", {});
                       { svelteHTML.createElement("div", { "class":`rule-label`,});row.label; }
                       { svelteHTML.createElement("div", { "class":`dim sm`,});row.hint; }
                     }
                     { svelteHTML.createElement("td", {});
                       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Provider for ${row.label}`,});
                           for(let p of __sveltets_2_ensureArray(PROVIDERS)){p;
                           { svelteHTML.createElement("button", {      "type":`button`,"aria-pressed":t.provider === p,"onclick":() => setProvider(row.kind, p),});t.provider === p;
                             { const $$_nocIredivorP11C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP11C({ target: __sveltets_2_any(), props: {    "provider":p,"size":12,}});}PROVIDER_NAME[p];
                           }
                        }
                       }
                     }
                     { svelteHTML.createElement("td", { "class":`model-cell`,});
                      t.provider; {
                         { const $$_rekciPledoM9C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM9C({ target: __sveltets_2_any(), props: {          "provider":t.provider,"value":t.model ?? '',"id":`route-model-${row.kind}`,"compact":true,"onchange":(m) => {
                            if (draft) draft.targets[row.kind] = { ...draft.targets[row.kind], model: m.trim() || null };
                          },}});}
                      }
                     }
                   }
                }
               }
             }
           }
           { svelteHTML.createElement("div", { "class":`kw`,});
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`kw-code`,});     }
               { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`kw-code`,"class":`input`,"bind:value":draft.code,"placeholder":`terraform, jq, regex`,});/*Ωignore_startΩ*/() => draft.code = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`kw-hard`,});      }
               { svelteHTML.createElement("input", {          "dir":`auto`,"id":`kw-hard`,"class":`input`,"bind:value":draft.hard,"placeholder":`deep dive, audit, proof`,});/*Ωignore_startΩ*/() => draft.hard = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
           }
           { svelteHTML.createElement("p", { "class":`help`,});       }
         }

         { svelteHTML.createElement("section", {});
           { svelteHTML.createElement("h2", { "class":`section-title`,});     }
           { svelteHTML.createElement("div", {     "class":`card set-card radios`,"role":`radiogroup`,"aria-label":`When a usage limit is reached`,});
             { svelteHTML.createElement("label", { "class":`radio`,});
               { svelteHTML.createElement("input", {        "type":`radio`,"name":`failover`,"checked":!draft.auto_failover,"onchange":() => { if (draft) draft.auto_failover = false; },});}
               { svelteHTML.createElement("span", {});
                 { svelteHTML.createElement("span", { "class":`opt`,});    }
                 { svelteHTML.createElement("span", { "class":`dim sm`,});                   }
               }
             }
             { svelteHTML.createElement("label", { "class":`radio`,});
               { svelteHTML.createElement("input", {        "type":`radio`,"name":`failover`,"checked":draft.auto_failover,"onchange":() => { if (draft) draft.auto_failover = true; },});}
               { svelteHTML.createElement("span", {});
                 { svelteHTML.createElement("span", { "class":`opt`,});  }
                 { svelteHTML.createElement("span", { "class":`dim sm`,});              }
               }
             }
             { svelteHTML.createElement("p", { "class":`note`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}                          }
           }
         }

         { svelteHTML.createElement("section", {});
           { svelteHTML.createElement("h2", { "class":`section-title`,});  }
           { svelteHTML.createElement("div", { "class":`card set-card`,});
           { const $$_elggoTgnitteS5C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS5C({ target: __sveltets_2_any(), props: {         "label":`Review memories before Otto keeps them`,"hint":`Off: Otto saves what matters and shows a chip with Undo. On: suggestions wait on the Memory tab until you accept them.`,"checked":draft.memory_approval,"onchange":(v) => { if (draft) draft.memory_approval = v; },}});}
           }
         }
       }
    }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AssistantSettings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AssistantSettings__SvelteComponent_ = ReturnType<typeof AssistantSettings__SvelteComponent_>;
/*Ωignore_endΩ*/export default AssistantSettings__SvelteComponent_;