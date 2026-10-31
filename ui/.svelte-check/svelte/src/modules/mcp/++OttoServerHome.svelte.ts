///<reference types="svelte" />
;// Otto's built-in MCP server is the control plane's home: session attachment,
// the outward catalog, what sessions see, and external-client setup.

import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import { mcpCpApi } from '../../lib/api/mcp';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyWithToast } from '../../lib/clipboard';
import { confirmer } from '../../lib/confirm.svelte';
import { mcpCpExtraApi, type McpGatewayToolRow } from './cp-api';
import ExposePanel from './ExposePanel.svelte';
import AutoApprovePanel from './AutoApprovePanel.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { attachView, builtInToolsLine, gatewayView } from './sessionAttach';
import { loadErrorText } from '../../lib/loadError';
import type {
    McpAutoApproveRef,
    McpOttoServerStatus,
    McpOttoToolInfo,
    McpSessionAttach,
  } from '../../lib/api/types';
;

  interface Props {
    wsId: string | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  


  let { wsId }: Props = $props();

  let status = $state<McpOttoServerStatus | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let loadError = $state<string | null>(null);
  let filter = $state('');
  let attach = $state<McpSessionAttach | null>(null);
  let attachBusy = $state(false);
  let attachGeneration = 0;
  let attachError = $state<string | null>(null);
  const attachState = $derived(attachView(wsId, attach, attachError));
  let gatewayTools = $state<McpGatewayToolRow[]>([]);
  let gatewayLoading = $state(false);
  let gatewayError = $state<string | null>(null);
  let exposeOpen = $state(false);
  /** Bumped when the catalog switches change rules — the rules panel reloads. */
  let rulesRevision = $state(0);

  const isMcpAdmin = $derived(auth.can('mcp', 'admin'));
  const tools = $derived(status?.tools ?? []);
  const enabledNames = $derived(new Set(tools.filter((tool) => tool.enabled).map((tool) => tool.name)));
  const mutatingCount = $derived(tools.filter((tool) => tool.mutating).length);
  // Global `mcp_require_approval_dangerous` — when off, nothing asks at all.
  const approvalsOn = $derived(status?.require_approval_dangerous ?? true);
  // Enabled mutating tools some auto-approve rule (any scope) covers.
  const skippedCount = $derived(
    tools.filter((tool) => tool.enabled && tool.mutating && (tool.auto_approved_by?.length ?? 0) > 0).length,
  );
  const gatewayNames = $derived([...new Set(gatewayTools.map((tool) => tool.name))]);
  const shownGatewayNames = $derived(gatewayNames.slice(0, 60));
  const gatewayState = $derived(gatewayView(wsId, gatewayLoading, gatewayError, gatewayNames.length));

  const filtered = $derived.by(() => {
    const query = filter.trim().toLowerCase();
    return query
      ? tools.filter((tool) => `${tool.name} ${tool.description}`.toLowerCase().includes(query))
      : tools;
  });
  function groupByCategory(list: McpOttoToolInfo[]): { cat: string; tools: McpOttoToolInfo[] }[] {
    const order: string[] = [];
    const grouped = new Map<string, McpOttoToolInfo[]>();
    for (const tool of list) {
      const category = tool.category || 'Other';
      if (!grouped.has(category)) {
        grouped.set(category, []);
        order.push(category);
      }
      grouped.get(category)!.push(tool);
    }
    return order.map((cat) => ({ cat, tools: grouped.get(cat)! }));
  }
  /** What the list shows (narrowed by the search box). */
  const groups = $derived(groupByCategory(filtered));
  /** Every tool, unfiltered: the token picker and the category All/None act on
   *  the WHOLE category — the search box used to silently narrow both. */
  const allGroups = $derived(groupByCategory(tools));
  const categoryTools = (cat: string) => allGroups.find((g) => g.cat === cat)?.tools ?? [];

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      status = await mcpCpApi.cpOttoServer();
    } catch (e) {
      status = null;
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  /** Resolves false when the save failed. The checkboxes here are one-way
   *  (`checked={…}`), so a failed save must put the clicked box back itself —
   *  otherwise it keeps showing a state the daemon never stored. */
  async function patch(body: Parameters<typeof mcpCpApi.cpUpdateOttoServer>[0]): Promise<boolean> {
    saving = true;
    try {
      const next = await mcpCpApi.cpUpdateOttoServer(body);
      status = { ...next, token: null };
      return true;
    } catch (e) {
      toastError('Couldn’t update the setting', e);
      return false;
    } finally {
      saving = false;
    }
  }

  async function toggleEnabled(input: HTMLInputElement): Promise<void> {
    if (!(await patch({ enabled: !(status?.enabled ?? false) }))) input.checked = status?.enabled ?? false;
  }

  async function toggleTool(name: string, input: HTMLInputElement): Promise<void> {
    const next = new Set(enabledNames);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    if (!(await patch({ tools: [...next] }))) input.checked = enabledNames.has(name);
  }

  async function setCategory(categoryTools: McpOttoToolInfo[], enable: boolean): Promise<void> {
    const next = new Set(enabledNames);
    for (const tool of categoryTools) {
      if (enable) next.add(tool.name);
      else next.delete(tool.name);
    }
    await patch({ tools: [...next] });
  }

  /** Mutating tools that are enabled — the only ones an approval applies to. */
  function gatedTools(list: McpOttoToolInfo[]): McpOttoToolInfo[] {
    return list.filter((tool) => tool.mutating && tool.enabled);
  }

  /** The GLOBAL per-tool rule behind a tool's "Auto-approve everywhere" switch. */
  function globalToolRule(tool: McpOttoToolInfo): McpAutoApproveRef | undefined {
    return tool.auto_approved_by?.find((r) => r.scope === 'global' && r.target_kind === 'tool');
  }

  /** The GLOBAL category rule a category's switch reflects (read off any tool
   *  of the category it covers). */
  function globalCategoryRule(category: string, list: McpOttoToolInfo[]): McpAutoApproveRef | undefined {
    for (const tool of list) {
      const rule = tool.auto_approved_by?.find(
        (r) => r.scope === 'global' && r.target_kind === 'category' && r.target === category,
      );
      if (rule) return rule;
    }
    return undefined;
  }

  /** Rules other than the tool's own global switch — shown as badges. */
  function otherRules(tool: McpOttoToolInfo): McpAutoApproveRef[] {
    const own = globalToolRule(tool);
    return (tool.auto_approved_by ?? []).filter((r) => r.id !== own?.id);
  }

  function ruleBadge(rule: McpAutoApproveRef): string {
    const where = rule.scope === 'global' ? 'everywhere' : rule.scope === 'workspace' ? 'in a workspace' : 'for one session';
    return rule.target_kind === 'category' ? `${rule.target} writes ${where}` : `${where}`;
  }

  async function confirmAuto(what: string, irreversible: boolean): Promise<boolean> {
    const first = await confirmer.ask(
      `Auto-approve ${what} everywhere? Agents and external clients will run it without a human approval. Every call is still audited as “Auto approved”, and you can turn this off here at any time.`,
      { title: 'Auto-approve', confirmLabel: 'Auto-approve', danger: true },
    );
    if (!first || !irreversible) return first;
    // The guardrail's second explicit toggle.
    return confirmer.ask(
      `${what} is IRREVERSIBLE — nobody can undo it from Otto once it runs. Run it without asking anyway?`,
      { title: 'Irreversible tool', confirmLabel: 'Run without asking', danger: true },
    );
  }

  async function afterRuleChange(): Promise<void> {
    rulesRevision++;
    await load();
  }

  /** The per-tool "Auto-approve everywhere" switch: a global per-tool rule. A
   *  cancelled confirm or failed save restores the checkbox. */
  async function setAuto(tool: McpOttoToolInfo, on: boolean, input: HTMLInputElement): Promise<void> {
    const rule = globalToolRule(tool);
    if (on && !(await confirmAuto(tool.name, !!tool.irreversible))) {
      input.checked = false;
      return;
    }
    saving = true;
    try {
      if (on) {
        await mcpCpExtraApi.createAutoApprove({
          scope: 'global',
          target_kind: 'tool',
          target: tool.name,
          allow_irreversible: tool.irreversible ? true : undefined,
        });
      } else if (rule) {
        await mcpCpExtraApi.deleteAutoApprove(rule.id);
      }
      await afterRuleChange();
    } catch (e) {
      input.checked = !!rule;
      toastError('Couldn’t update the setting', e);
    } finally {
      saving = false;
    }
  }

  /** "Auto-approve every <category> write": one global category rule —
   *  covers future tools of the category, never an irreversible one. */
  async function setCategoryAuto(category: string, list: McpOttoToolInfo[], on: boolean, input: HTMLInputElement): Promise<void> {
    const rule = globalCategoryRule(category, list);
    if (on && !(await confirmAuto(`every ${category} write (except irreversible tools)`, false))) {
      input.checked = false;
      return;
    }
    saving = true;
    try {
      if (on) await mcpCpExtraApi.createAutoApprove({ scope: 'global', target_kind: 'category', target: category });
      else if (rule) await mcpCpExtraApi.deleteAutoApprove(rule.id);
      await afterRuleChange();
    } catch (e) {
      input.checked = !!rule;
      toastError('Couldn’t update the setting', e);
    } finally {
      saving = false;
    }
  }

  async function loadAttach(id: string): Promise<void> {
    const generation = ++attachGeneration;
    attach = null;
    attachError = null;
    try {
      const next = await mcpCpExtraApi.sessionAttach(id);
      if (wsId === id && generation === attachGeneration) attach = next;
    } catch (e) {
      if (wsId === id && generation === attachGeneration) attachError = loadErrorText(e);
    }
  }

  async function updateAttach(enabled: boolean, input: HTMLInputElement): Promise<void> {
    const id = wsId;
    if (!id || !attach || attachBusy) return;
    const reported = attach.attached; // the box goes back to it if the save fails
    const generation = ++attachGeneration;
    attachBusy = true;
    attachError = null;
    try {
      const next = await mcpCpExtraApi.setSessionAttach(id, { enabled });
      if (wsId !== id || generation !== attachGeneration) return;
      attach = next;
      toasts.success(
        enabled ? 'Attached to sessions' : 'Detached from sessions',
        'Applies to sessions started from now on.',
      );
    } catch (e) {
      if (wsId !== id || generation !== attachGeneration) return;
      // Put the box back: the daemon still has the old value.
      input.checked = reported;
      toastError(enabled ? 'Couldn’t attach to sessions' : 'Couldn’t detach from sessions', e);
    } finally {
      if (wsId === id && generation === attachGeneration) attachBusy = false;
    }
  }

  async function loadGatewayTools(id: string): Promise<void> {
    gatewayLoading = true;
    gatewayError = null;
    try {
      const result = await mcpCpExtraApi.gatewayTools(id);
      if (wsId === id) gatewayTools = result.tools;
    } catch (e) {
      if (wsId === id) {
        gatewayTools = [];
        gatewayError = loadErrorText(e);
      }
    } finally {
      if (wsId === id) gatewayLoading = false;
    }
  }

  function copyGatewayTools(): void {
    void copyWithToast(gatewayNames.join('\n'), 'tool names');
  }

  $effect(() => {
    void load();
  });

  $effect(() => {
    const id = wsId;
    attachGeneration++;
    attachBusy = false;
    attach = null;
    attachError = null;
    gatewayTools = [];
    gatewayError = null;
    gatewayLoading = false;
    if (id) {
      void loadAttach(id);
      void loadGatewayTools(id);
    }
  });

  /** Tool descriptions are markdown-ish: render `inline code` spans as code
   *  instead of showing raw backticks. */
  function descParts(text: string | null | undefined): { text: string; code: boolean }[] {
    return (text ?? '').split('`').map((t, i) => ({ text: t, code: i % 2 === 1 })).filter((p) => p.text !== '');
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`otto`,});
   { svelteHTML.createElement("section", { "class":`hero`,});
     { svelteHTML.createElement("div", { "class":`hl-title`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plug`,"size":16,}});}
       { svelteHTML.createElement("span", {});    }
     }

    if(attachState === 'error'){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`the session-attach setting`,"variant":`compact`,"error":attachError,"empty":true,"onretry":() => wsId && void loadAttach(wsId),}});}
    } else if (attachState === 'loading'){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {      "what":`the session-attach setting`,"variant":`compact`,"loading":true,"empty":true,"rows":1,}});}
    } else if (attachState === 'ready' && attach){
      wsId; {
       { svelteHTML.createElement("label", { "class":`switchrow`,});
         { svelteHTML.createElement("input", {           "type":`checkbox`,...__sveltets_2_empty({"data-testid":`mcp-session-attach`}),"checked":attach.attached,"disabled":attachBusy || !isMcpAdmin,"onchange":(event) => void updateAttach(event.currentTarget.checked, event.currentTarget),});}
         { svelteHTML.createElement("span", { "class":`switchcopy`,});
           { svelteHTML.createElement("strong", {});    ws.current?.name ?? 'this workspace'; }
           { svelteHTML.createElement("span", { "class":`muted small`,});
                             
           }
         }
       }
      }
    }else{
       { svelteHTML.createElement("p", { "class":`muted small`,});       }
    }

     { svelteHTML.createElement("label", { "class":`switchrow`,});
       { svelteHTML.createElement("input", {           "type":`checkbox`,...__sveltets_2_empty({"data-testid":`mcp-outward-enabled`}),"checked":status?.enabled ?? false,"disabled":saving || !status || !isMcpAdmin,"onchange":(event) => void toggleEnabled(event.currentTarget),});}
       { svelteHTML.createElement("span", { "class":`switchcopy`,});
         { svelteHTML.createElement("strong", {});    }
         { svelteHTML.createElement("span", { "class":`muted small`,});
                                         
         }
       }
     }
    if(status){ { svelteHTML.createElement("p", { "class":`counts muted small`,});
      tools.length;   enabledNames.size;   mutatingCount; if(skippedCount){' ';  { svelteHTML.createElement("span", { "class":`noask`,});skippedCount;  }}
     }}
   }

   { const $$_lenaPevorppAotuA1C = __sveltets_2_ensureComponent(AutoApprovePanel); new $$_lenaPevorppAotuA1C({ target: __sveltets_2_any(), props: {     isMcpAdmin,"revision":rulesRevision,"onchange":() => void load(),}});}

  if((loading || loadError) && !status){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`the external tool catalog`,loading,"error":loadError,"empty":true,"rows":3,"onretry":() => void load(),}});}
  }

   { svelteHTML.createElement("div", { "class":`tools-head`,});
     { svelteHTML.createElement("div", {});
       { svelteHTML.createElement("h4", { "class":`sec`,});   }
       { svelteHTML.createElement("p", { "class":`muted small catalog-note`,});
                      
            { svelteHTML.createElement("code", {});  }         
               { svelteHTML.createElement("em", {});  }      
                
       }
     }
     { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input filter`,"type":`search`,"placeholder":`Filter tools…`,"bind:value":filter,"aria-label":`Filter tools`,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }
  if(status && !approvalsOn){
     { svelteHTML.createElement("p", {   "class":`warn`,...__sveltets_2_empty({"data-testid":`mcp-approvals-globally-off`}),});
             { svelteHTML.createElement("code", {});  }  
              { svelteHTML.createElement("em", {});  }       
        
     }
  }
     for(let group of __sveltets_2_ensureArray(groups)){group.cat;
    const gated = gatedTools(group.tools);
     { svelteHTML.createElement("div", { "class":`grp`,});
       { svelteHTML.createElement("div", { "class":`grp-head`,});
         { svelteHTML.createElement("span", { "class":`grp-name`,});group.cat; }
         { svelteHTML.createElement("span", { "class":`grp-count muted`,});
          group.tools.filter((tool) => tool.enabled).length; group.tools.length;
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":saving || !status || !isMcpAdmin,"onclick":() => void setCategory(categoryTools(group.cat), true),});  }
         { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":saving || !status || !isMcpAdmin,"onclick":() => void setCategory(categoryTools(group.cat), false),});  }
       }
      if(gated.length){
        const catRule = globalCategoryRule(group.cat, group.tools);
         { svelteHTML.createElement("label", {   "class":`grp-ask`,"title":`One rule for the category: covers its mutating tools (and future ones), never an irreversible tool.`,});
           { svelteHTML.createElement("input", {           "type":`checkbox`,...__sveltets_2_empty({"data-testid":`mcp-category-auto`}),"checked":!!catRule,"disabled":saving || !status || !isMcpAdmin || !approvalsOn,"onchange":(event) => void setCategoryAuto(group.cat, group.tools, event.currentTarget.checked, event.currentTarget),});}
           { svelteHTML.createElement("span", {});  group.cat;  }
          if(group.tools.some((tool) => tool.mutating && tool.irreversible)){
             { svelteHTML.createElement("span", { "class":`muted`,});    }
          }
         }
      }
       { svelteHTML.createElement("div", { "class":`tool-list`,});
           for(let tool of __sveltets_2_ensureArray(group.tools)){tool.name;
           { svelteHTML.createElement("div", { "class":`tool`,});
             { svelteHTML.createElement("label", { "class":`tool-main`,});
               { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":tool.enabled,"disabled":saving || !status || !isMcpAdmin,"onchange":(event) => void toggleTool(tool.name, event.currentTarget),});}
               { svelteHTML.createElement("span", { "class":`t-meta`,});
                 { svelteHTML.createElement("span", { "class":`t-name mono`,});
                  tool.name;if(tool.mutating){ { svelteHTML.createElement("span", { "class":`mut`,});  }}
                 }
                 { svelteHTML.createElement("span", { "class":`t-desc`,});    for(let part of __sveltets_2_ensureArray(descParts(tool.description))){let i = 1;i;if(part.code){ { svelteHTML.createElement("code", {});part.text; }}else{part.text;}} }
               }
             }
            if(tool.mutating && tool.enabled){
              const own = globalToolRule(tool);
               { svelteHTML.createElement("div", { "class":`ask`,});
                 { svelteHTML.createElement("label", {     "class":`ask-toggle`,"title":`On: calls run without asking a person — still audited. Off: a human approves each call.`,});
                   { svelteHTML.createElement("input", {           "type":`checkbox`,...__sveltets_2_empty({"data-testid":`mcp-auto-${tool.name}`}),"checked":!!own,"disabled":saving || !status || !isMcpAdmin,"onchange":(event) => void setAuto(tool, event.currentTarget.checked, event.currentTarget),});}
                   { svelteHTML.createElement("span", {});  }
                 }
                if(tool.irreversible){ { svelteHTML.createElement("span", { "class":`irr`,});  }}
                   for(let rule of __sveltets_2_ensureArray(otherRules(tool))){rule.id;
                   { svelteHTML.createElement("span", {   "class":`noask`,"title":rule.name,}); ruleBadge(rule); }
                }
                if(own){ { svelteHTML.createElement("span", { "class":`noask`,});      }}
                if(!own && !otherRules(tool).length){ { svelteHTML.createElement("span", { "class":`muted`,});    }}
               }
            }
           }
        }
       }
     }
  }
  if(status && !groups.length){
     { svelteHTML.createElement("p", { "class":`muted small pad`,});   filter;  }
  }

   { svelteHTML.createElement("section", {   "class":`panel`,...__sveltets_2_empty({"data-testid":`mcp-sessions-panel`}),});
     { svelteHTML.createElement("div", { "class":`tools-head`,});
       { svelteHTML.createElement("h4", { "class":`sec`,});    }
      if(wsId){
         { svelteHTML.createElement("div", { "class":`panel-actions`,});
           { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":gatewayLoading || !gatewayNames.length,"onclick":() => void copyGatewayTools(),});  }
           { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":gatewayLoading,"onclick":() => void loadGatewayTools(wsId!),});gatewayLoading ? 'Refreshing…' : 'Refresh'; }
         }
      }
     }
     { svelteHTML.createElement("p", {   "class":`small`,...__sveltets_2_empty({"data-testid":`mcp-builtin-tools`}),});builtInToolsLine(attachState, attach);       { svelteHTML.createElement("code", {});  }  }
    if(gatewayState === 'no-workspace'){
       { svelteHTML.createElement("p", { "class":`muted small`,});        }
    } else if (gatewayState === 'error'){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`the gateway tools`,"variant":`compact`,"error":gatewayError,"empty":true,"onretry":() => wsId && void loadGatewayTools(wsId),}});}
    } else if (gatewayState === 'loading'){ { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {      "what":`the gateway tools`,"variant":`compact`,"loading":true,"empty":true,"rows":1,}});}
    }else{
       { svelteHTML.createElement("p", { "class":`small`,});      gatewayNames.length; }
      if(shownGatewayNames.length){
         { svelteHTML.createElement("div", { "class":`gateway-list`,});
             for(let name of __sveltets_2_ensureArray(shownGatewayNames)){name; { svelteHTML.createElement("code", {});name; }}
          if(gatewayNames.length > shownGatewayNames.length){
             { svelteHTML.createElement("span", { "class":`muted small`,}); gatewayNames.length - shownGatewayNames.length;  }
          }
         }
      }
    }
   }

   { svelteHTML.createElement("div", { "class":`expose-wrap`,});
     { svelteHTML.createElement("button", {         "class":`disclose`,...__sveltets_2_empty({"data-testid":`mcp-expose-toggle`}),"aria-expanded":exposeOpen,"onclick":() => (exposeOpen = !exposeOpen),});
       { svelteHTML.createElement("span", { });exposeOpen; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":13,}});} }
         
     }
    if(exposeOpen){ { const $$_lenaPesopxE2C = __sveltets_2_ensureComponent(ExposePanel); new $$_lenaPesopxE2C({ target: __sveltets_2_any(), props: {   "groups":allGroups,isMcpAdmin,}});}}
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const OttoServerHome__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type OttoServerHome__SvelteComponent_ = ReturnType<typeof OttoServerHome__SvelteComponent_>;
/*Ωignore_endΩ*/export default OttoServerHome__SvelteComponent_;