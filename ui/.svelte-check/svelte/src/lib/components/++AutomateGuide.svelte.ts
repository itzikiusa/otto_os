///<reference types="svelte" />
;
  /** The five Automate modules (each renders the guide on its empty state). */
  export type AutomateModule = 'swarm' | 'loops' | 'workflows' | 'scheduled-tasks' | 'personal-agents';
  /** Every entry the chooser lists: the Automate modules plus the two
   *  overlapping automations that live elsewhere (S20-18) — Assistant tasks &
   *  reminders (overlap Scheduled Tasks) and Run with Otto (overlaps Goal
   *  Loops and Workflows' PR runs). */
  export type AutomateGuideId = AutomateModule | 'assistant-tasks' | 'run-with-otto';

  export const AUTOMATE_GUIDE: readonly { id: AutomateGuideId; route: string; label: string; when: string }[] = [
    { id: 'workflows', route: 'workflows', label: 'Workflows', when: 'A fixed sequence of steps you can draw — run on demand, from a trigger, or on a schedule.' },
    { id: 'scheduled-tasks', route: 'scheduled-tasks', label: 'Scheduled Tasks', when: 'One prompt on a cadence that delivers a report (daily digest, weekly check).' },
    { id: 'assistant-tasks', route: 'assistant/tasks', label: 'Assistant tasks & reminders', when: 'A quick “remind me” or one-off errand you ask for in the Assistant chat — no form, no cadence to set up.' },
    { id: 'run-with-otto', route: 'run-with-otto', label: 'Run with Otto', when: 'One ticket, issue or finding: an agent works it on a branch, proves it, and waits for your approval before any PR.' },
    { id: 'loops', route: 'loops', label: 'Goal Loops', when: 'One measurable goal: an agent iterates on a branch until it is met or the budget runs out.' },
    { id: 'swarm', route: 'swarm', label: 'Swarm', when: 'A team of role agents working a project board together — bigger, multi-task work.' },
    { id: 'personal-agents', route: 'personal-agents', label: 'Personal Agents', when: 'A named persona with memory and its own schedules that you talk to over time.' },
  ];
;;// "Which one do I want?" — the Automate modules (and the Assistant's tasks
// and Run with Otto) overlap: each can run an agent on its own. Every Automate
// empty state carries this one-line-each chooser, collapsed as a quiet
// secondary under the single primary CTA; each Automate page's ⋯ also opens
// it expanded in a sheet (AutomateGuideButton), so a user with one item
// still finds it.


import { router } from '../router.svelte';
import { auth } from '../stores/auth.svelte';
import { routeAllowed } from '../sidebar';

;type $$ComponentProps =  {
    current: AutomateModule;
    /** Start expanded (the ⋯ sheet) instead of collapsed (empty states). */
    open?: boolean;
    /** Called before navigating (the sheet closes itself). */
    onnavigate?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let {
    current,
    open = false,
    onnavigate,
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // Only the entries this user can actually open (S20-307): the same RBAC /
  // feature gate the sidebar applies — the current page always stays listed.
  const entries = $derived(
    AUTOMATE_GUIDE.filter((m) => m.id === current || routeAllowed(m.route, (f) => auth.can(f, 'view'))),
  );
;
async () => {



 { svelteHTML.createElement("details", {    "class":`automate-guide`,...__sveltets_2_empty({"data-testid":`automate-guide`}),open,});
   { svelteHTML.createElement("summary", {});     }
   { svelteHTML.createElement("ul", {});
       for(let m of __sveltets_2_ensureArray(entries)){m.id;
       { svelteHTML.createElement("li", { });m.id === current;
        if(m.id === current){
           { svelteHTML.createElement("strong", {});m.label; }  { svelteHTML.createElement("span", { "class":`here-tag`,});  }
        }else{
           { svelteHTML.createElement("button", {     "type":`button`,"class":`link`,"onclick":() => { onnavigate?.(); router.go(m.route); },});m.label; }
        }
         { svelteHTML.createElement("span", { "class":`when`,}); m.when; }
       }
    }
   }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AutomateGuide__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AutomateGuide__SvelteComponent_ = ReturnType<typeof AutomateGuide__SvelteComponent_>;
/*Ωignore_endΩ*/export default AutomateGuide__SvelteComponent_;