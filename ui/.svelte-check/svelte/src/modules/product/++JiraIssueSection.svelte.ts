///<reference types="svelte" />
;// The Jira Details sidebar of a Jira story's Overview (architecture-5-04):
// status + transition, assignee, the editmeta-driven fields (each with its
// inline `IssueEditor`), links and development info. It renders the
// `JiraIssue` the tab owns (jiraIssue.svelte.ts), starts its load and the
// lazy editmeta fetch, and lazy-loads dev status when Development opens.
// The tab keeps the story body, its comments/history/attachments and the
// collapse state they share with these cards.

import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import IssueEditor from './jira/IssueEditor.svelte';
import type { JiraIssue } from './jiraIssue.svelte';
import type { EditableField, ProductStory } from './types';
;

  interface Props {
    issue: JiraIssue;
    story: ProductStory;
    /** Section key → collapsed (shared with the tab's activity cards). */
    collapsed: Record<string, boolean>;
    ontoggle: (key: string) => void;
  };function $$render() {
 const editBtn/*Ωignore_positionΩ*/ = (key: string, current: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(jira.editableFor(key)){
     { svelteHTML.createElement("button", {           "class":`field-edit-btn`,...__sveltets_2_empty({"data-edit-for":key}),"title":`Edit ${jira.editableFor(key)!.name}`,"aria-label":`Edit ${jira.editableFor(key)!.name}`,"onclick":() => jira.beginEdit(jira.editableFor(key)!, current),});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
     }
  }
};return __sveltets_2_any(0)}; const fieldEditor/*Ωignore_positionΩ*/ = (ef: EditableField)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { const $$_rotidEeussI0C = __sveltets_2_ensureComponent(IssueEditor); new $$_rotidEeussI0C({ target: __sveltets_2_any(), props: {     "editor":jira,ef,"issueKey":story?.source_key ?? '',}});}
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { issue, story, collapsed, ontoggle }: Props = $props();

  const jira = $derived(issue.editor);
  const issueFull = $derived(issue.full);
  const issueLoading = $derived(issue.loading);
  const issueError = $derived(issue.error);
  const devStatus = $derived(issue.dev);
  const devLoading = $derived(issue.devLoading);
  const devError = $derived(issue.devError);

  $effect(() => {
    // `!issueError`: a failed load stays failed until Retry — without it the
    // effect re-fired on every settle and hammered the daemon in a loop.
    if (!issueFull && !issueLoading && !issueError) {
      void issue.load();
    }
  });

  // Once the issue is loaded, lazily fetch editmeta so edit affordances (and the
  // always-on Story Points / Estimate add-row) can render. Reads issueFull only;
  // the mutation of `editmeta` happens inside ensureEditmeta(), never in a $derived.
  $effect(() => {
    if (issueFull && jira.editmeta === null && !jira.editmetaLoading && !jira.editmetaError) {
      void jira.ensureEditmeta();
    }
  });

  function toggle(key: string): void {
    ontoggle(key);
    // Lazy-load development info when the section opens (no-op if already loaded).
    if (key === 'development' && !collapsed[key]) void issue.loadDev();
  }
