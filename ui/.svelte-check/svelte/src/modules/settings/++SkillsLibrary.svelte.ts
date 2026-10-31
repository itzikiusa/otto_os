///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { toastError } from '../../lib/toastError';
import { sectionLabel } from './sections';
import SectionIntro from './SectionIntro.svelte';
import { router } from '../../lib/router.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
// Settings → Skills: the catalogue of skills that ship with Otto ("bundled"),
// shown grouped by category with their state relative to the installed library
// copy. Installing/updating writes into the Otto library; the backend always
// backs up an existing copy before overwriting (backup=true), so nothing is
// ever destroyed silently. It ALSO materializes a copy into each provider CLI's
// global skills dir (~/.claude/skills, $CODEX_HOME/skills, ~/.gemini/skills) so
// the skill is discoverable everywhere; Remove reconciles those too (it only
// ever touches skills Otto's per-dir manifest owns). Install/update/install-all
// are root-only on the server; this page is gated to root in Settings.svelte
// (like Context Library). The "Update" button (shown when state is
// `update_available`/`ahead`) posts to the same install endpoint.
import { contextApi } from '../../lib/api/context';
import type { BundledSkill, BundledSkillState } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let skills: BundledSkill[] = $state([]);
  let loading = $state(true);
  // A failed load is shown inline with Retry — never as "No bundled skills".
  let loadError = $state('');
  // Names with an in-flight install/update/remove (disables that row's buttons).
  let busy: Set<string> = $state(new Set());
  // Category currently running "Install all" (disables that header button).
  let busyCategory: string | null = $state(null);

  // Preferred category ordering; anything else falls in afterwards, alphabetically.
  const CATEGORY_ORDER = [
    'product',
    'project',
    'development',
    'review',
    'design',
    'insights',
  ];

  // Name/description filter — two dozen skills in a few categories.
  let query = $state('');
  const q = $derived(query.trim().toLowerCase());
  const shown = $derived(
    q ? skills.filter((s) => s.name.toLowerCase().includes(q) || (s.description ?? '').toLowerCase().includes(q)) : skills,
  );

  // Skills grouped by category, in CATEGORY_ORDER then alphabetical for the rest.
  const groups = $derived.by(() => {
    const byCat = new Map<string, BundledSkill[]>();
    for (const s of shown) {
      const list = byCat.get(s.category) ?? [];
      list.push(s);
      byCat.set(s.category, list);
    }
    const cats = [...byCat.keys()].sort((a, b) => {
      const ia = CATEGORY_ORDER.indexOf(a);
      const ib = CATEGORY_ORDER.indexOf(b);
      if (ia !== -1 && ib !== -1) return ia - ib;
      if (ia !== -1) return -1;
      if (ib !== -1) return 1;
      return a.localeCompare(b);
    });
    return cats.map((cat) => ({
      category: cat,
      skills: (byCat.get(cat) ?? []).sort((a, b) => a.name.localeCompare(b.name)),
    }));
  });

  // ---------------------------------------------------------------------------
  // Load
  // ---------------------------------------------------------------------------

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      skills = await contextApi.listBundled();
      loadError = '';
    } catch (e) {
      // Keep the last good list (if any); LoadState shows a stale bar over it.
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function setBusy(name: string, on: boolean): void {
    const next = new Set(busy);
    if (on) next.add(name);
    else next.delete(name);
    busy = next;
  }

  // ---------------------------------------------------------------------------
  // Badge / action labels per state
  // ---------------------------------------------------------------------------

  // Not installed carries no chip: the Install button already says so, and a
  // grey "Not installed" on every row of a fresh library was pure noise.
  function badge(s: BundledSkill): { text: string; cls: string } | null {
    switch (s.state) {
      case 'not_installed':
        return null;
      case 'up_to_date':
        return { text: `Installed v${s.installed_version}`, cls: 'ok' };
      case 'update_available':
        return {
          text: `Update available · v${s.installed_version} → v${s.version}`,
          cls: 'info',
        };
      case 'ahead':
        return { text: 'Edited locally', cls: 'warn' };
      default:
        return { text: s.state, cls: '' };
    }
  }

  // ---------------------------------------------------------------------------
  // Install / update one skill
  // ---------------------------------------------------------------------------

  async function install(s: BundledSkill): Promise<void> {
    // "ahead" means the installed copy was hand-edited and is newer than the
    // bundled one — updating discards those edits (after a backup). Make the
    // keep-old-vs-sync choice explicit; doing nothing keeps the edited copy.
    if (s.state === 'ahead') {
      const ok = await confirmer.ask(
        `Your installed copy of “${s.name}” was edited and is ahead of the bundled version. ` +
          `Updating backs it up first, then replaces it with the bundled skill. ` +
          `Do nothing to keep your edited copy.`,
        { title: 'Replace edited skill?', confirmLabel: 'Back up & replace', danger: true },
      );
      if (!ok) return;
    }

    setBusy(s.name, true);
    try {
      const resp = await contextApi.installBundled(s.name);
      if (resp.backed_up) {
        toasts.success(
          'Backed up & updated',
          resp.backup_path
            ? `${s.name} — previous copy saved to ${resp.backup_path}`
            : `${s.name} — previous copy backed up`,
        );
      } else {
        toasts.success('Installed', s.name);
      }
      noteUserOwned(resp.user_owned ?? []);
      await load();
    } catch (e) {
      toastError(`Couldn’t install ${s.name}`, e);
    } finally {
      setBusy(s.name, false);
    }
  }

  // ---------------------------------------------------------------------------
  // Remove an installed skill
  // ---------------------------------------------------------------------------

  async function remove(s: BundledSkill): Promise<void> {
    if (
      !(await confirmer.ask(`Remove “${s.name}” from your library and from each agent CLI’s skills folder? You can install it again from this page.`, {
        title: 'Remove skill',
        danger: true, confirmLabel: 'Remove',
      }))
    )
      return;
    setBusy(s.name, true);
    try {
      await contextApi.deleteSkill(s.name);
      toasts.info('Removed', s.name);
      await load();
    } catch (e) {
      toastError(`Couldn’t remove ${s.name}`, e);
    } finally {
      setBusy(s.name, false);
    }
  }

  /** An agent CLI already had its own skill of that name: Otto left it alone. */
  function noteUserOwned(paths: string[]): void {
    if (paths.length === 0) return;
    toasts.info(
      'Kept your own skills',
      `These agent CLI skill folders already hold a skill you own, so Otto did not replace them:\n${paths.join('\n')}`,
    );
  }

  // ---------------------------------------------------------------------------
  // Install all in a category
  // ---------------------------------------------------------------------------

  async function installAll(category: string): Promise<void> {
    busyCategory = category;
    try {
      const resp = await contextApi.installAllBundled(category);
      const n = resp.installed.length;
      const b = resp.backed_up.length;
      const failed = resp.failed ?? [];
      const skipped = resp.skipped?.length ?? 0;
      if (failed.length > 0) {
        // Name every failure: the rest of the batch still installed.
        toasts.error(
          `${plural(failed.length, `${category} skill`)} didn’t install`,
          failed.map((f) => `${f.name}: ${f.error}`).join('\n') + (n > 0 ? `\nInstalled ${plural(n, 'other')}.` : ''),
        );
      } else if (n === 0) {
        toasts.info(
          'Nothing to install',
          skipped > 0
            ? `Library copies of these ${category} skills are already current, or ahead with local edits, so those files were left as they are. Their provider copies were still reconciled.`
            : `All ${category} skills are already up to date.`,
        );
      } else if (b > 0) {
        toasts.success(
          `Installed ${plural(n, `${category} skill`)}`,
          `${b} existing cop${b === 1 ? 'y was' : 'ies were'} backed up first.`,
        );
      } else {
        toasts.success(`Installed ${plural(n, `${category} skill`)}`);
      }
      noteUserOwned(resp.user_owned ?? []);
      await load();
    } catch (e) {
      toastError(`Couldn’t install the ${category} skills`, e);
    } finally {
      busyCategory = null;
    }
  }

  // True when a category has at least one skill that would change on "Install all".
  function categoryHasWork(skillsInCat: BundledSkill[]): boolean {
    return skillsInCat.some((s) => s.state !== 'up_to_date');
  }

  // Per-state action button label. "Replace…" asks first (edited copy).
  function actionLabel(state: BundledSkillState, name: string): string {
    if (busy.has(name)) return state === 'not_installed' ? 'Installing…' : 'Updating…';
    if (state === 'not_installed') return 'Install';
    return state === 'ahead' ? 'Replace…' : 'Update';
  }

  function installedCount(list: BundledSkill[]): number {
    return list.filter((s) => s.state !== 'not_installed').length;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('skills'),"subtitle":`Skills that ship with Otto`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});                                     { svelteHTML.createElement("strong", {});    }       { svelteHTML.createElement("button", {     "type":`button`,"class":`intro-link`,"onclick":() => router.go('skills-eval'),});  }       { svelteHTML.createElement("button", {     "type":`button`,"class":`intro-link`,"onclick":() => router.go('settings/context-library'),});  }  SectionIntro}

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`bundled skills`,loading,"error":loadError,"empty":skills.length === 0,"rows":5,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`box`,"title":`No bundled skills`,"body":`This build of Otto doesn’t ship any skills. Add your own in Settings → Context library.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("label", { "class":`filter`,});
         { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
         { svelteHTML.createElement("input", {                  "dir":`ltr`,"type":`search`,"class":`filter-input`,"bind:value":query,"placeholder":`Filter skills…`,"aria-label":`Filter skills`,"autocomplete":`off`,"spellcheck":`false`,"onkeydown":(e) => { if (e.key === 'Escape' && query) { e.preventDefault(); e.stopPropagation(); query = ''; } },});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("span", { "class":`summary`,});installedCount(skills);  skills.length;  }
     }
    if(groups.length === 0){
       { svelteHTML.createElement("p", { "class":`no-match`,});   query.trim();  }
    }
       for(let g of __sveltets_2_ensureArray(groups)){g.category;
       { svelteHTML.createElement("section", {   "class":`cat`,"aria-labelledby":`cat-${g.category}`,});
         { svelteHTML.createElement("div", { "class":`cat-head`,});
           { svelteHTML.createElement("h2", {   "class":`section-title`,"id":`cat-${g.category}`,});g.category; }
           { svelteHTML.createElement("span", { "class":`cat-count`,});installedCount(g.skills);  g.skills.length;  }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {         "class":`btn small`,"disabled":busyCategory === g.category || !categoryHasWork(g.skills),"title":categoryHasWork(g.skills) ? `Install or update every ${g.category} skill` : `Every ${g.category} skill is up to date`,"onclick":() => installAll(g.category),});
            busyCategory === g.category ? 'Installing…' : 'Install all';
           }
         }

         { svelteHTML.createElement("div", { "class":`skill-list`,});
             for(let s of __sveltets_2_ensureArray(g.skills)){s.name;
            const b = badge(s);
             { svelteHTML.createElement("div", { "class":`skill`,});
               { svelteHTML.createElement("div", { "class":`grow`,});
                 { svelteHTML.createElement("div", { "class":`skill-name`,});
                   { svelteHTML.createElement("span", { "class":`mono`,});s.name; }
                  if(b){ { svelteHTML.createElement("span", { "class":`chip ${b.cls}`,});b.text; }}
                 }
                if(s.description){
                   { svelteHTML.createElement("div", {   "class":`skill-desc dim`,"title":s.description,});s.description; }
                }
                if(s.state === 'update_available'){
                   { svelteHTML.createElement("div", { "class":`note dim`,});       }
                } else if (s.state === 'ahead'){
                   { svelteHTML.createElement("div", { "class":`note warn`,});
                                   
                   }
                }
               }

               { svelteHTML.createElement("div", { "class":`skill-actions`,});
                if(s.state === 'up_to_date'){
                   { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":busy.has(s.name),"onclick":() => remove(s),});
                    busy.has(s.name) ? 'Removing…' : 'Remove…';
                   }
                }else{
                   { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":busy.has(s.name),"onclick":() => install(s),});
                    actionLabel(s.state, s.name);
                   }
                }
               }
             }
          }
         }
       }
    }
   LoadState}
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillsLibrary__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillsLibrary__SvelteComponent_ = ReturnType<typeof SkillsLibrary__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillsLibrary__SvelteComponent_;