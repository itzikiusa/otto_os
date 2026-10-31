///<reference types="svelte" />
;
import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import Modal from '../../lib/components/Modal.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { brokers } from '../../lib/stores/brokers.svelte';
import { toasts } from '../../lib/toast.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import type {
    BrokerCluster,
    Environment,
    SaslMechanism,
    SecurityProtocol,
    SshTunnelConfig,
    UpsertClusterReq,
  } from '../../lib/api/types';
;

  interface Props {
    cluster?: BrokerCluster | null;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { cluster = null, onclose }: Props = $props();

  // The modal is remounted per open, so a one-time snapshot of the prop seeds
  // the form (untrack keeps these out of the reactive graph).
  const init = untrack(() => cluster);
  const editing = init !== null;

  let name = $state(init?.name ?? '');
  let bootstrap = $state(init?.bootstrap_servers ?? '');
  let security = $state<SecurityProtocol>(init?.security_protocol ?? 'plaintext');
  let mechanism = $state<SaslMechanism>(init?.sasl_mechanism ?? 'plain');
  let saslUser = $state(init?.sasl_username ?? '');
  let saslPass = $state('');
  let tlsSkip = $state(init?.tls_skip_verify ?? false);
  let srUrl = $state(init?.schema_registry_url ?? '');
  let srUser = $state(init?.schema_registry_username ?? '');
  let srPass = $state('');
  let metricsUrl = $state(init?.metrics_url ?? '');
  let environment = $state<Environment>(init?.environment ?? 'dev');
  let readOnly = $state(init?.read_only ?? false);
  let color = $state(init?.color ?? '');
  let saving = $state(false);
  // Inline validation: shown under the field after the first Save attempt.
  let submitted = $state(false);
  const nameErr = $derived(submitted && !name.trim() ? 'Give the cluster a name.' : null);
  const bootstrapErr = $derived(
    submitted && !bootstrap.trim() ? 'Enter at least one broker, e.g. broker1:9092.' : null,
  );

  // SSH tunnel (bastion) — for private clusters like AWS MSK in a VPC.
  let tunnelOpen = $state(!!init?.ssh);
  let tunHost = $state(init?.ssh?.host ?? '');
  let tunPort = $state(init?.ssh?.port != null ? String(init.ssh.port) : '');
  let tunUser = $state(init?.ssh?.user ?? '');
  let tunIdentity = $state(init?.ssh?.identity_file ?? '');
  let showTunnelFilePicker = $state(false);
  const tunHostErr = $derived(
    submitted && tunnelOpen && !tunHost.trim() ? 'Enter the bastion host, or turn the SSH tunnel off.' : null,
  );

  const usesSasl = $derived(security === 'sasl_plaintext' || security === 'sasl_ssl');
  const usesTls = $derived(security === 'ssl' || security === 'sasl_ssl');

  async function save() {
    submitted = true;
    // ssh: omit on create (keep) when off; send null on edit to clear. Only the
    // host is required — a blank user falls back server-side to ~/.ssh/config
    // or $USER, exactly like the connections form.
    if (!name.trim() || !bootstrap.trim() || (tunnelOpen && !tunHost.trim())) return;
    let ssh: SshTunnelConfig | null | undefined;
    if (tunnelOpen && tunHost.trim()) {
      ssh = {
        host: tunHost.trim(),
        port: tunPort.trim() ? Number(tunPort) : undefined,
        ...(tunUser.trim() ? { user: tunUser.trim() } : {}),
        identity_file: tunIdentity.trim() || null,
      };
    } else if (editing) {
      ssh = null;
    } else {
      ssh = undefined;
    }
    const req: UpsertClusterReq = {
      name: name.trim(),
      bootstrap_servers: bootstrap.trim(),
      security_protocol: security,
      sasl_mechanism: usesSasl ? mechanism : null,
      sasl_username: usesSasl ? saslUser.trim() || null : null,
      sasl_password: saslPass ? saslPass : undefined,
      tls_skip_verify: tlsSkip,
      schema_registry_url: srUrl.trim() || null,
      schema_registry_username: srUser.trim() || null,
      schema_registry_password: srPass ? srPass : undefined,
      metrics_url: metricsUrl.trim() || null,
      color: color.trim() || null,
      ssh,
      environment,
      read_only: readOnly,
    };
    saving = true;
    try {
      if (editing && cluster) await brokers.update(cluster.id, req);
      else await brokers.create(req);
      toasts.success(editing ? 'Cluster updated' : 'Cluster added');
      onclose();
    } catch (e) {
      toastError('Couldn’t save the cluster', e);
    } finally {
      saving = false;
    }
  }
;
async () => {


 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":editing ? 'Edit cluster' : 'Add Kafka cluster',"width":viewport.isPhone ? 320 : 540,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":save,"disabled":saving,});
      saving ? 'Saving…' : editing ? 'Save' : 'Add cluster';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`form`,});
     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {        "dir":`auto`,"bind:value":name,"placeholder":`prod-kafka`,"aria-invalid":nameErr ? 'true' : undefined,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(nameErr){ { svelteHTML.createElement("span", { "class":`field-err`,});nameErr; }}
     }
     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {        "dir":`ltr`,"bind:value":bootstrap,"placeholder":`broker1:9092,broker2:9092`,"aria-invalid":bootstrapErr ? 'true' : undefined,});/*Ωignore_startΩ*/() => bootstrap = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(bootstrapErr){ { svelteHTML.createElement("span", { "class":`field-err`,});bootstrapErr; }}
     }

     { svelteHTML.createElement("div", { "class":`cf-row`,});
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", { "bind:value":security,});/*Ωignore_startΩ*/() => security = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`plaintext`,});  }
           { svelteHTML.createElement("option", { "value":`ssl`,});  }
           { svelteHTML.createElement("option", { "value":`sasl_plaintext`,});  }
           { svelteHTML.createElement("option", { "value":`sasl_ssl`,});  }
         }
       }
      if(usesSasl){
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", { "bind:value":mechanism,});/*Ωignore_startΩ*/() => mechanism = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`plain`,});  }
             { svelteHTML.createElement("option", { "value":`scram_sha_256`,});  }
             { svelteHTML.createElement("option", { "value":`scram_sha_512`,});  }
           }
         }
      }
     }

    if(usesSasl){
       { svelteHTML.createElement("div", { "class":`cf-row`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":saslUser,"autocomplete":`off`,});/*Ωignore_startΩ*/() => saslUser = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {         "type":`password`,"bind:value":saslPass,"autocomplete":`new-password`,"placeholder":cluster?.has_sasl_password ? '•••••• (unchanged)' : '',});/*Ωignore_startΩ*/() => saslPass = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
    }
    if(usesTls){
       { svelteHTML.createElement("label", { "class":`check`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":tlsSkip,});/*Ωignore_startΩ*/() => tlsSkip = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", {});      }
       }
    }

     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});    { svelteHTML.createElement("em", {});     } }
       { svelteHTML.createElement("input", {      "dir":`ltr`,"bind:value":srUrl,"placeholder":`http://schema-registry:8081`,});/*Ωignore_startΩ*/() => srUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    if(srUrl.trim()){
       { svelteHTML.createElement("div", { "class":`cf-row`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":srUser,"autocomplete":`off`,});/*Ωignore_startΩ*/() => srUser = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {         "type":`password`,"bind:value":srPass,"autocomplete":`new-password`,"placeholder":cluster?.has_sr_password ? '•••••• (unchanged)' : '',});/*Ωignore_startΩ*/() => srPass = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
    }

     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});      } }
       { svelteHTML.createElement("input", {      "dir":`ltr`,"bind:value":metricsUrl,"placeholder":`http://broker:9644/public_metrics`,});/*Ωignore_startΩ*/() => metricsUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

     { svelteHTML.createElement("label", { "class":`check`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":tunnelOpen,});/*Ωignore_startΩ*/() => tunnelOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});            } }
     }
    if(tunnelOpen){
       { svelteHTML.createElement("div", { "class":`ssh-section`,});
         { svelteHTML.createElement("div", { "class":`cf-row`,});
           { svelteHTML.createElement("label", {   "class":`field`,"style":`flex: 1;`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {          "dir":`ltr`,"bind:value":tunHost,"placeholder":`bastion.example.com`,"spellcheck":`false`,"aria-invalid":tunHostErr ? 'true' : undefined,});/*Ωignore_startΩ*/() => tunHost = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(tunHostErr){ { svelteHTML.createElement("span", { "class":`field-err`,});tunHostErr; }}
           }
           { svelteHTML.createElement("label", {   "class":`field`,"style":`flex: 0 0 90px;`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {      "type":`number`,"bind:value":tunPort,"placeholder":`22`,});/*Ωignore_startΩ*/() => tunPort = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});  } }
           { svelteHTML.createElement("input", {        "dir":`auto`,"bind:value":tunUser,"placeholder":`default (~/.ssh/config or $USER)`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tunUser = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});     } }
           { svelteHTML.createElement("div", { "class":`file-input-row`,});
             { svelteHTML.createElement("input", {        "dir":`ltr`,"bind:value":tunIdentity,"placeholder":`~/.ssh/id_rsa`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => tunIdentity = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("button", {   "class":`btn browse-btn`,"onclick":() => (showTunnelFilePicker = true),});  }
           }
         }
       }
    }

     { svelteHTML.createElement("div", { "class":`cf-row`,});
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", { "bind:value":environment,});/*Ωignore_startΩ*/() => environment = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`dev`,});  }
           { svelteHTML.createElement("option", { "value":`staging`,});  }
           { svelteHTML.createElement("option", { "value":`prod`,});  }
         }
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":color,"placeholder":`#0a84ff`,});/*Ωignore_startΩ*/() => color = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
     }
     { svelteHTML.createElement("label", { "class":`check`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":readOnly,});/*Ωignore_startΩ*/() => readOnly = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", {});          }
     }
   }

  
 Modal}

if(showTunnelFilePicker){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {           "title":`Choose Identity File`,"start":tunIdentity ? tunIdentity.replace(/\/[^/]+$/, '') : '',"files":true,"onpick":(path) => { tunIdentity = path; showTunnelFilePicker = false; },"onclose":() => (showTunnelFilePicker = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ClusterForm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ClusterForm__SvelteComponent_ = ReturnType<typeof ClusterForm__SvelteComponent_>;
/*Ωignore_endΩ*/export default ClusterForm__SvelteComponent_;