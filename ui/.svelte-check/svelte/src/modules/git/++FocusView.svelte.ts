///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { providerErrorText } from '../../lib/loadError';
import { rel } from '../../lib/stores/now.svelte';
// Focus view (GitKraken-style): everything "mine" across the whole install in
// one place — MY PULL REQUESTS (open PRs aggregated across every registered
// repo, grouped by forge) and MY WORK (Jira issues assigned to me, rebuilt
// into project → epic → story → subtask hierarchy). Rows deep-link both ways:
// into the Otto repo tab and out to the provider.
import { untrack } from 'svelte';
import { dialogFocus } from '../../lib/dialogFocus';
import { api } from '../../lib/api/client';
import { getIssue, listIssueAccounts, myWork } from '../../lib/api/issues';
import { mapLimit } from '../../lib/poll';
import type { GitAccount, IssueAccount, IssueDetail, MyWorkIssue, PrListResp, PrSummary, Repo } from '../../lib/api/types';
import { ui } from '../../lib/stores/ui.svelte';
import { git } from '../../lib/stores/git.svelte';
import { router } from '../../lib/router.svelte';
import { openExternal } from '../../lib/external';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
function $$render() {
 const issueRow/*Ωignore_positionΩ*/ = (node: WorkNode, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", {   "class":`fx-issue`,"style":`padding-inline-start:${10 + depth * 18}px`,});
     { svelteHTML.createElement("button", {        "class":`fx-issue-main`,"onclick":(e) => { e.currentTarget.focus(); void openQuick(node.issue.key); },"title":`Quick view ${node.issue.key}`,});quickKey === node.issue.key;
       { svelteHTML.createElement("span", { "class":`fx-type t-${node.issue.issue_type.toLowerCase().replace(/[^a-z]+/g, '-')}`,});node.issue.issue_type; }
       { svelteHTML.createElement("span", { "class":`mono fx-key`,});node.issue.key; }
       { svelteHTML.createElement("span", { "class":`fx-summary`,});node.issue.summary; }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(node.issue.priority){ { svelteHTML.createElement("span", { "class":`fx-prio dim`,});node.issue.priority; }}
       { svelteHTML.createElement("span", { "class":`fx-status`,});node.issue.status; }
       { svelteHTML.createElement("span", {   "class":`fx-ago dim`,"title":node.issue.updated_at ? `Updated ${new Date(node.issue.updated_at).toLocaleString()}` : undefined,});ago(node.issue.updated_at); }
     }
     { svelteHTML.createElement("button", {         "class":`fx-ext`,"onclick":() => openExternal(node.issue.url),"title":`Open in ${tracker}`,"aria-label":`Open ${node.issue.key} in ${tracker}`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}
     }
   }
     for(let child of __sveltets_2_ensureArray(node.children)){child.issue.key;
    ;__sveltets_2_ensureSnippet(issueRow(child, depth + 1));
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── MY PULL REQUESTS ────────────────────────────────────────────────────────
  interface PrRow {
    repo: Repo;
    pr: PrSummary;
  }
  let prRows = $state<PrRow[]>([]);
  let prsLoading = $state(true);
  /** Repos whose PR listing failed (no forge account, network) — shown once. */
  let prErrors = $state(0);
  let prFilter = $state<'all' | 'mine'>('all');
  let myHandles = $state<Set<string>>(new Set());

  async function loadPrs(): Promise<void> {
    prsLoading = true;
    prErrors = 0;
    try {
      if (!git.allReposLoaded) await git.loadAllRepos();
      // Forge usernames — powers the "Mine" filter (best-effort).
      void api
        .get<GitAccount[]>('/git/accounts')
        .then((accs) => {
          myHandles = new Set(accs.map((a) => a.username.toLowerCase()).filter(Boolean));
        })
        .catch(() => {});
      const candidates = git.allRepos.filter(
        (r) => r.forge && r.forge !== 'unrecognized',
      );
      // One provider API call per repo (0.5–3 s each) across every workspace:
      // at most 2 at a time, on the background lane (r3-04-05). The old
      // unbounded fan-out filled the alias host's six sockets for ~(R/6) ×
      // provider latency, stalling every poll, git auto-fetch and DB "Run".
      const settled = await mapLimit(candidates, 2, async (repo): Promise<PromiseSettledResult<PrRow[]>> => {
        try {
          const page = await api.bg.get<PrListResp>(`/repos/${repo.id}/prs?state=open`);
          return { status: 'fulfilled', value: page.items.map((pr) => ({ repo, pr })) };
        } catch (reason) {
          return { status: 'rejected', reason };
        }
      });
      const rows: PrRow[] = [];
      for (const s of settled) {
        if (s.status === 'fulfilled') rows.push(...s.value);
        else prErrors++;
      }
      rows.sort((a, b) => (b.pr.updated_at ?? '').localeCompare(a.pr.updated_at ?? ''));
      prRows = rows;
    } finally {
      prsLoading = false;
    }
  }

  const visiblePrRows = $derived(
    prFilter === 'mine' && myHandles.size > 0
      ? prRows.filter((r) => myHandles.has(r.pr.author.toLowerCase()))
      : prRows,
  );

  /** Rows grouped by forge, insertion-ordered github → bitbucket → gitlab. */
  const prGroups = $derived.by(() => {
    const groups = new Map<string, PrRow[]>();
    for (const row of visiblePrRows) {
      const forge = (row.repo.forge as string) ?? 'other';
      const list = groups.get(forge);
      if (list) list.push(row);
      else groups.set(forge, [row]);
    }
    return [...groups.entries()];
  });

  // ── MY WORK (Jira) ──────────────────────────────────────────────────────────
  let issueAccounts = $state<IssueAccount[]>([]);
  let accountsError = $state<string | null>(null);
  let issueAccountId = $state<string>('');
  /** The selected account's tracker name — copy and "Open in" follow it. */
  const tracker = $derived(issueAccounts.find((a) => a.id === issueAccountId)?.provider === 'linear' ? 'Linear' : 'Jira');
  let work = $state<MyWorkIssue[]>([]);
  let workLoading = $state(true);
  let workError = $state<string | null>(null);

  async function loadAccounts(): Promise<void> {
    accountsError = null;
    workLoading = true;
    try {
      issueAccounts = await listIssueAccounts();
      if (!issueAccountId && issueAccounts[0]) issueAccountId = issueAccounts[0].id;
    } catch (e) {
      accountsError = providerErrorText(e) || 'The account service did not answer.';
    } finally {
      if (issueAccounts.length === 0) workLoading = false;
    }
  }

  let workRevision = 0;

  async function loadWork(accountId: string): Promise<void> {
    const revision = ++workRevision;
    workLoading = true;
    workError = null;
    try {
      const next = await myWork(accountId);
      if (revision === workRevision && accountId === issueAccountId) work = next;
    } catch (e) {
      if (revision !== workRevision || accountId !== issueAccountId) return;
      work = [];
      workError = providerErrorText(e);
    } finally {
      if (revision === workRevision && accountId === issueAccountId) workLoading = false;
    }
  }

  $effect(() => {
    void loadPrs();
    void loadAccounts();
  });
  $effect(() => {
    const id = issueAccountId;
    untrack(closeQuick);
    work = [];
    if (id) void loadWork(id);
    return () => { workRevision++; };
  });

  // Hierarchy: project → (parent group | top-level issue) → children.
  interface WorkNode {
    issue: MyWorkIssue;
    children: WorkNode[];
  }
  interface ParentGroup {
    key: string;
    summary: string;
    type: string;
    /** True when the parent itself is one of MY issues (renders as a node). */
    children: WorkNode[];
  }
  interface ProjectGroup {
    key: string;
    name: string;
    groups: ParentGroup[];
    loose: WorkNode[];
  }

  const projects = $derived.by((): ProjectGroup[] => {
    const byKey = new Map<string, WorkNode>();
    for (const w of work) byKey.set(w.key, { issue: w, children: [] });

    const projs = new Map<string, ProjectGroup>();
    const projOf = (w: MyWorkIssue): ProjectGroup => {
      let p = projs.get(w.project_key);
      if (!p) {
        p = { key: w.project_key, name: w.project_name, groups: [], loose: [] };
        projs.set(w.project_key, p);
      }
      return p;
    };

    for (const w of work) {
      const node = byKey.get(w.key)!;
      if (w.parent_key && byKey.has(w.parent_key)) {
        // Parent is also mine → nest directly under it.
        byKey.get(w.parent_key)!.children.push(node);
        continue;
      }
      const proj = projOf(w);
      if (w.parent_key) {
        // Parent exists but isn't mine → synthetic group header (epic/story).
        let g = proj.groups.find((g) => g.key === w.parent_key);
        if (!g) {
          g = {
            key: w.parent_key,
            summary: w.parent_summary ?? w.parent_key,
            type: w.parent_type ?? '',
            children: [],
          };
          proj.groups.push(g);
        }
        g.children.push(node);
      } else {
        proj.loose.push(node);
      }
    }
    return [...projs.values()];
  });

  // ── Issue quick-view (right side panel) ─────────────────────────────────────
  let quickKey = $state<string | null>(null);
  let quickIssue = $state<IssueDetail | null>(null);
  let quickLoading = $state(false);
  let quickError = $state<string | null>(null);

  let quickRevision = 0;
  let narrow = $state(false);
  $effect(() => {
    const media = window.matchMedia('(max-width: 1024px)');
    const sync = () => { narrow = media.matches; };
    sync();
    media.addEventListener('change', sync);
    return () => media.removeEventListener('change', sync);
  });
  $effect(() => {
    if (!narrow || quickKey === null) return;
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });
  function quickFocus(node: HTMLElement) {
    if (narrow) return dialogFocus(node, closeQuick);
  }

  async function openQuick(key: string): Promise<void> {
    const revision = ++quickRevision;
    const accountId = issueAccountId;
    quickError = null;
    quickKey = key;
    quickIssue = null;
    quickLoading = true;
    try {
      const next = await getIssue(accountId, key);
      if (revision === quickRevision && accountId === issueAccountId) quickIssue = next;
    } catch (e) {
      if (revision === quickRevision) {
        quickIssue = null;
        quickError = providerErrorText(e);
      }
    } finally {
      if (revision === quickRevision) quickLoading = false;
    }
  }

  function closeQuick(): void {
    quickRevision++;
    quickLoading = false;
    quickKey = null;
    quickIssue = null;
  }

  // ── Formatting ──────────────────────────────────────────────────────────────
  /** Dense "3m" / "2h" label via the shared clock (`rel`, compact style). */
  function ago(iso: string | null): string {
    return iso ? rel(iso, { compact: true }) : '';
  }

  const CI_LABEL: Record<string, string> = {
    success: 'CI passing',
    passing: 'CI passing',
    failure: 'CI failing',
    failed: 'CI failing',
    failing: 'CI failing',
    pending: 'CI running',
  };

  const forgeLabel: Record<string, string> = {
    github: 'GitHub',
    bitbucket: 'Bitbucket',
    gitlab: 'GitLab',
    other: 'Other',
  };
;
async () => {



 { svelteHTML.createElement("div", {  "class":`focus`,});quickKey !== null;
   { svelteHTML.createElement("div", {   "class":`focus-main`,"inert":narrow && quickKey !== null,});
    
     { svelteHTML.createElement("section", { "class":`fx-section`,});
       { svelteHTML.createElement("header", { "class":`fx-head`,});
        
         { svelteHTML.createElement("span", { "class":`fx-title`,});prFilter === 'mine' && myHandles.size > 0 ? 'My pull requests' : 'Open pull requests'; }
         { svelteHTML.createElement("span", { "class":`fx-count`,});visiblePrRows.length; }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(myHandles.size > 0){
           { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Which pull requests`,});
             { svelteHTML.createElement("button", {    "aria-pressed":prFilter === 'all',"onclick":() => (prFilter = 'all'),});prFilter === 'all';  }
             { svelteHTML.createElement("button", {    "aria-pressed":prFilter === 'mine',"onclick":() => (prFilter = 'mine'),});prFilter === 'mine';   }
           }
        }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void loadPrs(),"title":`Refresh pull requests`,"aria-label":`Refresh pull requests`,});
           { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}
         }
       }

      if(prsLoading){
         { svelteHTML.createElement("div", { "style":`padding: 10px`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {    "rows":4,"height":26,}});} }
      } else if (prGroups.length === 0){
         { svelteHTML.createElement("div", { "class":`fx-empty dim`,});
             prFilter === 'mine' ? ' opened by you' : ''; 
          plural(git.allRepos.length, 'registered repository', 'registered repositories');
          if(prErrors > 0){  plural(prErrors, 'repo'); }
         }
      }else{
        if(prErrors > 0){
           { svelteHTML.createElement("div", { "class":`fx-warn dim`,});     plural(prErrors, 'repo');         }
        }
           for(let [forge, rows] of __sveltets_2_ensureArray(prGroups)){forge;
           { svelteHTML.createElement("div", { "class":`fx-group-head`,});
             { const $$_nocIredivorP4C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP4C({ target: __sveltets_2_any(), props: {    "provider":forge,"size":13,}});}
             { svelteHTML.createElement("span", {});forgeLabel[forge] ?? forge; }
             { svelteHTML.createElement("span", { "class":`fx-count`,});rows.length; }
           }
             for(let row of __sveltets_2_ensureArray(rows)){row.repo.id + '#' + row.pr.number;
             { svelteHTML.createElement("div", { "class":`fx-pr`,});
               { svelteHTML.createElement("span", {   "class":`fx-ago dim`,"title":`Updated ${new Date(row.pr.updated_at).toLocaleString()}`,});ago(row.pr.updated_at); }
               { svelteHTML.createElement("button", {       "class":`fx-pr-title`,"onclick":() => openExternal(row.pr.url),"title":`Open #${row.pr.number} on the provider`,});
                 { svelteHTML.createElement("span", { "class":`mono fx-pr-num`,}); row.pr.number; }
                if(row.pr.draft){ { svelteHTML.createElement("span", { "class":`fx-draft`,});  }}
                 { svelteHTML.createElement("span", { "class":`fx-pr-text`,});row.pr.title; }
               }
               { svelteHTML.createElement("span", {   "class":`dim fx-pr-author`,"title":`Author`,});row.pr.author; }
               { svelteHTML.createElement("button", {       "class":`fx-repo-link`,"onclick":() => {
                  git.openRepoTab(row.repo.id, 'prs');
                  router.go(`git/${row.repo.id}/prs`);
                },"title":`Open ${row.repo.name} in Otto`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});}
                 { svelteHTML.createElement("span", { "class":`fx-repo-name`,});row.repo.name; }
               }
               { svelteHTML.createElement("span", {   "class":`fx-branch mono`,"title":`${row.pr.source_branch} → ${row.pr.target_branch}`,});
                row.pr.source_branch;
               }
              if(row.pr.ci_status){
                 { svelteHTML.createElement("span", {   "class":`fx-ci ci-${row.pr.ci_status}`,"title":`CI ${row.pr.ci_status}`,});CI_LABEL[row.pr.ci_status] ?? row.pr.ci_status; }
              }
             }
          }
        }
      }
     }

    
     { svelteHTML.createElement("section", { "class":`fx-section`,});
       { svelteHTML.createElement("header", { "class":`fx-head`,});
         { svelteHTML.createElement("span", { "class":`fx-title`,}); tracker;  }
        if(!workLoading && work.length > 0){ { svelteHTML.createElement("span", { "class":`fx-count`,});work.length; }}
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(issueAccounts.length > 1){
           { svelteHTML.createElement("select", {     "class":`input fx-account`,"bind:value":issueAccountId,"title":`Issue-tracker account`,});/*Ωignore_startΩ*/() => issueAccountId = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let a of __sveltets_2_ensureArray(issueAccounts)){a.id;
               { svelteHTML.createElement("option", { "value":a.id,});a.label; }
            }
           }
        }
        if(issueAccountId){
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void loadWork(issueAccountId),"title":`Refresh my ${tracker} work`,"aria-label":`Refresh my ${tracker} work`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}
           }
        }
       }

      if(accountsError){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`issue-tracker accounts`,"error":accountsError,"empty":true,"onretry":() => void loadAccounts(),}});}
      } else if (issueAccounts.length === 0 && !workLoading){
         { svelteHTML.createElement("div", { "class":`fx-empty dim fx-empty-row`,});
           { svelteHTML.createElement("span", {});              }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('settings/jira'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});}   }
         }
      } else if (workLoading){
         { svelteHTML.createElement("div", { "style":`padding: 10px`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {    "rows":5,"height":24,}});} }
      } else if (workError){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`your ${tracker} work`,"error":workError,"empty":true,"onretry":() => void loadWork(issueAccountId),}});}
      } else if (work.length === 0){
         { svelteHTML.createElement("div", { "class":`fx-empty dim`,});       }
      }else{
           for(let proj of __sveltets_2_ensureArray(projects)){proj.key;
           { svelteHTML.createElement("div", { "class":`fx-group-head`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
             { svelteHTML.createElement("span", {});proj.name || proj.key; }
             { svelteHTML.createElement("span", { "class":`mono dim`,});proj.key; }
           }
             for(let g of __sveltets_2_ensureArray(proj.groups)){g.key;
             { svelteHTML.createElement("div", { "class":`fx-parent`,});
               { svelteHTML.createElement("span", { "class":`fx-type t-epic`,});g.type || 'Parent'; }
               { svelteHTML.createElement("span", { "class":`mono fx-key dim`,});g.key; }
               { svelteHTML.createElement("span", { "class":`fx-parent-summary`,});g.summary; }
             }
               for(let node of __sveltets_2_ensureArray(g.children)){node.issue.key;
              ;__sveltets_2_ensureSnippet(issueRow(node, 1));
            }
          }
             for(let node of __sveltets_2_ensureArray(proj.loose)){node.issue.key;
            ;__sveltets_2_ensureSnippet(issueRow(node, 0));
          }
        }
      }
     }
   }

  
  if(quickKey !== null){
    narrow; {
     {const $$action_0 = __sveltets_2_ensureAction(quickFocus(svelteHTML.mapElementTag('aside')));{ svelteHTML.createElement("aside", __sveltets_2_union($$action_0), {        "class":`fx-quick`,"role":narrow ? 'dialog' : 'complementary',"aria-modal":narrow ? 'true' : undefined,"aria-label":`Issue ${quickKey}`,});
       { svelteHTML.createElement("header", { "class":`fx-quick-head`,});
         { svelteHTML.createElement("span", { "class":`mono fx-key`,});quickKey; }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {         "class":`btn small ghost`,"onclick":() => quickIssue && openExternal(quickIssue.url),"disabled":!quickIssue,"title":`Open in ${tracker}`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}   tracker;
         }
         { svelteHTML.createElement("button", {       "class":`fx-quick-close`,"onclick":closeQuick,"title":`Close issue quick view`,"aria-label":`Close issue quick view`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} }
       }
      if(quickLoading){
         { svelteHTML.createElement("div", { "style":`padding: 12px`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"height":20,}});} }
      } else if (quickIssue){
         { svelteHTML.createElement("div", { "class":`fx-quick-body`,});
           { svelteHTML.createElement("div", { "class":`fx-quick-summary`,});quickIssue.summary; }
           { svelteHTML.createElement("div", { "class":`fx-quick-meta`,});
             { svelteHTML.createElement("span", { "class":`fx-type`,});quickIssue.issue_type; }
             { svelteHTML.createElement("span", { "class":`fx-status`,});quickIssue.status; }
            if(quickIssue.assignee){ { svelteHTML.createElement("span", { "class":`dim`,}); quickIssue.assignee; }}
           }
          if(quickIssue.description){
             { svelteHTML.createElement("pre", { "class":`fx-quick-desc`,});quickIssue.description; }
          }else{
             { svelteHTML.createElement("div", {   "class":`dim`,"style":`font-size: var(--fs-s)`,});  }
          }
         }
      }else{
         { svelteHTML.createElement("div", {   "class":`fx-empty fx-empty-row`,"role":`alert`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
           { svelteHTML.createElement("span", { "class":`grow`,});  quickKey; if(quickError){ { svelteHTML.createElement("span", { "class":`dim`,});quickError; }} }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => quickKey && void openQuick(quickKey),});  }
         }
      }
     }}
    }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FocusView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FocusView__SvelteComponent_ = ReturnType<typeof FocusView__SvelteComponent_>;
/*Ωignore_endΩ*/export default FocusView__SvelteComponent_;