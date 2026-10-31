///<reference types="svelte" />
;
import { untrack } from 'svelte';
import Badge from '../../lib/components/Badge.svelte';
// Per-session agent activity: a normalized task tracker + a live trail of
// what's going on (skills loaded, commands run, files touched, prompts,
// notes) — by user and by agent. Fed by REST load + the events WS.
// Supports source filtering, search, level coloring and expandable detail.
//
// The task tracker is two-way (docs/design/conversation-view.md §4.5/§5.4):
// the agent's own plan (TodoWrite/TaskCreate, `source:'agent'`) is merged with
// tasks pushed from here or the Mission Control board (`source:'user'`, shown
// with a `from board` badge). A user task waits as *queued* until the nudge
// sweep hands it to the agent's PTY (idle, or after the 120 s max defer).
import { ws } from '../../lib/stores/workspace.svelte';
import { toastError } from '../../lib/toastError';
import { activity } from '../../lib/stores/activity.svelte';
import { rel } from '../../lib/stores/now.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import type { TaskStatus, TrailKind, TrailSource } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const session = $derived(ws.activeSession);
  const wsId = $derived(ws.currentId);

  const tasks = $derived(activity.tasks(session?.id ?? null));
  const trail = $derived(activity.trail(session?.id ?? null));

  const doneCount = $derived(tasks.filter((t) => t.status === 'completed').length);
  const progress = $derived(tasks.length ? Math.round((doneCount / tasks.length) * 100) : 0);
  const pendingNudges = $derived(tasks.filter((t) => t.nudge_pending).length);
  // Codex has no task tool, so its list is only ever what users add here.
  const isCodex = $derived(session?.provider === 'codex');
  const canEdit = $derived(ws.myRole !== 'viewer' && session?.kind === 'agent');
  const sessionEnded = $derived(
    !!session && (ws.statusMap[session.id] ?? session.status) === 'exited',
  );

  // "+ Add task" inline form.
  let addingTask = $state(false);
  let taskTitle = $state('');
  let taskDesc = $state('');
  let taskBusy = $state(false);
  let taskTitleEl = $state<HTMLInputElement | null>(null);

  function openAddTask(): void {
    addingTask = true;
    queueMicrotask(() => taskTitleEl?.focus());
  }

  function cancelAddTask(): void {
    addingTask = false;
    taskTitle = '';
    taskDesc = '';
  }

  async function submitTask(): Promise<void> {
    const title = taskTitle.trim();
    if (!title || !session || taskBusy) return;
    taskBusy = true;
    try {
      await activity.addTask(session.id, title, taskDesc.trim() || undefined);
      cancelAddTask();
    } catch (e) {
      toastError('Couldn’t add task', e);
    } finally {
      taskBusy = false;
    }
  }

  function onTaskKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void submitTask();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelAddTask();
    }
  }

  // Filters: source + free-text search. Newest first for at-a-glance scanning.
  type SourceFilter = 'all' | 'agent' | 'user' | 'otto';
  let sourceFilter = $state<SourceFilter>('all');
  let query = $state('');

  const filtered = $derived(
    [...trail]
      .reverse()
      .filter((e) => sourceFilter === 'all' || e.source === sourceFilter)
      .filter((e) => {
        const q = query.trim().toLowerCase();
        return q === '' || e.summary.toLowerCase().includes(q);
      }),
  );

  // Load once whenever the focused session changes (live updates arrive via WS).
  $effect(() => {
    const sid = session?.id;
    const w = wsId;
    if (sid && w) return untrack(() => activity.watch(w, sid));
  });

  const loadError = $derived(session ? (activity.loadErrorBySession[session.id] ?? null) : null);

  let note = $state('');
  let adding = $state(false);
  let expanded = $state<Record<string, boolean>>({});

  function toggle(id: string): void {
    expanded[id] = !expanded[id];
  }

  async function addNote(): Promise<void> {
    const text = note.trim();
    if (!text || !session || !wsId || adding) return;
    adding = true;
    try {
      await activity.addNote(wsId, session.id, text);
      note = '';
    } catch (e) {
      toastError('Couldn’t add note', e);
    } finally {
      adding = false;
    }
  }

  function onNoteKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void addNote();
    }
  }

  const KIND_ICON: Record<TrailKind, IconName> = {
    session: 'play',
    prompt: 'comment',
    skill: 'zap',
    command: 'command',
    tool: 'box',
    file: 'file',
    web: 'globe',
    task: 'check',
    note: 'note',
    other: 'dot',
  };

  const TASK_GLYPH: Record<TaskStatus, string> = {
    pending: '○',
    in_progress: '◐',
    completed: '●',
    blocked: '▲',
    cancelled: '✕',
  };

  const SOURCE_TABS: { id: SourceFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'agent', label: 'Agent' },
    { id: 'user', label: 'You' },
    { id: 'otto', label: 'Otto' },
  ];

  function sourceLabel(s: TrailSource): string {
    return s === 'user' ? 'you' : s === 'otto' ? 'otto' : 'agent';
  }

  function pretty(detail: unknown): string {
    try {
      return JSON.stringify(detail, null, 2);
    } catch {
      return String(detail);
    }
  }
