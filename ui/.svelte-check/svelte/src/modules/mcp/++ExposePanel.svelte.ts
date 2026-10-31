///<reference types="svelte" />
;
import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import { api, baseUrl } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { COPY_FAILED_BODY, copyTextOrThrow } from '../../lib/clipboard';
import { confirmer } from '../../lib/confirm.svelte';
import TokensPanel from './TokensPanel.svelte';
import type { McpOttoToolInfo } from '../../lib/api/types';
;

  interface Props {
    groups: { cat: string; tools: McpOttoToolInfo[] }[];
    isMcpAdmin: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  


  let { groups, isMcpAdmin }: Props = $props();

  const HTTP_PATH = '/api/v1/mcp/http';
  const httpUrl = $derived(`${baseUrl()}${HTTP_PATH}`);
  const httpCommand = $derived(
    `claude mcp add --transport http otto ${httpUrl} --header "Authorization: Bearer YOUR_OTTO_MCP_TOKEN"`,
  );
  const snippet = JSON.stringify(
    {
      mcpServers: {
        otto: {
          command: 'ottod',
          args: ['mcp-server'],
          env: { OTTO_API_TOKEN: 'YOUR_OTTO_MCP_TOKEN' },
        },
      },
    },
    null,
    2,
  );

  const loopbackPort = $derived.by(() => {
    try {
      return Number(new URL(baseUrl()).port) || 7700;
    } catch {
      return 7700;
    }
  });

  let networkOpen = $state(false);
  let networkError = $state(false);
  let netEnabled = $state(false);
  let netPort = $state(7701);
  let netPortTouched = $state(false);
  let netBusy = $state(false);
  const portConflict = $derived(netPort === loopbackPort);

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`${what} copied`);
    } catch {
      toasts.error('Couldn’t copy', COPY_FAILED_BODY);
    }
  }

  async function loadNetwork(): Promise<void> {
    networkError = false;
    try {
      const settings = await api.get<Record<string, unknown>>('/settings');
      const listener = settings['network_listener'] as
        | { enabled?: boolean; port?: number }
        | undefined;
      netEnabled = listener?.enabled ?? false;
      if (listener?.port != null) {
        netPort = listener.port;
        netPortTouched = true;
      } else if (!netPortTouched) {
        netPort = loopbackPort + 1;
      }
    } catch {
      networkError = true;
    }
  }

  async function toggleNetwork(input: HTMLInputElement): Promise<void> {
    if (!netEnabled && portConflict) {
      // One-way `checked`: the click already ticked the box — put it back.
      input.checked = netEnabled;
      toasts.error(
        'Pick a different port',
        `The network port must differ from the daemon’s loopback port (${loopbackPort}).`,
      );
      return;
    }
    // Leaving loopback is the one switch here that weakens a security default
    // (AGENTS.md) — say where it listens and who can reach it before doing it.
    if (
      !netEnabled &&
      !(await confirmer.ask(
        `The daemon will also listen on 0.0.0.0:${netPort} (TLS, self-signed), so any machine that can reach this Mac on that port can try to connect. Every call still needs a valid access token. Takes effect after a daemon restart.`,
        { title: 'Allow network access?', confirmLabel: 'Allow network access' },
      ))
    ) {
      input.checked = false;
      return;
    }
    netBusy = true;
    try {
      const next = !netEnabled;
      await api.put<Record<string, unknown>>('/settings', {
        network_listener: { enabled: next, port: netPort },
      });
      netEnabled = next;
      toasts.success(
        next ? 'Network access enabled' : 'Network access disabled',
        'Restart the daemon to apply the listener change.',
      );
    } catch (e) {
      // One-way `checked` — put the box back to what the daemon still has.
      input.checked = netEnabled;
      toastError('Couldn’t update the setting', e);
    } finally {
      netBusy = false;
    }
  }

  $effect(() => {
    void loadNetwork();
  });
;
async () => {

 { svelteHTML.createElement("div", {   "class":`expose`,...__sveltets_2_empty({"data-testid":`mcp-expose-panel`}),});
   { svelteHTML.createElement("h4", { "class":`sec`,});  }
   { svelteHTML.createElement("p", { "class":`muted small`,});
           { svelteHTML.createElement("code", {});  }     
   }
   { svelteHTML.createElement("div", { "class":`snippet`,});
     { svelteHTML.createElement("button", {   "class":`btn small copy`,"onclick":() => void copy(snippet, '.mcp.json'),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} 
     }
     { svelteHTML.createElement("pre", {});snippet; }
   }

   { svelteHTML.createElement("h4", { "class":`sec`,});  }
   { svelteHTML.createElement("p", { "class":`muted small`,});
                   
                
   }
   { svelteHTML.createElement("div", { "class":`urlrow`,});
     { svelteHTML.createElement("code", {   "class":`url`,...__sveltets_2_empty({"data-testid":`mcp-http-url`}),});httpUrl; }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copy(httpUrl, 'URL'),});  }
   }
   { svelteHTML.createElement("div", { "class":`urlrow`,});
     { svelteHTML.createElement("code", { "class":`url`,});httpCommand; }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copy(httpCommand, 'Command'),});  }
   }

   { svelteHTML.createElement("button", {       "class":`subdisclose`,"aria-expanded":networkOpen,"onclick":() => (networkOpen = !networkOpen),});
     { svelteHTML.createElement("span", { });networkOpen; { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
      
   }
  if(networkOpen){
     { svelteHTML.createElement("div", { "class":`network`,});
      if(networkError){
         { svelteHTML.createElement("p", { "class":`muted small`,});    }
      }else{
         { svelteHTML.createElement("label", { "class":`netrow`,});
           { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":netEnabled,"disabled":netBusy || !isMcpAdmin || (!netEnabled && portConflict),"onchange":(event) => void toggleNetwork(event.currentTarget),});}
           { svelteHTML.createElement("div", { "class":`t-meta`,});
             { svelteHTML.createElement("span", { "class":`t-name`,});
                   
               { svelteHTML.createElement("input", {                 "class":`port`,"type":`number`,"min":`1`,"max":`65535`,"bind:value":netPort,"oninput":() => (netPortTouched = true),"disabled":netEnabled || !isMcpAdmin,"aria-label":`Network port`,});/*Ωignore_startΩ*/() => netPort = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
             { svelteHTML.createElement("span", { "class":`t-desc`,});
                   { svelteHTML.createElement("code", {}); netPort; }     
                   { svelteHTML.createElement("code", {}); netPort;HTTP_PATH; }
                     
             }
            if(portConflict){
               { svelteHTML.createElement("span", { "class":`t-warn`,});
                         loopbackPort;    
               }
            }
           }
         }
      }
     }
  }

  if(isMcpAdmin){
     { const $$_lenaPsnekoT1C = __sveltets_2_ensureComponent(TokensPanel); new $$_lenaPsnekoT1C({ target: __sveltets_2_any(), props: { groups,}});}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ExposePanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ExposePanel__SvelteComponent_ = ReturnType<typeof ExposePanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ExposePanel__SvelteComponent_;