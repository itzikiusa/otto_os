///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
// Add / edit account wizard (2 steps). Step 1 picks the credential source:
// an existing `~/.aws` profile (from `/aws/discover`, with SSO / role hints)
// or typed access keys (+ optional session token / role ARN) and the region.
// Step 2 names the account, sets environment + color, then "Save & test":
// the daemon needs a row to run `sts get-caller-identity`, so the test runs
// right after save and its result (identity / login required) is shown in a
// final panel with a "Sign in" shortcut. Editing skips discovery.
import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
import { untrack } from 'svelte';
import { auth } from '../../lib/stores/auth.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { aws } from '../../lib/stores/aws.svelte';
import { awsApi } from '../../lib/api/aws';
import { toasts } from '../../lib/toast.svelte';
import { radioKey } from '../../lib/radioKey';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import type {
    AwsAccount,
    AwsAuthMode,
    AwsTestResp,
    DiscoveredProfile,
    Environment,
    UpsertAwsAccountReq,
  } from '../../lib/api/types';
;

  interface Props {
    /** null = create. */
    account: AwsAccount | null;
    onclose: () => void;
    onsignin: (a: AwsAccount) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { account, onclose, onsignin }: Props = $props();

  // Mounted fresh per open (parent {#if}-gates it) — capturing the initial
  // values into form state is intentional.
  // svelte-ignore state_referenced_locally
  const init = account;
  const editing = init !== null;

  // Account colours are theme tokens (stored as the `var(--…)` reference and
  // painted as a CSS background), so every swatch adapts to light / dark.
  // Accounts saved with an older hex colour keep rendering it as-is.
  const COLORS: readonly { value: string; name: string }[] = [
    { value: 'var(--cat-2)', name: 'Amber' },
    { value: 'var(--cat-1)', name: 'Blue' },
    { value: 'var(--cat-3)', name: 'Green' },
    { value: 'var(--cat-4)', name: 'Violet' },
    { value: 'var(--danger)', name: 'Red' },
    { value: 'var(--cat-6)', name: 'Teal' },
    { value: 'var(--cat-5)', name: 'Pink' },
    { value: 'var(--text-dim)', name: 'Slate' },
  ];
  const MODE_TABS: TabItem<AwsAuthMode>[] = [
    { id: 'profile', label: 'Use an existing AWS profile' },
    { id: 'access_keys', label: 'Enter access keys' },
  ];

  let step = $state<1 | 2 | 3>(editing ? 2 : 1);
  let mode = $state<AwsAuthMode>(init?.auth_mode ?? 'profile');
  let profile = $state(init?.profile ?? '');
  let region = $state(init?.region ?? 'us-east-1');
  let accessKeyId = $state(init?.access_key_id ?? '');
  let secret = $state('');
  let sessionToken = $state('');
  let roleArn = $state(init?.role_arn ?? '');
  // Advanced: custom endpoint (LocalStack / VPC endpoints / S3-compatible).
  // Sent as a plain string so an emptied field CLEARS it on PATCH.
  let endpointUrl = $state(init?.endpoint_url ?? '');
  let advancedOpen = $state(Boolean(init?.endpoint_url));
  let name = $state(init?.name ?? '');
  let environment = $state<Environment>(init?.environment ?? 'dev');
  let color = $state(init?.color ?? COLORS[0].value);

  let profiles = $state<DiscoveredProfile[]>([]);
  let profilesLoading = $state(false);
  let profileFilter = $state('');
  let busy = $state(false);
  let error = $state('');
  let saved = $state<AwsAccount | null>(null);
  let testResult = $state<AwsTestResp | null>(null);
  let testing = $state(false);

  $effect(() => {
    untrack(() => {
      void aws.loadRegions();
      if (!editing && auth.isRoot) void loadProfiles();
    });
  });

  async function loadProfiles(): Promise<void> {
    profilesLoading = true;
    try {
      profiles = (await awsApi.discover()).profiles;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      profiles = [];
    } finally {
      profilesLoading = false;
    }
  }

  const filteredProfiles = $derived.by(() => {
    const q = profileFilter.trim().toLowerCase();
    return q ? profiles.filter((p) => p.name.toLowerCase().includes(q)) : profiles;
  });

  function pickProfile(p: DiscoveredProfile): void {
    profile = p.name;
    if (p.region) region = p.region;
    if (!name) name = p.name;
    step = 2;
  }

  const step1Valid = $derived(
    mode === 'profile' ? profile.trim() !== '' : accessKeyId.trim() !== '' && (editing || secret !== ''),
  );
  const step2Valid = $derived(name.trim() !== '' && region.trim() !== '');

  function hint(p: DiscoveredProfile): string {
    const bits: string[] = [];
    if (p.sso_start_url || p.sso_session) bits.push('SSO');
    if (p.role_arn) bits.push(`role ${p.role_arn.split('/').pop()}`);
    if (p.region) bits.push(p.region);
    bits.push(p.source);
    return bits.join(' · ');
  }

  function body(): UpsertAwsAccountReq {
    const b: UpsertAwsAccountReq = {
      name: name.trim(),
      auth_mode: mode,
      region: region.trim(),
      environment,
      color,
      role_arn: roleArn.trim() || null,
      endpoint_url: endpointUrl.trim(),
    };
    if (mode === 'profile') {
      b.profile = profile.trim();
    } else {
      b.access_key_id = accessKeyId.trim();
      if (secret) b.secret_access_key = secret;
      if (sessionToken) b.session_token = sessionToken;
    }
    return b;
  }

  async function saveAndTest(): Promise<void> {
    if (!step2Valid || busy || (!editing && !auth.isRoot)) return;
    busy = true;
    error = '';
    try {
      const a = editing && init ? await aws.updateAccount(init.id, auth.isRoot ? body() : {name:name.trim(),color}) : await aws.createAccount(body());
      saved = a;
      step = 3;
      await runTest(a);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function runTest(a: AwsAccount): Promise<void> {
    testing = true;
    // The test step is inserted with its text, so it carries no live region:
    // its start and outcome are spoken here, at the edges.
    announce('Testing the connection…');
    try {
      testResult = await awsApi.test(a.id);
      if (testResult.ok) void aws.loadPermissions(a.id, true);
    } catch (e) {
      testResult = {
        ok: false,
        latency_ms: 0,
        message: e instanceof Error ? e.message : String(e),
        login_required: false,
      };
    } finally {
      testing = false;
    }
    if (testResult?.ok) announce(`Connected in ${testResult.latency_ms} ms`);
    else if (testResult) announce(`Connection test failed. ${testResult.message}`, { assertive: true });
  }

  function finish(): void {
    if (saved) toasts.success(editing ? 'Account updated' : 'Account added', saved.name);
    onclose();
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (error) announce(error, { assertive: true }); });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":editing ? `Edit ${init?.name ?? 'account'}` : 'Add AWS account',"width":620,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(step === 1){
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!step1Valid,"onclick":() => (step = 2),});  }
    } else if (step === 2){
      if(!editing){ { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (step = 1),});  }}
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":!step2Valid || busy,"onclick":() => void saveAndTest(),...__sveltets_2_empty({"data-testid":`aws-wizard-save`}),});
        busy ? 'Saving…' : 'Save & test';
       }
    }else{
      if(saved && !testResult?.ok){
         { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => saved && void runTest(saved),"disabled":testing,});  }
         { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (step = 2),});  }
      }
      
       { svelteHTML.createElement("button", {    "class":`btn`,"onclick":finish,});!(testResult?.login_required && saved?.auth_mode === 'profile');  }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`wiz`,...__sveltets_2_empty({"data-testid":`aws-account-wizard`}),});
     { svelteHTML.createElement("ol", {   "class":`steps`,"aria-label":`Steps`,});
       { svelteHTML.createElement("li", {  });step === 1;step > 1;   }
       { svelteHTML.createElement("li", {  });step === 2;step > 2;   }
       { svelteHTML.createElement("li", { });step === 3;   }
     }

    if(step === 1){
       { const $$_sbaT2C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT2C({ target: __sveltets_2_any(), props: {        "label":`Credential source`,"tabs":MODE_TABS,"value":mode,"onchange":(id) => (mode = id),}});}

      if(mode === 'profile'){
         { svelteHTML.createElement("p", { "class":`hint`,});
               { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }
                        
                 
         }
        if(profiles.length > 6){
           { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`in`,"type":`search`,"placeholder":`Filter profiles…`,"bind:value":profileFilter,"aria-label":`Filter profiles`,});/*Ωignore_startΩ*/() => profileFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        }
         { svelteHTML.createElement("ul", {   "class":`profiles`,"aria-busy":profilesLoading,});
          if(profilesLoading){
             { svelteHTML.createElement("li", { "class":`dim`,});  }
          }else{
               for(let p of __sveltets_2_ensureArray(filteredProfiles)){p.name + p.source;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("button", {    "class":`prof`,"onclick":() => pickProfile(p),});profile === p.name;
                   { svelteHTML.createElement("span", { "class":`pname mono`,});p.name; }
                   { svelteHTML.createElement("span", { "class":`phint`,});hint(p); }
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
                 }
               }
            }
               { svelteHTML.createElement("li", { "class":`dim`,});  profileFilter ? ' for that filter' : ' in ~/.aws';        }
            
          }
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`in mono`,"bind:value":profile,"placeholder":`default`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => profile = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }else{
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});   }
           { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`in mono`,"bind:value":accessKeyId,"placeholder":`AKIA…`,"autocomplete":`off`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => accessKeyId = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  editing ? ' (leave blank to keep)' : ''; }
           { svelteHTML.createElement("input", {        "class":`in mono`,"type":`password`,"bind:value":secret,"autocomplete":`new-password`,});/*Ωignore_startΩ*/() => secret = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});   } }
           { svelteHTML.createElement("input", {        "class":`in mono`,"type":`password`,"bind:value":sessionToken,"autocomplete":`off`,});/*Ωignore_startΩ*/() => sessionToken = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("p", { "class":`hint`,});             }
         { svelteHTML.createElement("details", {   "class":`adv`,"bind:open":advancedOpen,});/*Ωignore_startΩ*/() => advancedOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("summary", {});  }
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});  } }
             { svelteHTML.createElement("input", {                "dir":`ltr`,"class":`in mono`,"type":`url`,"bind:value":endpointUrl,"placeholder":`https://…`,"spellcheck":`false`,"autocomplete":`off`,...__sveltets_2_empty({"data-testid":`aws-wizard-endpoint`}),});/*Ωignore_startΩ*/() => endpointUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("span", { "class":`hint`,});  { svelteHTML.createElement("code", {});  }             { svelteHTML.createElement("code", {});  }      }
           }
         }
      }

       { svelteHTML.createElement("div", { "class":`row2`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
          if(aws.regions.length){
             { svelteHTML.createElement("select", {   "class":`in`,"bind:value":region,});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/
                 for(let r of __sveltets_2_ensureArray(aws.regions)){r.code;
                 { svelteHTML.createElement("option", { "value":r.code,});r.code;  r.name; }
              }
             }
          }else{
             { svelteHTML.createElement("input", {        "dir":`ltr`,"class":`in mono`,"bind:value":region,"placeholder":`us-east-1`,});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          }
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});    { svelteHTML.createElement("em", {});  } }
           { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`in mono`,"bind:value":roleArn,"placeholder":`arn:aws:iam::123456789012:role/Admin`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => roleArn = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
    } else if (step === 2){
      if(!auth.isRoot){ { svelteHTML.createElement("p", { "class":`dim`,});              }}
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("input", {          "dir":`auto`,"class":`in`,"bind:value":name,"placeholder":`prod-eu / sandbox / data-lake`,...__sveltets_2_empty({"data-testid":`aws-wizard-name`}),});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`row2`,});
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("div", {     "class":`env-row`,"role":`radiogroup`,"aria-label":`Environment`,});
               for(let e of __sveltets_2_ensureArray((['dev', 'staging', 'prod'] as Environment[]))){e;
               { svelteHTML.createElement("button", {                 "role":`radio`,"aria-checked":environment === e,"tabindex":environment === e ? 0 : -1,"onkeydown":radioKey,"class":`env-chip`,"disabled":!auth.isRoot,"onclick":() => (environment = e),});environment === e;e === 'prod';e; }
            }
           }
          if(environment === 'prod'){
             { svelteHTML.createElement("span", { "class":`hint danger`,});                }
          }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("div", {     "class":`colors`,"role":`radiogroup`,"aria-label":`Color`,});
                for(let c of __sveltets_2_ensureArray(COLORS)){let ci = 1;c.value;
               { svelteHTML.createElement("button", {                    "role":`radio`,"aria-checked":color === c.value,"tabindex":color === c.value || (ci === 0 && !COLORS.some((x) => x.value === color)) ? 0 : -1,"onkeydown":radioKey,"class":`sw`,"style":`background:${c.value}`,"onclick":() => (color = c.value),"aria-label":`Color ${c.name}`,"title":c.name,});color === c.value; }
            }
           }
         }
       }
      if(editing && auth.isRoot){
         { svelteHTML.createElement("details", { "class":`adv`,});
           { svelteHTML.createElement("summary", {});   }
           { svelteHTML.createElement("div", { "class":`row2`,});
             { svelteHTML.createElement("label", { "class":`field`,});
               { svelteHTML.createElement("span", {});  }
              if(aws.regions.length){
                 { svelteHTML.createElement("select", {   "class":`in`,"bind:value":region,});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/
                     for(let r of __sveltets_2_ensureArray(aws.regions)){r.code; { svelteHTML.createElement("option", { "value":r.code,});r.code;  r.name; }}
                 }
              }else{
                 { svelteHTML.createElement("input", {      "dir":`ltr`,"class":`in mono`,"bind:value":region,});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/}
              }
             }
            if(mode === 'profile'){
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {      "dir":`ltr`,"class":`in mono`,"bind:value":profile,});/*Ωignore_startΩ*/() => profile = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
            }else{
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});   } { svelteHTML.createElement("input", {      "dir":`ltr`,"class":`in mono`,"bind:value":accessKeyId,});/*Ωignore_startΩ*/() => accessKeyId = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});     } { svelteHTML.createElement("input", {        "class":`in mono`,"type":`password`,"bind:value":secret,"autocomplete":`new-password`,});/*Ωignore_startΩ*/() => secret = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});   } { svelteHTML.createElement("input", {      "class":`in mono`,"type":`password`,"bind:value":sessionToken,});/*Ωignore_startΩ*/() => sessionToken = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
            }
             { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});   } { svelteHTML.createElement("input", {      "dir":`ltr`,"class":`in mono`,"bind:value":roleArn,});/*Ωignore_startΩ*/() => roleArn = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});    } } { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`in mono`,"type":`url`,"bind:value":endpointUrl,"placeholder":`http://localhost:4566`,"spellcheck":`false`,...__sveltets_2_empty({"data-testid":`aws-wizard-endpoint`}),});/*Ωignore_startΩ*/() => endpointUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
           }
         }
      }
       { svelteHTML.createElement("div", { "class":`summary`,});
         { svelteHTML.createElement("span", {   "class":`dot`,"style":`background:${color}`,}); }
         { svelteHTML.createElement("strong", {});name || 'Unnamed'; }
         { const $$_egdaBvnE3C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE3C({ target: __sveltets_2_any(), props: {  "env":environment,}});}
         { svelteHTML.createElement("span", { "class":`mono dim`,});mode === 'profile' ? `profile ${profile}` : `keys ${accessKeyId}`;  region; }
        if(endpointUrl.trim()){ { svelteHTML.createElement("span", { "class":`mono dim`,}); endpointUrl.trim(); }}
       }
    }else{
       { svelteHTML.createElement("div", { "class":`test`,});
        if(testing){
           { svelteHTML.createElement("p", {}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 12px`,"aria-hidden":`true`,}); }   { svelteHTML.createElement("code", {});  }  }
        } else if (testResult?.ok){
           { svelteHTML.createElement("p", { "class":`ok`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":14,}});}   testResult.latency_ms;  }
          if(testResult.identity){
             { svelteHTML.createElement("dl", { "class":`idn`,});
               { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});testResult.identity.account; }
               { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});testResult.identity.arn; }
             }
          }
           { svelteHTML.createElement("p", { "class":`dim`,});               }
        } else if (testResult){
           { svelteHTML.createElement("p", { "class":`bad`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} testResult.message; }
          if(testResult.login_required && saved){
            if(saved.auth_mode === 'profile'){
               { svelteHTML.createElement("p", { "class":`dim`,});       { svelteHTML.createElement("code", {});   }  }
               { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => saved && onsignin(saved),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}    }
            }else{
               { svelteHTML.createElement("p", { "class":`dim`,});          }
            }
          }
        }
       }
    }

    if(error){ { svelteHTML.createElement("p", { "class":`err`,});error; }}
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AccountWizard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AccountWizard__SvelteComponent_ = ReturnType<typeof AccountWizard__SvelteComponent_>;
/*Ωignore_endΩ*/export default AccountWizard__SvelteComponent_;