;
async () => {







   { svelteHTML.createElement("div", { "class":`jira-section`,});
    if(issueLoading){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":6,"height":36,"label":`Jira details`,}});}
    } else if (issueError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`Jira details`,"variant":`compact`,"error":issueError,"empty":true,"onretry":() => void issue.load(),}});}
    } else if (issueFull){

      
       { svelteHTML.createElement("div", { "class":`jira-card`,});
         { svelteHTML.createElement("div", { "class":`jira-card-header`,});
           { svelteHTML.createElement("span", { "class":`jira-section-label`,});  }
           { svelteHTML.createElement("div", { "class":`status-control`,});
             { svelteHTML.createElement("span", { "class":`status-badge`,});issueFull.status; }
             { svelteHTML.createElement("button", {             "class":`change-btn`,"onclick":(e) => void jira.openTransitionMenu(e),"disabled":jira.transitionWorking || jira.transitionsLoading,"title":`Change the status of ${story.source_key} in Jira`,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`ov-transition-btn`}),});
              if(jira.transitionWorking){ } else if (jira.transitionsLoading){ }else{  { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}}
             }
           }
         }
       }

      
       { svelteHTML.createElement("div", { "class":`jira-card`,});
         { svelteHTML.createElement("div", { "class":`jira-card-header`,});
           { svelteHTML.createElement("span", { "class":`jira-section-label`,});  }
           { svelteHTML.createElement("div", { "class":`assignee-control`,});
            if(issueFull.assignee){
               { svelteHTML.createElement("div", { "class":`user-row`,});
                if(issueFull.assignee.avatar_url){
                   { svelteHTML.createElement("img", {      "class":`avatar`,"src":issueFull.assignee.avatar_url,"alt":issueFull.assignee.display_name,});}
                }else{
                   { svelteHTML.createElement("span", { "class":`avatar-placeholder`,});issueFull.assignee.display_name.slice(0, 1).toUpperCase(); }
                }
                 { svelteHTML.createElement("span", { "class":`user-name`,});issueFull.assignee.display_name; }
               }
            }else{
               { svelteHTML.createElement("span", { "class":`unassigned`,});  }
            }
             { svelteHTML.createElement("button", {             "class":`change-btn`,"onclick":(e) => void jira.openAssigneeMenu(e),"disabled":jira.assigneeWorking || jira.assignablesLoading,"title":`Change the assignee of ${story.source_key} in Jira`,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`ov-assignee-btn`}),});
              if(jira.assigneeWorking){ } else if (jira.assignablesLoading){ }else{  { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}}
             }
           }
         }
       }

      
       { svelteHTML.createElement("div", { "class":`jira-card collapsible-card`,});
         { svelteHTML.createElement("button", {       "class":`jira-coll-trigger`,"onclick":() => toggle('details'),"aria-expanded":!collapsed.details,});
           { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":collapsed.details ? 'chevronRight' : 'chevronDown',"size":12,}});} }
           { svelteHTML.createElement("span", { "class":`jira-section-label`,});  }
         }
        if(!collapsed.details){
           { svelteHTML.createElement("div", { "class":`details-grid`,});
            if(issueFull.reporter){
               { svelteHTML.createElement("span", { "class":`detail-key`,});  }
               { svelteHTML.createElement("span", { "class":`detail-val`,});
                if(jira.editingField === 'reporter' && jira.editableFor('reporter')){
                  ;__sveltets_2_ensureSnippet(fieldEditor(jira.editableFor('reporter')!));
                }else{
                   { svelteHTML.createElement("div", { "class":`detail-val-row`,});
                     { svelteHTML.createElement("div", { "class":`user-row-sm`,});
                      if(issueFull.reporter.avatar_url){
                         { svelteHTML.createElement("img", {      "class":`avatar-sm`,"src":issueFull.reporter.avatar_url,"alt":issueFull.reporter.display_name,});}
                      }
                      issueFull.reporter.display_name;
                     }
                    ;__sveltets_2_ensureSnippet(editBtn('reporter', issueFull.reporter.display_name));
                   }
                }
               }
            }

             { svelteHTML.createElement("span", { "class":`detail-key`,});  }
             { svelteHTML.createElement("span", { "class":`detail-val`,});
              if(jira.editingField === 'priority' && jira.editableFor('priority')){
                ;__sveltets_2_ensureSnippet(fieldEditor(jira.editableFor('priority')!));
              }else{
                 { svelteHTML.createElement("div", { "class":`detail-val-row`,});
                   { svelteHTML.createElement("span", {});issueFull.priority ?? '—'; }
                  ;__sveltets_2_ensureSnippet(editBtn('priority', issueFull.priority ?? ''));
                 }
              }
             }

            
            if(jira.estimateField){
               { svelteHTML.createElement("span", { "class":`detail-key`,});jira.estimateField.name; }
               { svelteHTML.createElement("span", { "class":`detail-val`,});
                if(jira.editingField === jira.estimateField.key){
                  ;__sveltets_2_ensureSnippet(fieldEditor(jira.estimateField));
                }else{
                   { svelteHTML.createElement("div", { "class":`detail-val-row`,});
                    if(issueFull.estimate){
                       { svelteHTML.createElement("span", { "class":`estimate-chip`,});issueFull.estimate; }
                    }else{
                       { svelteHTML.createElement("span", { "class":`detail-empty`,});  }
                    }
                    ;__sveltets_2_ensureSnippet(editBtn(jira.estimateField.key, jira.rawFieldValue(jira.estimateField.key)));
                   }
                }
               }
            } else if (issueFull.estimate){
               { svelteHTML.createElement("span", { "class":`detail-key`,});  }
               { svelteHTML.createElement("span", { "class":`detail-val`,}); { svelteHTML.createElement("span", { "class":`estimate-chip`,});issueFull.estimate; } }
            }

             { svelteHTML.createElement("span", { "class":`detail-key`,});  }
             { svelteHTML.createElement("span", { "class":`detail-val`,});
              if(jira.editingField === 'labels' && jira.editableFor('labels')){
                ;__sveltets_2_ensureSnippet(fieldEditor(jira.editableFor('labels')!));
              }else{
                 { svelteHTML.createElement("div", { "class":`detail-val-row`,});
                  if(issueFull.labels && issueFull.labels.length > 0){
                     { svelteHTML.createElement("div", { "class":`label-chips`,});
                         for(let lbl of __sveltets_2_ensureArray(issueFull.labels)){lbl;
                         { svelteHTML.createElement("span", { "class":`label-chip`,});lbl; }
                      }
                     }
                  }else{
                     { svelteHTML.createElement("span", { "class":`detail-empty`,});  }
                  }
                  ;__sveltets_2_ensureSnippet(editBtn('labels', (issueFull.labels ?? []).join(', ')));
                 }
              }
             }

               for(let field of __sveltets_2_ensureArray(issueFull.fields.filter((f) => f.value && f.value.trim() !== ''))){field.key;
              if(field.key === jira.estimateField?.key || (jira.estimateField === null && /story\s*point/i.test(field.name))){
                
              }else{
                 { svelteHTML.createElement("span", { "class":`detail-key`,});field.name; }
                 { svelteHTML.createElement("span", { "class":`detail-val`,});
                  if(jira.editingField === field.key && jira.editableFor(field.key)){
                    ;__sveltets_2_ensureSnippet(fieldEditor(jira.editableFor(field.key)!));
                  }else{
                     { svelteHTML.createElement("div", { "class":`detail-val-row`,});
                       { svelteHTML.createElement("span", {});field.value; }
                      ;__sveltets_2_ensureSnippet(editBtn(field.key, field.value));
                     }
                  }
                 }
              }
            }

            
            if(jira.editmetaError){
               { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {         "what":`editable fields`,"variant":`compact`,"error":jira.editmetaError,"onretry":() => jira.retryEditmeta(),}});}
            }
            if(jira.editmeta){
                 for(let ef of __sveltets_2_ensureArray(jira.editmeta.filter((ef) => {
                // Skip fields rendered by dedicated controls above.
                if (ef.key === 'priority' || ef.key === 'labels') return false;
                if (jira.estimateField && ef.key === jira.estimateField.key) return false;
                if (!jira.estimateField && /story\s*point/i.test(ef.name)) return false;
                // Skip if there's already a populated value row for this field.
                const existing = issueFull?.fields.find((f) => f.key === ef.key);
                if (existing && existing.value && existing.value.trim() !== '') return false;
                return true;
              }))){ef.key;
                 { svelteHTML.createElement("span", { "class":`detail-key detail-key-empty`,});ef.name; }
                 { svelteHTML.createElement("span", { "class":`detail-val`,});
                  if(jira.editingField === ef.key){
                    ;__sveltets_2_ensureSnippet(fieldEditor(ef));
                  }else{
                     { svelteHTML.createElement("div", { "class":`detail-val-row`,});
                       { svelteHTML.createElement("span", { "class":`detail-empty`,});  }
                       { svelteHTML.createElement("button", {         "class":`field-edit-btn`,"title":`Set ${ef.name}`,"aria-label":`Set ${ef.name}`,"onclick":() => jira.beginEdit(ef, ''),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} }
                     }
                  }
                 }
              }
            }
           }
        }
       }

      
      if(issueFull.links && issueFull.links.length > 0){
         { svelteHTML.createElement("div", { "class":`jira-card collapsible-card`,});
           { svelteHTML.createElement("button", {       "class":`jira-coll-trigger`,"onclick":() => toggle('links'),"aria-expanded":!collapsed.links,});
             { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":collapsed.links ? 'chevronRight' : 'chevronDown',"size":12,}});} }
             { svelteHTML.createElement("span", { "class":`jira-section-label`,});  }
             { svelteHTML.createElement("span", { "class":`section-count`,}); issueFull.links.length;  }
           }
          if(!collapsed.links){
             { svelteHTML.createElement("div", { "class":`links-list`,});
                 for(let lnk of __sveltets_2_ensureArray(issueFull.links)){lnk.key;
                 { svelteHTML.createElement("div", { "class":`link-row`,});
                   { svelteHTML.createElement("span", { "class":`link-rel`,});lnk.rel; }
                   { svelteHTML.createElement("span", { "class":`link-key mono-sm`,});lnk.key; }
                   { svelteHTML.createElement("span", { "class":`link-type chip-sm`,});lnk.issue_type; }
                   { svelteHTML.createElement("span", { "class":`link-summary`,});lnk.summary; }
                   { svelteHTML.createElement("span", { "class":`link-status status-sm`,});lnk.status; }
                 }
              }
             }
          }
         }
      }

      
       { svelteHTML.createElement("div", { "class":`jira-card collapsible-card`,});
         { svelteHTML.createElement("button", {       "class":`jira-coll-trigger`,"onclick":() => toggle('development'),"aria-expanded":!collapsed.development,});
           { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":collapsed.development ? 'chevronRight' : 'chevronDown',"size":12,}});} }
           { svelteHTML.createElement("span", { "class":`jira-section-label`,});  }
          if(devStatus){
             { svelteHTML.createElement("span", { "class":`section-count`,});
              devStatus.branches.length + devStatus.commits.length + devStatus.pull_requests.length;
             }
          }
         }
        if(!collapsed.development){
           { svelteHTML.createElement("div", { "class":`dev-body`,});
            if(devLoading || devError){
               { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          "what":`development info`,"variant":`compact`,"loading":devLoading,"error":devError,"empty":true,"onretry":() => { issue.devLoaded = false; void issue.loadDev(); },}});}
            } else if (devStatus && (devStatus.branches.length || devStatus.commits.length || devStatus.pull_requests.length)){
              if(devStatus.pull_requests.length){
                 { svelteHTML.createElement("div", { "class":`dev-group`,});
                   { svelteHTML.createElement("span", { "class":`dev-group-label`,});  }
                     for(let pr of __sveltets_2_ensureArray(devStatus.pull_requests)){pr.repo + ':' + pr.id;
                     { svelteHTML.createElement("a", {         "class":`dev-row`,"href":pr.url,"target":`_blank`,"rel":`noopener noreferrer`,});
                       { svelteHTML.createElement("span", { "class":`dev-pr-status status-sm`,});pr.status; }
                       { svelteHTML.createElement("span", { "class":`dev-pr-name`,});pr.name; }
                       { svelteHTML.createElement("span", { "class":`dev-repo chip-sm`,});pr.repo; }
                     }
                  }
                 }
              }
              if(devStatus.branches.length){
                 { svelteHTML.createElement("div", { "class":`dev-group`,});
                   { svelteHTML.createElement("span", { "class":`dev-group-label`,});  }
                     for(let b of __sveltets_2_ensureArray(devStatus.branches)){b.repo + ':' + b.name;
                     { svelteHTML.createElement("a", {         "class":`dev-row`,"href":b.url,"target":`_blank`,"rel":`noopener noreferrer`,});
                       { svelteHTML.createElement("span", { "class":`dev-branch-name mono-sm`,});b.name; }
                       { svelteHTML.createElement("span", { "class":`dev-repo chip-sm`,});b.repo; }
                     }
                  }
                 }
              }
              if(devStatus.commits.length){
                 { svelteHTML.createElement("div", { "class":`dev-group`,});
                   { svelteHTML.createElement("span", { "class":`dev-group-label`,});  }
                     for(let c of __sveltets_2_ensureArray(devStatus.commits)){c.repo + ':' + c.id;
                     { svelteHTML.createElement("a", {         "class":`dev-row`,"href":c.url,"target":`_blank`,"rel":`noopener noreferrer`,});
                       { svelteHTML.createElement("span", { "class":`dev-commit-id mono-sm`,});c.id.slice(0, 8); }
                       { svelteHTML.createElement("span", { "class":`dev-commit-msg`,});c.message; }
                       { svelteHTML.createElement("span", { "class":`dev-repo chip-sm`,});c.repo; }
                     }
                  }
                 }
              }
            }else{
               { svelteHTML.createElement("div", { "class":`comments-empty`,});
                          
                    
               }
            }
           }
        }
       }

    }
   }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const JiraIssueSection__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type JiraIssueSection__SvelteComponent_ = ReturnType<typeof JiraIssueSection__SvelteComponent_>;
/*Ωignore_endΩ*/export default JiraIssueSection__SvelteComponent_;