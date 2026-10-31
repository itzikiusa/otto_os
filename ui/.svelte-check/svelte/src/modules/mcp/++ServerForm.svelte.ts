///<reference types="svelte" />
;// Register a governed MCP server. Two transports:
//   stdio — Otto spawns `command args` (with env + secret env) as the daemon.
//           This runs an ARBITRARY local command and requires the owner.
//   http  — Otto POSTs JSON-RPC to `url` with headers (+ secret headers).
// Secret env/header values are write-only: they go to the macOS Keychain and
// are never returned — responses only carry their key names + has_secret.

import { auth } from '../../lib/stores/auth.svelte';
import { toastError } from '../../lib/toastError';
import { announce } from '../../lib/announce.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { mcpCpApi } from '../../lib/api/mcp';
import { toasts } from '../../lib/toast.svelte';
import type {
    CreateMcpControlServerReq,
    McpInjectionRisk,
    McpToolAccess,
    McpTransport,
  } from '../../lib/api/types';
import { MCP_SERVER_TEMPLATES, type McpServerTemplate } from './templates';
;

  interface Props {
    wsId: string;
    onclose: () => void;
    onsaved: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, onclose, onsaved }: Props = $props();

  let name = $state('');
  let transport = $state<McpTransport>('stdio');
  let description = $state('');
  // stdio
  let command = $state('');
  let argsText = $state('');
  let envText = $state('');
  let secretEnvText = $state('');
  // http
  let url = $state('');
  let headersText = $state('');
  let secretHeadersText = $state('');
  // governance
  let injectionRisk = $state<McpInjectionRisk>('medium');
  let defaultToolAccess = $state<McpToolAccess>('allow');
  let enabled = $state(false);
  let advancedOpen = $state(false);

  let saving = $state(false);

  // ── Templates (static starters, e.g. Blender) ──────────────────────────────
  let templateId = $state('');
  const template = $derived(MCP_SERVER_TEMPLATES.find((t) => t.id === templateId) ?? null);

  /** Pre-fill the form from a template; the user still reviews + saves. */
  function applyTemplate(t: McpServerTemplate | null): void {
    if (!t) return;
    name = t.name;
    transport = t.transport;
    description = t.description;
    command = t.command ?? '';
    argsText = (t.args ?? []).join('\n');
    envText = Object.entries(t.env ?? {}).map(([k, v]) => `${k}=${v}`).join('\n');
    url = t.url ?? '';
    injectionRisk = t.injection_risk;
    defaultToolAccess = t.default_tool_access;
    enabled = false; // templates never start enabled
  }

  /** One value per non-empty line. */
  function parseLines(text: string): string[] {
    return text
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean);
  }

  /** `KEY=value` per non-empty line → a record (value may be empty). */
  function parseKv(text: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of text.split('\n')) {
      const t = line.trim();
      if (!t) continue;
      const eq = t.indexOf('=');
      if (eq < 0) out[t] = '';
      else out[t.slice(0, eq).trim()] = t.slice(eq + 1).trim();
    }
    return out;
  }

  /** Inline validation, next to each field (never a toast). */
  let nameError = $state<string | null>(null);
  let commandError = $state<string | null>(null);
  let urlError = $state<string | null>(null);
  let envError = $state<string | null>(null);

  /** The server's save-time env rule (`otto_mcp::service::env_key_error`):
   *  sessions pass env through an `sh` env file, so a key must be a shell
   *  variable name or the server never reaches an agent session. */
  const SHELL_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
  function badEnvKey(...maps: Record<string, string>[]): string | null {
    for (const m of maps) for (const k of Object.keys(m)) if (!SHELL_NAME.test(k)) return k;
    return null;
  }

  async function save(): Promise<void> {
    if (!auth.isRoot) return;
    nameError = name.trim() ? null : 'Give the server a name.';
    commandError = transport === 'stdio' && !command.trim() ? 'A stdio server needs a command.' : null;
    urlError = transport === 'http' && !url.trim() ? 'An HTTP server needs a URL.' : null;
    const bad = transport === 'stdio' ? badEnvKey(parseKv(envText), parseKv(secretEnvText)) : null;
    envError =
      bad == null
        ? null
        : `“${bad}” can’t be an env key. Use letters, digits and _, not starting with a digit.`;
    const invalid = nameError ?? commandError ?? urlError ?? envError;
    if (invalid) {
      // Spoken once at the edge; the inline lines are plain text tied to
      // their fields (an inserted role="alert" is silent in VoiceOver).
      announce(invalid, { assertive: true });
      return;
    }
    const body: CreateMcpControlServerReq = {
      name: name.trim(),
      transport,
      description: description.trim() || null,
      injection_risk: injectionRisk,
      default_tool_access: defaultToolAccess,
      enabled,
    };
    if (transport === 'stdio') {
      body.command = command.trim();
      body.args = parseLines(argsText);
      body.env = parseKv(envText);
      body.secret_env = parseKv(secretEnvText);
    } else {
      body.url = url.trim();
      body.headers = parseKv(headersText);
      body.secret_headers = parseKv(secretHeadersText);
    }
    saving = true;
    try {
      await mcpCpApi.cpCreate(wsId, body);
      toasts.success('MCP server added', name.trim());
      onsaved();
      onclose();
    } catch (e) {
      toastError('Couldn’t add server', e);
    } finally {
      saving = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Add MCP server`,"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn`,"onclick":onclose,"disabled":saving,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":save,"disabled":saving,});
      saving ? 'Adding…' : 'Add server';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`form`,});
     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});     { svelteHTML.createElement("em", {});  } }
       { svelteHTML.createElement("select", {         "class":`template-pick`,"bind:value":templateId,"onchange":() => applyTemplate(template),"aria-label":`Server template`,});/*Ωignore_startΩ*/() => templateId = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});   }
           for(let t of __sveltets_2_ensureArray(MCP_SERVER_TEMPLATES)){t.id;
           { svelteHTML.createElement("option", { "value":t.id,});t.name;  t.transport; }
        }
       }
     }
    if(template?.notes){
       { svelteHTML.createElement("p", { "class":`tpl-notes`,});
        template.notes;
        if(template.link){
           { svelteHTML.createElement("a", {     "href":template.link,"target":`_blank`,"rel":`noopener noreferrer`,});    }
        }
       }
    }

     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {            "dir":`auto`,"bind:value":name,"placeholder":`github`,"aria-invalid":nameError != null,"aria-describedby":nameError ? 'mcp-server-name-err' : undefined,"oninput":() => (nameError = null),});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(nameError){ { svelteHTML.createElement("span", {   "id":`mcp-server-name-err`,"class":`field-error`,});nameError; }}
     }

     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("select", { "bind:value":transport,});/*Ωignore_startΩ*/() => transport = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`stdio`,});     }
         { svelteHTML.createElement("option", { "value":`http`,});     }
       }
     }

    if(transport === 'stdio'){
       { svelteHTML.createElement("p", { "class":`warn`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}         { svelteHTML.createElement("strong", {});    } 
              
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("input", {              "dir":`ltr`,"bind:value":command,"placeholder":`npx`,"class":`mono`,"aria-invalid":commandError != null,"aria-describedby":commandError ? 'mcp-server-command-err' : undefined,"oninput":() => (commandError = null),});/*Ωignore_startΩ*/() => command = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(commandError){ { svelteHTML.createElement("span", {   "id":`mcp-server-command-err`,"class":`field-error`,});commandError; }}
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  { svelteHTML.createElement("em", {});   } }
         { svelteHTML.createElement("textarea", {         "dir":`ltr`,"bind:value":argsText,"rows":3,"class":`mono`,"placeholder":'-y\n@modelcontextprotocol/server-github',});/*Ωignore_startΩ*/() => argsText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  { svelteHTML.createElement("em", {});    } }
         { svelteHTML.createElement("textarea", {               "dir":`ltr`,"bind:value":envText,"rows":2,"class":`mono`,"placeholder":`LOG_LEVEL=info`,"aria-invalid":envError != null,"aria-describedby":envError ? 'mcp-server-env-err' : undefined,"oninput":() => (envError = null),});/*Ωignore_startΩ*/() => envText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("span", { "class":`env-note`,});
                         
                  
                
         }
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});        } }
         { svelteHTML.createElement("textarea", {               "dir":`ltr`,"bind:value":secretEnvText,"rows":2,"class":`mono`,"placeholder":`GITHUB_TOKEN=ghp_…`,"aria-invalid":envError != null,"aria-describedby":envError ? 'mcp-server-env-err' : undefined,"oninput":() => (envError = null),});/*Ωignore_startΩ*/() => secretEnvText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
        if(envError){ { svelteHTML.createElement("span", {   "id":`mcp-server-env-err`,"class":`field-error`,});envError; }}
       }
    }else{
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("input", {              "dir":`ltr`,"bind:value":url,"placeholder":`https://mcp.example.com/rpc`,"class":`mono`,"aria-invalid":urlError != null,"aria-describedby":urlError ? 'mcp-server-url-err' : undefined,"oninput":() => (urlError = null),});/*Ωignore_startΩ*/() => url = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(urlError){ { svelteHTML.createElement("span", {   "id":`mcp-server-url-err`,"class":`field-error`,});urlError; }}
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  { svelteHTML.createElement("em", {});    } }
         { svelteHTML.createElement("textarea", {         "dir":`ltr`,"bind:value":headersText,"rows":2,"class":`mono`,"placeholder":`X-Client=otto`,});/*Ωignore_startΩ*/() => headersText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});      } }
         { svelteHTML.createElement("textarea", {         "dir":`ltr`,"bind:value":secretHeadersText,"rows":2,"class":`mono`,"placeholder":`Authorization=Bearer …`,});/*Ωignore_startΩ*/() => secretHeadersText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
    }

     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":description,"placeholder":`What this server provides (optional)`,});/*Ωignore_startΩ*/() => description = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

     { svelteHTML.createElement("button", {         "type":`button`,"class":`disclose`,"aria-expanded":advancedOpen,"onclick":() => (advancedOpen = !advancedOpen),});
       { svelteHTML.createElement("span", {});  }
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":advancedOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
     }
    if(advancedOpen){
       { svelteHTML.createElement("div", { "class":`row2`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", { "bind:value":injectionRisk,});/*Ωignore_startΩ*/() => injectionRisk = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`low`,});  }
             { svelteHTML.createElement("option", { "value":`medium`,});  }
             { svelteHTML.createElement("option", { "value":`high`,});  }
           }
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});   }
           { svelteHTML.createElement("select", { "bind:value":defaultToolAccess,});/*Ωignore_startΩ*/() => defaultToolAccess = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`allow`,});  }
             { svelteHTML.createElement("option", { "value":`deny`,});  }
           }
         }
       }
    }

     { svelteHTML.createElement("label", { "class":`check`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":enabled,});/*Ωignore_startΩ*/() => enabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", {});       }
     }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ServerForm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ServerForm__SvelteComponent_ = ReturnType<typeof ServerForm__SvelteComponent_>;
/*Ωignore_endΩ*/export default ServerForm__SvelteComponent_;