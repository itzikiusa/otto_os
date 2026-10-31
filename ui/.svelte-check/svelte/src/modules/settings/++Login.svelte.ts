///<reference types="svelte" />
;
import { auth } from '../../lib/stores/auth.svelte';
import { ApiError } from '../../lib/api/client';
function $$render() {

  
  

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (busy) return;
    busy = true;
    error = '';
    try {
      await auth.login(username, password);
    } catch (err) {
      // Only a network failure is "unreachable": a 429 / 403 is the daemon
      // answering, and saying so tells the person what to do next.
      error = !(err instanceof ApiError)
        ? 'Couldn’t sign in — daemon unreachable?'
        : err.status === 401
          ? 'Wrong username or password.'
          : err.status === 429
            ? 'Too many sign-in attempts. Wait a minute, then try again.'
            : err.status === 403
              ? 'This account can’t sign in here (disabled, or not allowed from this address).'
              : `Couldn’t sign in (${err.status}). ${err.message}`;
    } finally {
      busy = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`login-wrap`,});
   { svelteHTML.createElement("form", {   "class":`login-card card`,"onsubmit":submit,});
     { svelteHTML.createElement("div", { "class":`login-mark`,});  }
     { svelteHTML.createElement("p", { "class":`login-sub`,});      }

     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`login-user`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`login-user`,"class":`input`,"bind:value":username,"autocomplete":`username`,});/*Ωignore_startΩ*/() => username = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`login-pass`,});  }
       { svelteHTML.createElement("input", {           "id":`login-pass`,"class":`input`,"type":`password`,"bind:value":password,"autocomplete":`current-password`,});/*Ωignore_startΩ*/() => password = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

    if(error){ { svelteHTML.createElement("div", {   "class":`login-error`,"role":`alert`,});error; }}

     { svelteHTML.createElement("button", {     "class":`btn primary login-btn`,"type":`submit`,"disabled":busy || !username || !password,});
      busy ? 'Signing in…' : 'Sign In';
     }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Login__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Login__SvelteComponent_ = ReturnType<typeof Login__SvelteComponent_>;
/*Ωignore_endΩ*/export default Login__SvelteComponent_;