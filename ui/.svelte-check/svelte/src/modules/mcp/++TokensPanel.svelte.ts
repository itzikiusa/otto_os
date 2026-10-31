///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { mcpTokensApi } from '../../lib/api/mcp';
import { api, ApiError, baseUrl } from '../../lib/api/client';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { COPY_FAILED_BODY, copyTextOrThrow } from '../../lib/clipboard';
import { mcpCpExtraApi } from './cp-api';
import type { McpOttoToolInfo, McpScope, McpTokenInfo, User } from '../../lib/api/types';
;

  interface Props {
    groups: { cat: string; tools: McpOttoToolInfo[] }[];
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  


  let { groups }: Props = $props();

  let tokens = $state<McpTokenInfo[]>([]);
  let users = $state<User[]>([]);
  let tokensLoaded = $state(false);
  let showCreate = $state(false);
  let fOwner = $state('');
  let fLabel = $state('');
  let fAllowWrites = $state(false);
  let fRestrictTools = $state(false);
  let fTools = $state<Set<string>>(new Set());
  let fWorkspace = $state('');
  let creating = $state(false);
  let createdToken = $state<{ secret: string; info: McpTokenInfo } | null>(null);
  let rotatedToken = $state<{ secret: string; info: McpTokenInfo } | null>(null);

  /** Failed token-list load — inline with Retry, never "No MCP tokens yet.". */
  let tokensError = $state<string | null>(null);

  async function loadTokens(): Promise<void> {
    try {
      const result = await mcpTokensApi.list();
      tokens = result.tokens;
      tokensError = null;
    } catch (e) {
      tokensError = loadErrorText(e);
    } finally {
      tokensLoaded = true;
    }
  }

  /** The owner list failed to load: the picker falls back to "Me" and says why. */
  let usersError = $state<string | null>(null);
  async function loadUsers(): Promise<void> {
    try {
      users = await api.get<User[]>('/users');
      usersError = null;
    } catch (e) {
      users = [];
      // Only the owner may list users (403 otherwise): then "Me" is the whole
      // list by design, not a failure.
      usersError = e instanceof ApiError && e.status === 403 ? null : loadErrorText(e);
    }
  }

  function bare(name: string): string {
    return name.replace(/^otto\./, '');
  }

  function toggleFormTool(name: string): void {
    const tool = bare(name);
    const next = new Set(fTools);
    if (next.has(tool)) next.delete(tool);
    else next.add(tool);
    fTools = next;
  }

  async function createToken(): Promise<void> {
    if (fRestrictTools && fTools.size === 0) return;
    creating = true;
    try {
      const scope: McpScope = {
        tools: fRestrictTools ? [...fTools] : null,
        allow_writes: fAllowWrites,
        workspace_id: fWorkspace || null,
      };
      const resp = await mcpTokensApi.create({
        user_id: fOwner || undefined,
        label: fLabel.trim() || undefined,
        scope,
      });
      createdToken = { secret: resp.token, info: resp.info };
      rotatedToken = null;
      toasts.success('MCP token created', 'Copy it now — it is shown only once.');
      fLabel = '';
      fTools = new Set();
      fRestrictTools = false;
      fAllowWrites = false;
      fWorkspace = '';
      fOwner = '';
      showCreate = false;
      await loadTokens();
    } catch (e) {
      toastError('Couldn’t create the token', e);
    } finally {
      creating = false;
    }
  }

  async function rotateToken(t: McpTokenInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Rotate MCP token ${t.label ? `“${t.label}” ` : ''}(${t.token_prefix}…) for ${t.username}? Only this token is replaced — every other token keeps working. The old secret stops working immediately.`,
      { title: 'Rotate token', confirmLabel: 'Rotate', danger: true },
    );
    if (!ok) return;
    try {
      const resp = await mcpCpExtraApi.rotateToken(t.id);
      rotatedToken = { secret: resp.token, info: resp.info };
      createdToken = null;
      toasts.success('Token rotated', 'Only this token was replaced.');
      await loadTokens();
    } catch (e) {
      toastError('Couldn’t rotate the token', e);
    }
  }

  async function revokeToken(t: McpTokenInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke MCP token ${t.label ? `“${t.label}” ` : ''}(${t.token_prefix}…) for ${t.username}? Any client using it stops working immediately.`,
      { title: 'Revoke token', confirmLabel: 'Revoke', danger: true },
    );
    if (!ok) return;
    try {
      await mcpTokensApi.revoke(t.id);
      toasts.success('Token revoked');
      await loadTokens();
    } catch (e) {
      toastError('Couldn’t revoke the token', e);
    }
  }

  function scopeSummary(scope: McpScope): string {
    const toolPart =
      scope.tools == null
        ? 'all tools'
        : `${plural(scope.tools.length, 'tool')}`;
    const writePart = scope.allow_writes ? 'read + write' : 'read-only';
    const wsPart = scope.workspace_id ? ' • 1 workspace' : '';
    return `${toolPart} • ${writePart}${wsPart}`;
  }

  function clientCommand(token: string): string {
    return `claude mcp add --transport http otto ${baseUrl()}/api/v1/mcp/http --header "Authorization: Bearer ${token}"`;
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`${what} copied`);
    } catch {
      toasts.error('Couldn’t copy', COPY_FAILED_BODY);
    }
  }

  $effect(() => {
    void loadTokens();
    void loadUsers();
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`tokens`,});
   { svelteHTML.createElement("div", { "class":`tools-head`,});
     { svelteHTML.createElement("h4", { "class":`sec`,});  }
     { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-testid":`mcp-new-token`}),"onclick":() => (showCreate = !showCreate),});
      showCreate ? 'Cancel' : 'New token';
     }
   }
   { svelteHTML.createElement("p", { "class":`muted small`,});
                   
   }

  if(createdToken){
     { svelteHTML.createElement("div", {   "class":`token-once`,...__sveltets_2_empty({"data-testid":`mcp-created-token`}),});
       { svelteHTML.createElement("div", { "class":`to-head`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":13,}});}
         { svelteHTML.createElement("strong", {});   createdToken.info.username;       }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copy(createdToken!.secret, 'Token'),});  }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copy(clientCommand(createdToken!.secret), 'Command'),});  }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (createdToken = null),});  }
       }
       { svelteHTML.createElement("code", { "class":`token`,});createdToken.secret; }
       { svelteHTML.createElement("code", { "class":`token cmd`,});clientCommand(createdToken.secret); }
     }
  }

  if(rotatedToken){
     { svelteHTML.createElement("div", {   "class":`token-once`,...__sveltets_2_empty({"data-testid":`mcp-rotated-token`}),});
       { svelteHTML.createElement("div", { "class":`to-head`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":13,}});}
         { svelteHTML.createElement("strong", {});   rotatedToken.info.username;       }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copy(rotatedToken!.secret, 'Token'),});  }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copy(clientCommand(rotatedToken!.secret), 'Command'),});  }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (rotatedToken = null),});  }
       }
       { svelteHTML.createElement("code", { "class":`token`,});rotatedToken.secret; }
       { svelteHTML.createElement("code", { "class":`token cmd`,});clientCommand(rotatedToken.secret); }
     }
  }

  if(showCreate){
     { svelteHTML.createElement("div", { "class":`create`,});
       { svelteHTML.createElement("div", { "class":`frow`,});
        
         { svelteHTML.createElement("div", { "class":`field tok-field`,});
           { svelteHTML.createElement("label", { "for":`mcp-token-owner`,});  }
           { svelteHTML.createElement("select", {       "id":`mcp-token-owner`,"class":`input`,"bind:value":fOwner,"aria-describedby":usersError ? 'mcp-token-owner-err' : undefined,});/*Ωignore_startΩ*/() => fOwner = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",}); auth.me?.username ?? 'self';  }
               for(let u of __sveltets_2_ensureArray(users)){u.id;
               { svelteHTML.createElement("option", { "value":u.id,});u.username; }
            }
           }
          if(usersError){
             { svelteHTML.createElement("span", {   "class":`hint tok-note`,"id":`mcp-token-owner-err`,});
                        
               { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small ghost`,"onclick":() => void loadUsers(),});  }
             }
          }
         }
         { svelteHTML.createElement("div", { "class":`field tok-field`,});
           { svelteHTML.createElement("label", { "for":`mcp-token-label`,});  }
           { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`mcp-token-label`,"class":`input`,"placeholder":`ci-readonly`,"bind:value":fLabel,});/*Ωignore_startΩ*/() => fLabel = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`field tok-field`,});
           { svelteHTML.createElement("label", { "for":`mcp-token-workspace`,});   }
           { svelteHTML.createElement("select", {     "id":`mcp-token-workspace`,"class":`input`,"bind:value":fWorkspace,});/*Ωignore_startΩ*/() => fWorkspace = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  }
               for(let workspace of __sveltets_2_ensureArray(ws.workspaces)){workspace.id;
               { svelteHTML.createElement("option", { "value":workspace.id,});workspace.name; }
            }
           }
         }
       }
       { svelteHTML.createElement("label", { "class":`chk`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fAllowWrites,});/*Ωignore_startΩ*/() => fAllowWrites = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 
       }
       { svelteHTML.createElement("label", { "class":`chk`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fRestrictTools,});/*Ωignore_startΩ*/() => fRestrictTools = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                
       }
      if(fRestrictTools){
         { svelteHTML.createElement("div", { "class":`tool-pick`,});
             for(let group of __sveltets_2_ensureArray(groups)){group.cat;
             { svelteHTML.createElement("div", { "class":`pick-grp`,});
               { svelteHTML.createElement("span", { "class":`grp-name`,});group.cat; }
                 for(let tool of __sveltets_2_ensureArray(group.tools)){tool.name;
                 { svelteHTML.createElement("label", { "class":`ptool`,});
                   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":fTools.has(bare(tool.name)),"onchange":() => toggleFormTool(tool.name),});}
                   { svelteHTML.createElement("span", { "class":`mono`,});bare(tool.name); }if(tool.mutating){ { svelteHTML.createElement("span", { "class":`mut`,});  }}
                 }
              }
             }
          }
         }
      }
       { svelteHTML.createElement("div", { "class":`cactions`,});
         { svelteHTML.createElement("button", {           "class":`btn primary`,...__sveltets_2_empty({"data-testid":`mcp-create-token`}),"disabled":creating || (fRestrictTools && fTools.size === 0),"title":fRestrictTools && fTools.size === 0 ? 'Pick at least one tool — a token restricted to none can call nothing' : undefined,"onclick":() => void createToken(),});
          creating ? 'Creating…' : 'Create token';
         }
       }
     }
  }

   { svelteHTML.createElement("div", {   "class":`tok-list`,...__sveltets_2_empty({"data-testid":`mcp-tokens`}),});
    if(tokensError && !tokens.length){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`MCP tokens`,"variant":`compact`,"error":tokensError,"empty":true,"onretry":() => void loadTokens(),}});}
    } else if (!tokens.length){
       { svelteHTML.createElement("p", { "class":`muted small pad`,});tokensLoaded ? 'No MCP tokens yet.' : 'Loading MCP tokens…'; }
    }else{
         for(let tokenInfo of __sveltets_2_ensureArray(tokens)){tokenInfo.id;
         { svelteHTML.createElement("div", {       "class":`tok-row`,...__sveltets_2_empty({"data-testid":`mcp-token-row`}),...__sveltets_2_empty({"data-token-id":tokenInfo.id}),});
           { svelteHTML.createElement("div", { "class":`tok-main`,});
             { svelteHTML.createElement("span", { "class":`tok-label`,});tokenInfo.label || '(no label)'; }
             { svelteHTML.createElement("span", { "class":`tok-prefix mono`,});tokenInfo.token_prefix;  }
           }
           { svelteHTML.createElement("span", { "class":`tok-user`,});tokenInfo.username; }
           { svelteHTML.createElement("span", { "class":`tok-scope muted`,});scopeSummary(tokenInfo.scope); }
           { svelteHTML.createElement("span", { "class":`tok-seen muted`,});rel(tokenInfo.last_seen_at); }
           { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-testid":`mcp-token-rotate`}),"onclick":() => void rotateToken(tokenInfo),});  }
           { svelteHTML.createElement("button", {       "class":`btn small danger`,...__sveltets_2_empty({"data-testid":`mcp-token-revoke`}),"onclick":() => void revokeToken(tokenInfo),});  }
         }
      }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TokensPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TokensPanel__SvelteComponent_ = ReturnType<typeof TokensPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default TokensPanel__SvelteComponent_;