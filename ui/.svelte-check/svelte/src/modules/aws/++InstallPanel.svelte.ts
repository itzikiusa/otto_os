///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
// First-run panel: the daemon can't find the `aws` CLI. Offers "Install now"
// (Admin on `aws`), shows the installer's progress + a collapsible log tail
// while `/aws/status` is polled every 1.5 s (the store re-arms the poll on
// each response), and auto-continues when the binary appears.
import { aws } from '../../lib/stores/aws.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import Icon from '../../lib/components/Icon.svelte';
function $$render() {

  
  
  
  
  
  
  
  

  const job = $derived(aws.status?.install ?? null);
  const running = $derived(job?.state === 'running');
  const failed = $derived(job?.state === 'failed');
  const canInstall = $derived(auth.can('aws', 'admin'));
  let logOpen = $state(false);

  async function install(): Promise<void> {
    try {
      await aws.startInstall();
    } catch (e) {
      toastError('Couldn’t start the install', e);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`install`,...__sveltets_2_empty({"data-testid":`aws-install-panel`}),});
   { svelteHTML.createElement("div", { "class":`card`,});
     { svelteHTML.createElement("div", { "class":`ico`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`cloud`,"size":26,}});} }
     { svelteHTML.createElement("h2", {});     }
     { svelteHTML.createElement("p", { "class":`body`,});
             { svelteHTML.createElement("code", {});  }         
          { svelteHTML.createElement("code", {});  }        
              { svelteHTML.createElement("code", {});  }  
       { svelteHTML.createElement("code", {});  }
     }

    if(running){
       { svelteHTML.createElement("div", {       "class":`indeterminate`,"role":`progressbar`,"aria-label":`Installing the AWS CLI`,"aria-busy":`true`,}); }
       { svelteHTML.createElement("p", { "class":`status`,});        }
    } else if (failed){
       { svelteHTML.createElement("p", { "class":`status err`,});
            job?.error ? `: ${job.error}` : '';
       }
    }

     { svelteHTML.createElement("div", { "class":`actions`,});
      if(canInstall){
         { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":() => void install(),"disabled":running || aws.installBusy,});
          running ? 'Installing…' : failed ? 'Retry install' : 'Install now';
         }
      }else{
         { svelteHTML.createElement("span", { "class":`dim`,});          { svelteHTML.createElement("code", {});  }  }
      }
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => void aws.loadStatus(),});  }
     }

    if(job?.log_tail){
       { svelteHTML.createElement("button", {     "class":`log-toggle`,"onclick":() => (logOpen = !logOpen),"aria-expanded":logOpen,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":logOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
         
       }
      if(logOpen){
         { svelteHTML.createElement("pre", { "class":`log`,});job.log_tail; }
      }
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const InstallPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type InstallPanel__SvelteComponent_ = ReturnType<typeof InstallPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default InstallPanel__SvelteComponent_;