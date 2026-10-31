///<reference types="svelte" />
;// FilesPanel: multi-section file tree container (up to 4 sections).
// Each section is an independent FileTree rooted at its own folder, with
// folder navigation; it starts at the Side panel's folder (the session's
// cwd, or the workspace folder picked in the panel).

import { rightPanel } from '../../lib/stores/rightPanel.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { sessionFs } from '../../lib/api/fs';
import FileTree from './FileTree.svelte';
import Icon from '../../lib/components/Icon.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  

  // ── section state ─────────────────────────────────────────────────────────

  interface Section {
    id: number;
    /** undefined → the Side panel's folder */
    root: string | undefined;
  }

  let nextId = $state(1);

  let sections: Section[] = $state([{ id: nextId++, root: undefined }]);

  const MAX = 4;

  // Inside an agent session the tree is the session's cwd. Engine-made cwds
  // (workflow worktrees, persona folders, goal-loop / scheduled / swarm work
  // dirs) live in Otto's data dir, which `/fs/*` refuses non-root callers, so
  // the session-scoped routes back it up (PC-1, access-control-4-03). One
  // adapter per session, so the tree doesn't reload on every render.
  const sessionSource = $derived.by(() => {
    const s = ws.activeSession;
    return rightPanel.session && s?.cwd ? sessionFs(s.id, s.cwd) : null;
  });

  function addSection(): void {
    if (sections.length >= MAX) return;
    sections = [...sections, { id: nextId++, root: undefined }];
  }

  function removeSection(id: number): void {
    if (sections.length <= 1) return;
    sections = sections.filter((s) => s.id !== id);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`fp-wrap`,});
  
   { svelteHTML.createElement("div", { "class":`fp-toolbar`,});
     { svelteHTML.createElement("span", { "class":`fp-count dim`,});sections.length;  MAX; }
     { svelteHTML.createElement("button", {           "class":`icon-btn fp-add-btn`,"disabled":sections.length >= MAX,"title":sections.length >= MAX ? `Up to ${MAX} file sections` : 'Add file section',"aria-label":`Add file section`,"onclick":addSection,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}
       { svelteHTML.createElement("span", { "class":`fp-add-label`,});  }
     }
   }

  
   { svelteHTML.createElement("div", { "class":`fp-sections`,});
        for(let section of __sveltets_2_ensureArray(sections)){let i = 1;section.id;
      if(i > 0){
         { svelteHTML.createElement("div", { "class":`fp-divider`,}); }
      }
       { svelteHTML.createElement("div", { "class":`fp-section`,});
         { const $$_eerTeliF3C = __sveltets_2_ensureComponent(FileTree); new $$_eerTeliF3C({ target: __sveltets_2_any(), props: {            "root":section.root ?? (rightPanel.folder || undefined),"primary":i === 0,"navigation":true,"lister":sessionSource?.lister,"reader":sessionSource?.reader,"onClose":sections.length > 1 ? () => removeSection(section.id) : undefined,}});}
       }
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FilesPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FilesPanel__SvelteComponent_ = ReturnType<typeof FilesPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default FilesPanel__SvelteComponent_;