;
async () => {

if(!session){
   { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`zap`,"title":`No session selected`,"body":`Open or focus an agent session to follow its live trail and task tracker.`,}});}
}else{
   { svelteHTML.createElement("div", { "class":`activity`,});
    if(loadError){
      
       { svelteHTML.createElement("div", { "class":`load-state`,});
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           "what":`this session’s activity`,"variant":`compact`,"error":loadError,"empty":tasks.length === 0 && trail.length === 0,"onretry":() => session && wsId && void activity.load(wsId, session.id, true),}});}
       }
    }
    
     { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("div", { "class":`section-title`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", { "class":`title-right`,});
          if(pendingNudges > 0){
             { svelteHTML.createElement("span", {   "class":`count nudge`,"title":`Waiting to be handed to the agent (delivered when idle, or after 120 s)`,});
              pendingNudges; 
             }
          }
          if(tasks.length > 0){ { svelteHTML.createElement("span", {   "class":`count`,...__sveltets_2_empty({"data-testid":`task-count`}),});doneCount; tasks.length; }}
          if(canEdit && !addingTask){
             { svelteHTML.createElement("button", {         "class":`add-task-btn`,"onclick":openAddTask,"title":sessionEnded ? 'The session has exited — the task is kept and handed over on resume' : 'Push a task to this agent',...__sveltets_2_empty({"data-testid":`add-task-btn`}),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
             }
          }
         }
       }

      if(addingTask){
         { svelteHTML.createElement("form", {     "class":`task-add`,"onsubmit":(e) => { e.preventDefault(); void submitTask(); },...__sveltets_2_empty({"data-testid":`add-task-form`}),});
           { const $$_input3 = svelteHTML.createElement("input", {               "dir":`auto`,"class":`task-input`,"placeholder":`Task title — the agent adds it to its list and does it next`,"bind:value":taskTitle,"onkeydown":onTaskKeydown,"spellcheck":`false`,"aria-label":`Task title`,});/*Ωignore_startΩ*/() => taskTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/taskTitleEl = $$_input3;}
           { svelteHTML.createElement("textarea", {                "dir":`auto`,"class":`task-input desc`,"placeholder":`Details (optional) — sent along with the nudge`,"bind:value":taskDesc,"onkeydown":onTaskKeydown,"rows":2,"spellcheck":`false`,"aria-label":`Task description`,});/*Ωignore_startΩ*/() => taskDesc = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           { svelteHTML.createElement("div", { "class":`task-add-row`,});
             { svelteHTML.createElement("button", {     "type":`button`,"class":`tbtn`,"onclick":cancelAddTask,});  }
             { svelteHTML.createElement("button", {     "type":`submit`,"class":`tbtn primary`,"disabled":taskTitle.trim() === '' || taskBusy,});
              taskBusy ? 'Adding…' : 'Add';
             }
           }
         }
      }

      if(tasks.length === 0){
        if(isCodex){
           { svelteHTML.createElement("p", { "class":`empty-line dim`,});
                              
           }
        }else{
           { svelteHTML.createElement("p", { "class":`empty-line dim`,});           }
        }
      }else{
         { svelteHTML.createElement("div", {   "class":`progress-track`,"aria-hidden":`true`,});
           { svelteHTML.createElement("div", {   "class":`progress-fill`,"style":`width:${progress}%`,}); }
         }
         { svelteHTML.createElement("ul", {   "class":`tasks`,...__sveltets_2_empty({"data-testid":`task-list`}),});
             for(let t of __sveltets_2_ensureArray(tasks)){t.id;
             { svelteHTML.createElement("li", {           "class":`task task-${t.status}`,"title":t.description ? `${t.status.replace('_', ' ')} — ${t.description}` : t.status.replace('_', ' '),...__sveltets_2_empty({"data-testid":`task-row`}),...__sveltets_2_empty({"data-source":t.source}),});t.source === 'user';t.nudge_pending;
               { svelteHTML.createElement("span", { "class":`task-glyph`,});TASK_GLYPH[t.status]; }
               { svelteHTML.createElement("span", { "class":`task-title`,});t.title; }
              if(t.source === 'user'){
                 { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {      "tone":`accent`,"label":`From board`,"title":`Added from the board / Activity panel`,}});}
              }
              if(t.nudge_pending){
                 { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {      "tone":`warn`,"label":`Queued`,"title":`Waiting to be handed to the agent`,}});}
              }
             }
          }
         }
        if(isCodex){
           { svelteHTML.createElement("p", { "class":`empty-line dim hint`,});               }
        }
      }
     }

    
     { svelteHTML.createElement("section", { "class":`section grow`,});
       { svelteHTML.createElement("div", { "class":`section-title`,});
         { svelteHTML.createElement("span", {});  }
        if(trail.length > 0){ { svelteHTML.createElement("span", { "class":`count`,});trail.length; }}
       }

       { svelteHTML.createElement("div", { "class":`filters`,});
         { svelteHTML.createElement("div", { "class":`tabs`,});
             for(let t of __sveltets_2_ensureArray(SOURCE_TABS)){t.id;
             { svelteHTML.createElement("button", {        "class":`tab`,"aria-pressed":sourceFilter === t.id,"onclick":() => (sourceFilter = t.id),});sourceFilter === t.id;
              t.label;
             }
          }
         }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`search`,"placeholder":`Filter…`,"aria-label":`Filter the trail`,"bind:value":query,"spellcheck":`false`,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }

       { svelteHTML.createElement("div", { "class":`note-add`,});
         { svelteHTML.createElement("input", {              "dir":`auto`,"class":`note-input`,"placeholder":`Waiting on the staging deploy`,"aria-label":`Note`,"bind:value":note,"onkeydown":onNoteKeydown,"spellcheck":`false`,});/*Ωignore_startΩ*/() => note = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {           "class":`note-btn`,"title":note.trim() === '' ? 'Type a note first' : 'Add note',"aria-label":`Add note`,"disabled":note.trim() === '' || adding,"onclick":addNote,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}
         }
       }

      if(filtered.length === 0){
         { svelteHTML.createElement("p", { "class":`empty-line dim`,});
          trail.length === 0 ? 'No activity recorded yet.' : 'Nothing matches this filter.';
         }
      }else{
         { svelteHTML.createElement("ul", { "class":`trail`,});
             for(let e of __sveltets_2_ensureArray(filtered)){e.id;
             { svelteHTML.createElement("li", { "class":`trail-row src-${e.source} kind-${e.kind} lvl-${e.level}`,});
                const rowMain/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
                 { svelteHTML.createElement("span", { "class":`row-icon`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[e.kind] ?? 'dot',"size":12,}});} }
                 { svelteHTML.createElement("span", { "class":`row-body`,});
                   { svelteHTML.createElement("span", { "class":`row-summary`,});e.summary; }
                   { svelteHTML.createElement("span", { "class":`row-meta`,});
                     { svelteHTML.createElement("span", { "class":`row-src`,});sourceLabel(e.source); }
                     { svelteHTML.createElement("span", {   "class":`row-time mono`,"title":new Date(e.ts).toLocaleString(),});rel(e.ts); }
                    if(e.detail != null){
                       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":expanded[e.id] ? 'chevronDown' : 'chevronRight',"size":12,}});}
                    }
                   }
                 }
              };return __sveltets_2_any(0)};
              
              if(e.detail != null){
                 { svelteHTML.createElement("button", {         "type":`button`,"class":`row-main clickable`,"aria-expanded":!!expanded[e.id],"onclick":() => toggle(e.id),});;__sveltets_2_ensureSnippet(rowMain()); }
              }else{
                 { svelteHTML.createElement("div", { "class":`row-main`,});;__sveltets_2_ensureSnippet(rowMain()); }
              }
              if(e.detail != null && expanded[e.id]){
                 { svelteHTML.createElement("pre", { "class":`row-detail mono`,});pretty(e.detail); }
              }
             }
          }
         }
      }
     }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ActivityPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ActivityPanel__SvelteComponent_ = ReturnType<typeof ActivityPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ActivityPanel__SvelteComponent_;