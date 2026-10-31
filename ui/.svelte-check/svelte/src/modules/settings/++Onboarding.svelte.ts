///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
// First-run wizard: welcome → root password → first workspace → tools → done.
import { tick } from 'svelte';
import { api, setToken, ApiError } from '../../lib/api/client';
import type { LoginResp, Workspace } from '../../lib/api/types';
import { auth, ONBOARDING_RESUME_KEY } from '../../lib/stores/auth.svelte';
import { ssSet, ssRemove } from '../../lib/storage';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import type { FsBrowse } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  // A reload after the root account exists resumes here (auth boot reads the
  // tab's saved step); the account is signed in already, so Finish skips it.
  const resumed = auth.onboardingResumeStep !== null;
  let step = $state(auth.onboardingResumeStep ?? 0);
  $effect(() => {
    if (step >= 2 && (rootLogin || resumed)) ssSet(ONBOARDING_RESUME_KEY, String(step));
  });
  let card = $state<HTMLElement | null>(null);
  $effect(() => {
    const currentStep = step;
    void tick().then(() => {
      if (step === currentStep) card?.querySelector<HTMLElement>('h1')?.focus();
    });
  });
  let password = $state('');
  let password2 = $state('');
  let displayName = $state('');
  let wsName = $state('');
  let wsPath = $state('');
  let skipWorkspace = $state(false);
  let busy = $state(false);
  let error = $state('');
  // Root creation is irreversible; retain its result if optional workspace
  // setup fails so Retry cannot attempt to create the account again.
  let rootLogin = $state<LoginResp | null>(null);

  /** Length in code points — the daemon counts `chars()`, so an emoji is one
   *  character here too (UTF-16 `length` counted it as two and enabled the
   *  button for a password the server then refused). */
  const pwLen = $derived([...password].length);
  const strength = $derived.by(() => {
    let score = 0;
    if (pwLen >= 10) score++;
    if (pwLen >= 14) score++;
    if (/[A-Z]/.test(password) && /[a-z]/.test(password)) score++;
    if (/\d/.test(password)) score++;
    if (/[^A-Za-z0-9]/.test(password)) score++;
    return score; // 0..5
  });
  const strengthLabel = $derived(
    ['too short', 'weak', 'fair', 'good', 'strong', 'excellent'][strength] ?? 'weak',
  );
  const pwValid = $derived(pwLen >= 10 && password === password2);

  // ClickHouse powers usage tracking; surface whether it's already present so
  // the user knows what to expect (install happens from the Usage dashboard
  // after setup, since it needs an authenticated session).
  const clickhouse = $derived(auth.meta?.tools?.find((t) => t.name === 'clickhouse'));

  // Detected agent CLIs (claude / codex / agy). Otto can't run an agent session
  // without one, so the tool-check step calls this out specifically — the
  // first-run coach on the Agents page then walks the user to a launched agent.
  const agentTools = $derived(
    (auth.meta?.tools ?? []).filter((t) => t.name === 'claude' || t.name === 'codex' || t.name === 'agy'),
  );
  const hasAgentCli = $derived(agentTools.some((t) => t.found));

  /** Create the root account (once) and authenticate with it. Runs when the
   *  password step is confirmed, so the workspace step can browse and check
   *  folders on the daemon (`/fs/browse` needs a signed-in user). */
  async function ensureRoot(): Promise<LoginResp> {
    if (!rootLogin) {
      try {
        rootLogin = await api.post<LoginResp>('/onboarding/root', {
          password,
          display_name: displayName.trim() === '' ? null : displayName.trim(),
        });
      } catch (e) {
        // The account may exist even if its response was lost. Authenticate
        // with the just-entered credentials; never replace an existing root.
        if (e instanceof ApiError && e.status !== 409 && e.status < 500) throw e;
        try {
          rootLogin = await api.post<LoginResp>('/auth/login', { username: 'root', password });
        } catch {
          // A 409 whose credentials don't sign in means another root already
          // exists (set up elsewhere): retrying here can never succeed, so the
          // step offers the sign-in screen instead of an "already onboarded" loop.
          if (e instanceof ApiError && e.status === 409) rootTaken = true;
          throw e;
        }
      }
    }
    setToken(rootLogin.token);
    return rootLogin;
  }

  /** A raw fetch failure ("Load failed" / "Failed to fetch") means the daemon
   *  isn't answering — say that, not the browser's words. */
  function failureText(e: unknown): string {
    if (!(e instanceof ApiError)) return 'Otto’s background service isn’t responding — it may still be starting. Try again in a moment.';
    return e.message;
  }

  let pwError = $state('');
  let rootTaken = $state(false);
  async function confirmPassword(): Promise<void> {
    if (busy || !pwValid) return;
    busy = true;
    pwError = '';
    rootTaken = false;
    try {
      await ensureRoot();
      step = 2;
    } catch (e) {
      pwError = rootTaken
        ? 'A root account already exists on this Otto daemon. Sign in with its password instead.'
        : `Couldn’t create the account: ${failureText(e)}`;
    } finally {
      busy = false;
    }
  }

  // Workspace folder: picked from the daemon's file system, and checked
  // before moving on — a typo used to surface only as a failed "Finish".
  let browsing = $state(false);
  let pathError = $state('');
  let checkingPath = $state(false);
  async function confirmWorkspace(): Promise<void> {
    if (checkingPath) return;
    checkingPath = true;
    pathError = '';
    try {
      const view = await api.get<FsBrowse>(`/fs/browse?path=${encodeURIComponent(wsPath.trim())}`);
      wsPath = view.path;
      skipWorkspace = false;
      step = 3;
    } catch (e) {
      pathError = `Otto can’t open that folder: ${e instanceof Error ? e.message : String(e)}. Check the path, or choose it with Browse.`;
    } finally {
      checkingPath = false;
    }
  }

  async function finish(): Promise<void> {
    if (busy) return;
    busy = true;
    error = '';
    let login: LoginResp | null = null;
    try {
      // Resumed after a reload: the account exists and this tab is signed in.
      if (!resumed || rootLogin) login = await ensureRoot();
    } catch (e) {
      error = failureText(e);
      busy = false;
      return;
    }
    // The workspace is OPTIONAL: a failure here must not block Finish (it
    // used to keep failing, with the error on the tool-check step). Finish
    // anyway and say how to recover — the Agents first-run coach creates one.
    let wsFailed: unknown = null;
    if (!skipWorkspace && wsName.trim() !== '' && wsPath.trim() !== '') {
      try {
        await api.post<Workspace>('/workspaces', { name: wsName.trim(), root_path: wsPath.trim() });
      } catch (e) {
        wsFailed = e;
      }
    }
    try {
      ssRemove(ONBOARDING_RESUME_KEY);
      auth.onboardingResumeStep = null;
      if (auth.meta) auth.meta.needs_onboarding = false;
      if (login) await auth.acceptLogin(login);
      else await auth.boot();
    } catch (e) {
      error = failureText(e);
      busy = false;
      return;
    }
    if (wsFailed) {
      toasts.error('Setup finished, but the workspace wasn’t created', `${failureText(wsFailed)} — create it from the Agents page.`);
    }
    busy = false;
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (rootLogin) announce('Root account created.'); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`ob-wrap`,});
   { const $$_div1 = svelteHTML.createElement("div", {  "class":`ob-card card`,});card = $$_div1;
     { svelteHTML.createElement("div", {     "class":`ob-progress`,"role":`img`,"aria-label":`Setup step ${step + 1} of 5`,});
          for(let _ of __sveltets_2_ensureArray(Array(5))){let i = 1;i;
         { svelteHTML.createElement("span", {  "class":`ob-dot`,});i <= step; }
      }
     }

    if(step === 0){
       { svelteHTML.createElement("div", { "class":`ob-body`,});
         { svelteHTML.createElement("div", { "class":`ob-mark`,});  }
         { svelteHTML.createElement("h1", { "tabindex":-1,});   }
         { svelteHTML.createElement("p", {});
                   
                       
                   
         }
         { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => (step = 1),});  }
       }
    } else if (step === 1){
       { svelteHTML.createElement("div", { "class":`ob-body`,});
         { svelteHTML.createElement("h1", { "tabindex":-1,});    }
         { svelteHTML.createElement("p", {});         }
        
         { svelteHTML.createElement("form", {     "class":`ob-form`,"onsubmit":(e) => {
            e.preventDefault();
            if (pwValid && !busy && !rootTaken) void confirmPassword();
          },});

         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`ob-name`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ob-name`,"class":`input`,"bind:value":displayName,"placeholder":`Root`,});/*Ωignore_startΩ*/() => displayName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`ob-pass`,});  }
           { svelteHTML.createElement("input", {            "id":`ob-pass`,"aria-describedby":`ob-strength`,"class":`input`,"type":`password`,"bind:value":password,"autocomplete":`new-password`,});/*Ωignore_startΩ*/() => password = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("div", { "class":`strength`,});
             { svelteHTML.createElement("div", { "class":`strength-bar`,});
               { svelteHTML.createElement("div", {     "class":`strength-fill s${strength}`,"style":`width: ${Math.min(100, strength * 20)}%`,}); }
             }
             { svelteHTML.createElement("span", {   "id":`ob-strength`,"class":`hint`,});
              pwLen < 10 ? `At least 10 characters (${pwLen}/10)` : strengthLabel;
             }
           }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`ob-pass2`,});  }
           { svelteHTML.createElement("input", {              "id":`ob-pass2`,"aria-describedby":`ob-mismatch`,"aria-invalid":password2.length > 0 && password !== password2,"class":`input`,"type":`password`,"bind:value":password2,"autocomplete":`new-password`,});/*Ωignore_startΩ*/() => password2 = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {     "id":`ob-mismatch`,"class":`hint err`,"role":`status`,});password2.length > 0 && password !== password2 ? "Passwords don’t match" : ''; }
         }

        if(pwError){ { svelteHTML.createElement("div", {   "class":`hint err`,"role":`alert`,});pwError; }}

         { svelteHTML.createElement("div", { "class":`ob-actions`,});
           { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":busy,"onclick":() => (step = 0),});  }
          if(rootTaken){
             { svelteHTML.createElement("button", {     "type":`button`,"class":`btn primary`,"onclick":() => void auth.boot(),});    }
          }else{
             { svelteHTML.createElement("button", {     "type":`submit`,"class":`btn primary`,"disabled":!pwValid || busy,});
              busy ? 'Creating the account…' : pwError ? 'Try again' : 'Create account';
             }
          }
         }
         }
       }
    } else if (step === 2){
       { svelteHTML.createElement("div", { "class":`ob-body`,});
         { svelteHTML.createElement("h1", { "tabindex":-1,});    }
        if(rootLogin || resumed){ { svelteHTML.createElement("p", { "class":`ob-done`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}    }}
         { svelteHTML.createElement("p", {});             }
         { svelteHTML.createElement("form", {     "class":`ob-form`,"onsubmit":(e) => {
            e.preventDefault();
            if (wsName.trim() !== '' && wsPath.trim() !== '' && !checkingPath) void confirmWorkspace();
          },});

         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`ob-wsname`,});  }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ob-wsname`,"class":`input`,"bind:value":wsName,"placeholder":`my-project`,});/*Ωignore_startΩ*/() => wsName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`ob-wspath`,});  }
           { svelteHTML.createElement("div", { "class":`ob-path-row`,});
             { svelteHTML.createElement("input", {                   "id":`ob-wspath`,"dir":`ltr`,"class":`input mono ob-path-input`,"bind:value":wsPath,"oninput":() => (pathError = ''),"placeholder":`/Users/you/code/my-project`,"spellcheck":`false`,"aria-invalid":pathError !== '',"aria-describedby":`ob-path-err`,});/*Ωignore_startΩ*/() => wsPath = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("button", {     "type":`button`,"class":`btn`,"onclick":() => (browsing = true),});  }
           }
           { svelteHTML.createElement("span", {     "id":`ob-path-err`,"class":`hint err`,"role":`status`,});pathError; }
         }

         { svelteHTML.createElement("div", { "class":`ob-actions`,});
          
          if(!rootLogin && !resumed){ { svelteHTML.createElement("button", {     "type":`button`,"class":`btn`,"onclick":() => (step = 1),});  }}
           { svelteHTML.createElement("button", {     "type":`button`,"class":`btn ghost`,"onclick":() => { skipWorkspace = true; step = 3; },});  }
           { svelteHTML.createElement("button", {       "type":`submit`,"class":`btn primary`,"disabled":wsName.trim() === '' || wsPath.trim() === '' || checkingPath,});
            checkingPath ? 'Checking…' : 'Continue';
           }
         }
         }
       }
    } else if (step === 3){
       { svelteHTML.createElement("div", { "class":`ob-body`,});
         { svelteHTML.createElement("h1", { "tabindex":-1,});  }
         { svelteHTML.createElement("p", {});
                     
           { svelteHTML.createElement("strong", {});  }      { svelteHTML.createElement("em", {});  }    
                     
           { svelteHTML.createElement("span", { "class":`mono`,});  }
         }

         { svelteHTML.createElement("div", { "class":`tools`,});
           { svelteHTML.createElement("div", { "class":`tool-row`,});
             { svelteHTML.createElement("span", {  "class":`tool-status`,});clickhouse?.found;
              if(clickhouse?.found){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}else{ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}}
             }
             { svelteHTML.createElement("span", { "class":`mono`,});  }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("span", { "class":`dim`,});clickhouse?.found ? (clickhouse.version ?? 'found') : 'not installed'; }
           }
         }
         { svelteHTML.createElement("p", { "class":`hint-line`,});
          if(clickhouse?.found){       }else{        { svelteHTML.createElement("strong", {});    }
             { svelteHTML.createElement("span", { "class":`mono`,});    } }
         }

         { svelteHTML.createElement("div", { "class":`ob-actions`,});
           { svelteHTML.createElement("button", {     "type":`button`,"class":`btn`,"onclick":() => (step = 2),});  }
           { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => (step = 4),});  }
         }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`ob-body`,});
         { svelteHTML.createElement("h1", { "tabindex":-1,});  }
         { svelteHTML.createElement("p", {});             }

         { svelteHTML.createElement("div", { "class":`tools`,});
             for(let t of __sveltets_2_ensureArray(auth.meta?.tools ?? [])){t.name;
             { svelteHTML.createElement("div", { "class":`tool-row`,});
               { svelteHTML.createElement("span", {  "class":`tool-status`,});t.found;
                if(t.found){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}else{ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}}
               }
               { svelteHTML.createElement("span", { "class":`mono`,});t.name; }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("span", { "class":`dim`,});t.found ? (t.version ?? 'found') : 'not found'; }
             }
          }
             { svelteHTML.createElement("div", { "class":`dim`,});   }
          
         }

         { svelteHTML.createElement("p", { "class":`hint-line`,});
          if(hasAgentCli){             
               }else{     
             { svelteHTML.createElement("span", { "class":`mono`,});    }     
                }
         }

        if(error){ { svelteHTML.createElement("div", {   "class":`hint err`,"role":`alert`,});error; }}

         { svelteHTML.createElement("div", { "class":`ob-actions`,});
           { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":busy,"onclick":() => (step = 3),});  }
           { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy,"onclick":finish,});
            busy ? 'Setting up…' : 'Finish setup';
           }
         }
       }
    }
   }
 }

if(browsing){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Choose the workspace folder`,"start":wsPath.trim(),"onpick":(path) => {
      wsPath = path;
      pathError = '';
      if (wsName.trim() === '') wsName = path.split('/').filter(Boolean).pop() ?? '';
      browsing = false;
    },"onclose":() => (browsing = false),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Onboarding__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Onboarding__SvelteComponent_ = ReturnType<typeof Onboarding__SvelteComponent_>;
/*Ωignore_endΩ*/export default Onboarding__SvelteComponent_;