///<reference types="svelte" />
;// The Permissions tab — a READ-ONLY preview. Editable grants (an agent
// identity with folders, sites, channels and tools set to allow / ask /
// deny, plan §2.5) arrive in a later phase and the daemon has no grants
// route yet, so this page says what is true today instead of showing
// controls that do nothing.

import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import type { Tone } from '../../lib/assistant/model';
function $$render() {

  
  
  
  
  
  
  
  

  const TODAY: { icon: IconName; resource: string; mode: string; tone: Tone; note: string }[] = [
    { icon: 'send', resource: 'Send, post, publish or submit', mode: 'Ask', tone: 'warn', note: 'An approval card shows where it goes, what is sent and who sees it' },
    { icon: 'hand', resource: 'Always allow a destination', mode: 'Ask', tone: 'warn', note: 'Offered per destination and tool, from the approval card itself' },
    { icon: 'lock', resource: 'Purchases', mode: 'Ask', tone: 'warn', note: 'Never “Always allow”' },
    { icon: 'db', resource: 'Prod connections', mode: 'Ask', tone: 'warn', note: 'Never “Always allow”' },
    { icon: 'key', resource: 'Site logins', mode: 'Keychain', tone: 'neutral', note: 'Site Credentials are used by name, never pasted into prompts' },
    { icon: 'eyeOff', resource: 'Incognito threads', mode: 'No memory', tone: 'neutral', note: 'Nothing is recalled or remembered; deleted 24 h after the last turn' },
  ];
;
async () => {

 { svelteHTML.createElement("div", {   "class":`perm`,...__sveltets_2_empty({"data-testid":`assistant-permissions`}),});
   { svelteHTML.createElement("div", {   "class":`banner`,"role":`note`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":14,}});}
     { svelteHTML.createElement("p", {});
       { svelteHTML.createElement("strong", {});   }
                            
          
     }
   }

   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("h2", { "class":`h`,});  }
     { svelteHTML.createElement("p", { "class":`help`,});                                }
   }

   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("h2", { "class":`h`,});    }
     { svelteHTML.createElement("p", { "class":`help`,});           }
     { svelteHTML.createElement("div", { "class":`table-wrap`,});
       { svelteHTML.createElement("table", {});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", {   "scope":`col`,"class":`note-col`,});  } }
         }
         { svelteHTML.createElement("tbody", {});
             for(let r of __sveltets_2_ensureArray(TODAY)){r.resource;
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("td", {});
                 { svelteHTML.createElement("span", { "class":`res`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":r.icon,"size":14,}});}r.resource; }
                
                 { svelteHTML.createElement("span", { "class":`note-stack dim`,});r.note; }
               }
               { svelteHTML.createElement("td", {}); { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "tone":r.tone,"label":r.mode,}});} }
               { svelteHTML.createElement("td", { "class":`note-col dim`,});r.note; }
             }
          }
         }
       }
     }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const PermissionsTab__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type PermissionsTab__SvelteComponent_ = InstanceType<typeof PermissionsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default PermissionsTab__SvelteComponent_;