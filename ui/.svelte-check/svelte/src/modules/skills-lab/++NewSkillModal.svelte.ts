///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { loadErrorText } from '../../lib/loadError';
// "New skill" sheet with a small template picker:
//   • Blank           — a SKILL.md scaffold (frontmatter + When to use / Method)
//   • From bundled    — start from a bundled skill's SKILL.md under a new name
//   • Import a file   — a skill package .zip (the daemon unpacks it)
// Importing from a URL isn't offered: the daemon has no fetch-a-package
// endpoint, and fetching arbitrary URLs from the webview would bypass the
// SSRF guard.
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { skillLabApi } from '../../lib/api/skillLab';
import type { BundledSkillView, LibrarySkill } from '../../lib/api/types';
;

  type Template = 'blank' | 'bundled' | 'import';;;

  interface Props {
    bundled: BundledSkillView[];
    categories: string[];
    /** Existing names, to catch a clash before the daemon's 409. */
    taken: Set<string>;
    initial?: Template;
    onclose: () => void;
    oncreated: (s: LibrarySkill) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  


  let { bundled, categories, taken, initial = 'blank', onclose, oncreated }: Props = $props();

  // The sheet is re-created per open, so the initial template is read once.
  // svelte-ignore state_referenced_locally
  let template = $state<Template>(initial);
  let name = $state('');
  let category = $state('development');
  let description = $state('');
  let fromBundled = $state('');
  let file = $state<File | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let touched = $state(false);

  const NAME_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
  const nameError = $derived.by(() => {
    if (template === 'import') return null;
    const n = name.trim();
    if (!n) return touched ? 'Give the skill a name.' : null;
    if (!NAME_RE.test(n)) return 'Use lowercase letters, digits and dashes (kebab-case), up to 64 characters.';
    if (taken.has(n)) return `A skill named “${n}” already exists in the library.`;
    return null;
  });
  // Tooltip for the disabled Create button (the inline name error only shows
  // after the field is touched).
  const blockReason = $derived(
    template === 'import'
      ? file
        ? ''
        : 'Choose a .zip skill package'
      : template === 'bundled' && !fromBundled
        ? 'Choose the bundled skill to start from'
        : !name.trim()
          ? 'Give the skill a name'
          : (nameError ?? ''),
  );
  const valid = $derived(
    template === 'import' ? !!file : !!name.trim() && !nameError && (template !== 'bundled' || !!fromBundled),
  );

  function pickBundled(n: string): void {
    fromBundled = n;
    const b = bundled.find((x) => x.name === n);
    if (b) {
      if (!name.trim()) name = `${b.name}-custom`;
      category = b.category || category;
      if (!description.trim()) description = b.description;
    }
  }

  function blankBody(): string {
    const n = name.trim();
    const d = description.trim() || 'What this skill does and when an agent should use it.';
    return `---\nname: ${n}\ndescription: ${d}\ncategory: ${category.trim() || 'development'}\nversion: 1\n---\n\n# ${n}\n\n## When to use\n\n${d}\n\n## Method\n\n1. \n2. \n3. \n\n## Output\n\nWhat the agent hands back, and in what shape.\n`;
  }

  async function create(): Promise<void> {
    touched = true;
    if (!valid || busy) return;
    busy = true;
    error = null;
    try {
      let s: LibrarySkill;
      if (template === 'import' && file) {
        s = await skillLabApi.importZip(file);
      } else if (template === 'bundled') {
        const src = await skillLabApi.getBundled(fromBundled);
        // Keep the method, rename it: rewrite `name:`/`description:` when present.
        let body = src.body.replace(/^name:.*$/m, `name: ${name.trim()}`);
        if (description.trim()) body = body.replace(/^description:.*$/m, `description: ${description.trim().replace(/\n/g, ' ')}`);
        // The Overview reads the category from the frontmatter first, so a
        // changed category must land there too or it looks ignored.
        if (category.trim()) body = body.replace(/^category:.*$/m, `category: ${category.trim()}`);
        s = await skillLabApi.create({ name: name.trim(), category: category.trim(), description: description.trim(), body });
      } else {
        s = await skillLabApi.create({ name: name.trim(), category: category.trim(), description: description.trim(), body: blankBody() });
      }
      oncreated(s);
    } catch (e) {
      const conflict = (e as { status?: unknown })?.status === 409 || /already exists/i.test(e instanceof Error ? e.message : '');
      error = conflict ? `A skill with that name already exists. Choose another name.` : loadErrorText(e);
    } finally {
      busy = false;
    }
  }

  const TEMPLATES: { id: Template; icon: 'file' | 'box' | 'download'; title: string; body: string }[] = [
    { id: 'blank', icon: 'file', title: 'Blank', body: 'A SKILL.md scaffold: frontmatter, when to use, method, output.' },
    { id: 'bundled', icon: 'box', title: 'From a bundled skill', body: 'Copy one of Otto’s skills under a new name and adapt it.' },
    { id: 'import', icon: 'download', title: 'Import a file', body: 'A skill package (.zip) with SKILL.md at its root.' },
  ];
  // Radiogroup keyboard: ←/→ (and ↑/↓) move the choice, as native radios do.
  let tplEl = $state<HTMLElement | null>(null);
  function onTplKey(e: KeyboardEvent): void {
    const i = TEMPLATES.findIndex((t) => t.id === template);
    let n = -1;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') n = (i + 1) % TEMPLATES.length;
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') n = (i - 1 + TEMPLATES.length) % TEMPLATES.length;
    if (n < 0) return;
    e.preventDefault();
    template = TEMPLATES[n].id;
    queueMicrotask(() => (tplEl?.querySelectorAll('[role="radio"]')[n] as HTMLElement | undefined)?.focus());
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (error) announce(`Couldn’t create the skill. ${error}`, { assertive: true }); });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`New skill`,"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {         "class":`btn primary`,"onclick":create,"disabled":!valid || busy,"title":!valid ? blockReason : undefined,...__sveltets_2_empty({"data-testid":`create-skill`}),});
      busy ? (template === 'import' ? 'Importing…' : 'Creating…') : template === 'import' ? 'Import skill' : 'Create skill';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { const $$_div1 = svelteHTML.createElement("div", {          "class":`templates`,"role":`radiogroup`,"aria-label":`Start from`,"tabindex":-1,"onkeydown":onTplKey,});tplEl = $$_div1;
       for(let t of __sveltets_2_ensureArray(TEMPLATES)){t.id;
       { svelteHTML.createElement("button", {              "class":`tpl`,"role":`radio`,"aria-checked":template === t.id,"tabindex":template === t.id ? 0 : -1,"onclick":() => (template = t.id),...__sveltets_2_empty({"data-testid":`tpl-${t.id}`}),});template === t.id;
         { svelteHTML.createElement("span", { "class":`tpl-icon`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":t.icon,"size":16,}});} }
         { svelteHTML.createElement("span", { "class":`tpl-title`,});t.title; }
         { svelteHTML.createElement("span", { "class":`tpl-body`,});t.body; }
       }
    }
   }

  if(template === 'import'){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ns-file`,});  }
       { svelteHTML.createElement("input", {          "id":`ns-file`,"class":`input file`,"type":`file`,"accept":`.zip`,"onchange":(e) => (file = (e.currentTarget as HTMLInputElement).files?.[0] ?? null),});}
       { svelteHTML.createElement("span", { "class":`hint`,});            }
     }
  }else{
    if(template === 'bundled'){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ns-src`,});  }
         { svelteHTML.createElement("select", {       "id":`ns-src`,"class":`input`,"value":fromBundled,"onchange":(e) => pickBundled((e.currentTarget as HTMLSelectElement).value),});
           { svelteHTML.createElement("option", {"value":"","disabled":true,});   }
             for(let b of __sveltets_2_ensureArray(bundled)){b.name; { svelteHTML.createElement("option", { "value":b.name,});b.name;  b.category; }}
         }
       }
    }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ns-name`,});  }
       { svelteHTML.createElement("input", {                  "dir":`ltr`,"id":`ns-name`,"class":`input mono`,"bind:value":name,"onblur":() => (touched = true),"placeholder":`release-notes-writer`,"aria-invalid":!!nameError,"aria-describedby":`ns-name-hint`,...__sveltets_2_empty({"data-testid":`new-skill-name`}),});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(nameError){
         { svelteHTML.createElement("span", {   "class":`err`,"id":`ns-name-hint`,});nameError; }
      }else{
         { svelteHTML.createElement("span", {   "class":`hint`,"id":`ns-name-hint`,});       }
      }
     }
     { svelteHTML.createElement("div", { "class":`row2`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ns-cat`,});  }
         { svelteHTML.createElement("input", {            "dir":`auto`,"id":`ns-cat`,"class":`input`,"list":`ns-cats`,"bind:value":category,"placeholder":`review`,});/*Ωignore_startΩ*/() => category = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("datalist", { "id":`ns-cats`,});   for(let c of __sveltets_2_ensureArray(categories)){c; { svelteHTML.createElement("option", { "value":c,}); }} }
       }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ns-desc`,});  }
       { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`ns-desc`,"class":`input`,"rows":2,"bind:value":description,"placeholder":`Draft release notes from the PRs merged since the last tag.`,});/*Ωignore_startΩ*/() => description = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("span", { "class":`hint`,});                 }
     }
  }
  if(error){ { svelteHTML.createElement("p", { "class":`err`,});    error; }}

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NewSkillModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NewSkillModal__SvelteComponent_ = ReturnType<typeof NewSkillModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default NewSkillModal__SvelteComponent_;