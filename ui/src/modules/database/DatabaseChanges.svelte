<script lang="ts">
  import { badgeTone, runStatus } from '../../lib/status';
  import Badge from '../../lib/components/Badge.svelte';
  import { onMount, untrack } from 'svelte';
  import { sentenceCase, runStateLabel } from '../../lib/labels';
  import { pollWhileVisible } from '../../lib/poll';
  import { databaseChangesApi as changesApi, type DatabaseChange, type ChangeDetail, type ChangeInput, type ChangeTarget } from '../../lib/api/database-changes';
  import { database } from '../../lib/stores/database.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { plural } from '../../lib/plural';
  import { loadErrorText } from '../../lib/loadError';
  let {connectionId,node=null}:{connectionId:string;node?:string|null}=$props();
  let changes:DatabaseChange[]=$state([]);
  let detail:ChangeDetail|null=$state(null);
  let title=$state(''), description=$state(''), script=$state(''), targetDb=$state('');
  let draftTargets:ChangeTarget[]=$state([]);
  let executors:{id:string;display_name:string;username:string}[]=$state([]);
  let executor=$state(''), note=$state(''), error=$state(''), busy=$state(false), editing=$state(false);
  // `loadError` is the LIST load (inline, with Retry); `error` is the last failed ACTION.
  let loading=$state(true), loadError=$state('');
  // Which action is in flight, so only that button reads "Saving…" / "Working…".
  let pending=$state('');
  let generation=0;
  const selected:DatabaseChange|null=$derived.by(()=>detail?.change ?? null);
  // The unselected main pane summarises the collection instead of a bare "pick one":
  // "5 changes · 2 awaiting review · 1 approved".
  const changesSummary=$derived.by(()=>{
    const count=(status:string)=>changes.filter(c=>c.status===status).length;
    const parts=[plural(changes.length,'change')];
    const review=count('awaiting_review'), approved=count('approved'), unknown=count('outcome_unknown');
    if(review)parts.push(`${review} awaiting review`);
    if(approved)parts.push(`${approved} approved`);
    if(unknown)parts.push(`${unknown} outcome unknown`);
    return parts.join(' · ');
  });
  const child=$derived((node ?? '').replace(/^db:/,'').split('/')[0] || undefined);
  const owner=$derived(selected?.author_id===auth.me?.id);
  const independent=$derived(selected !== null && ![selected.author_id,selected.real_author_id].includes(auth.me?.id ?? '') && ![selected.author_id,selected.real_author_id].includes(auth.realUser?.id ?? ''));
  function can(operation:string) {
    const targets=editing ? draftTargets : selected?.targets ?? [{connection_id:connectionId,node:targetDb || child || ''}];
    return targets.every(t=>resourceAccess.can('connection',t.connection_id,operation,'database',operation==='change_submit'?'edit':'admin',t.node.replace(/^db:/,'') || undefined));
  }
  async function refresh() {
    const mine=++generation;
    // Only the first load shows the skeleton; polls refresh quietly (a failed one raises the stale bar).
    loading=changes.length===0;
    try {
      const rows=await changesApi.list(connectionId);
      if(mine!==generation)return;
      changes=rows;
      loadError='';
      if(selected){
        const next=await changesApi.get(selected.id);
        if(mine===generation){detail=next; await loadCapabilities(next.change);}
      }
    } catch(e) {
      if(mine===generation)loadError=loadErrorText(e);
    } finally {
      if(mine===generation)loading=false;
    }
  }
  async function loadCapabilities(change:DatabaseChange) {
    await Promise.all(change.targets.map(t=>resourceAccess.load('connection',t.connection_id,t.node.replace(/^db:/,''))));
  }
  async function select(change:DatabaseChange) {
    editing=false;error='';const mine=++generation;
    try {
      const next=await changesApi.get(change.id);
      if(mine!==generation)return;
      detail=next;
      executor=next.change.executor_id ?? auth.me?.id ?? '';
      await loadCapabilities(next.change);
      if(next.change.author_id===auth.me?.id){executors=await changesApi.executors(next.change.id);}
    } catch(e) {
      if(mine===generation)error=String(e);
    }
  }
  function draft() {
    detail=null;title='';description='';script='';
    targetDb=child ?? '';executor=auth.me?.id ?? '';
    editing=true;draftTargets=[{connection_id:connectionId,node:child ?? ''}];
    note='';error='';
  }
  function revise() {
    if(!selected)return;
    title=selected.title;description=selected.description;script=selected.script;
    targetDb=selected.targets[0]?.node.replace(/^db:/,'') ?? '';
    draftTargets=structuredClone(selected.targets);
    editing=true;
  }
  async function perform(key:string,work:()=>Promise<DatabaseChange>) {
    busy=true;pending=key;error='';
    try {
      const change=await work();
      editing=false;
      await select(change);
      await refresh();
    } catch(e) {
      error=String(e);
    } finally {
      busy=false;pending='';
    }
  }
  /** Button label: the idle verb, or its in-flight form while that action runs. */
  const label=(key:string,idle:string,active='Working…')=>pending===key?active:idle;
  function save() {
    const input:ChangeInput={title,description,script,targets:draftTargets};
    void perform('save',()=>selected ? changesApi.revise(selected.id,selected.revision,input):changesApi.create(input));
  }
  function action(which:'submit'|'approve'|'reject'|'execute'|'cancel') {if(selected)void perform(which,()=>changesApi.action(selected!.id,selected!.revision,which,note));}
  $effect(()=>{
    const id=connectionId;
    const currentChild=child;
    untrack(()=>{
      detail=null;editing=false;targetDb=currentChild ?? '';
      void resourceAccess.load('connection',id,currentChild);
      void refresh();
    });
  });
  $effect(()=>{
    for(const target of draftTargets){
      const db=target.node.replace(/^db:/,'');
      if(db)void resourceAccess.load('connection',target.connection_id,db);
    }
  });
  // Hidden-paused, jittered, backed-off polling (lib/poll.ts) instead of a raw 5 s interval.
  onMount(()=>{
    const poller=pollWhileVisible(async()=>{if(!busy&&!editing)await refresh();},{ms:5000,immediate:false,maxBackoff:8});
    return()=>{generation++;poller.stop();};
  });
</script>
<div class="changes" data-testid="database-changes">
  <div class="toolbar">
    <div>
      <h2>Database changes</h2>
      <p>Review scripts independently, then execute the approved revision.</p>
    </div>
    <button class="btn primary" onclick={draft} disabled={busy || !resourceAccess.can('connection',connectionId,'change_submit','database','edit',child)}>New change</button>
    <button class="btn" onclick={()=>void refresh()} disabled={busy}>Refresh</button>
  </div>
  {#if error}<div class="error" role="alert">{error}</div>{/if}
  <div class="layout">
    <nav aria-label="Database changes">
      <LoadState what="database changes" {loading} error={loadError} empty={changes.length===0} onretry={()=>void refresh()} variant="compact">
        {#snippet emptyView()}<p class="hint">No visible changes yet.</p>{/snippet}
        {#each changes as change (change.id)}
          <button class="item" class:active={selected?.id===change.id} onclick={()=>void select(change)} disabled={busy}>
            <strong>{change.title}</strong>
            <span>{sentenceCase(change.status)} · r{change.revision}</span>
          </button>
        {/each}
      </LoadState>
    </nav>
    <main>
      {#if editing}
        <h3>{selected ? 'Revise change' : 'New change'}</h3>
        <label>Title<input dir="auto" bind:value={title} maxlength="200" /></label>
        <label>Description<textarea dir="auto" bind:value={description} rows="2" maxlength="16384"></textarea></label>
        <fieldset>
          <legend>Database targets</legend>
          {#each draftTargets as target,index}
            <div class="target-row">
              <label>Connection {index+1}
                <select bind:value={target.connection_id}>
                  {#each database.connections.filter(c=>['mysql','postgres'].includes(c.kind)) as conn}<option value={conn.id}>{conn.name}</option>{/each}
                </select>
              </label>
              <label>Database {index+1}<input dir="auto" bind:value={target.node} placeholder="Exact database name" /></label>
              {#if draftTargets.length>1}
                <button class="btn" aria-label={`Remove target ${index+1}`} onclick={()=>draftTargets=draftTargets.filter((_,i)=>i!==index)}>Remove</button>
              {/if}
            </div>
          {/each}
          <button class="btn" disabled={draftTargets.length>=20} onclick={()=>draftTargets=[...draftTargets,{connection_id:connectionId,node:''}]}>Add target</button>
        </fieldset>
        <label>SQL script<textarea dir="ltr" class="script" bind:value={script} rows="14" spellcheck="false" maxlength="262144" placeholder="ALTER TABLE …"></textarea></label>
        <p class="hint">Saving a revision invalidates earlier validation and approval. Scripts run in target order; a rollout is not one transaction.</p>
        <div class="actions">
          <button class="btn primary" onclick={save} disabled={busy || !title.trim() || !script.trim() || draftTargets.some(t=>!t.node.trim()) || !can('change_submit')}>{label('save','Save draft','Saving…')}</button>
          <button class="btn" onclick={()=>editing=false} disabled={busy}>Close editor</button>
        </div>
      {:else if selected && detail}
        <div class="title"><h3>{selected.title}</h3><Badge tone={badgeTone(runStatus(selected.status).tone)} label={sentenceCase(selected.status)} /></div>
        <p>{selected.description}</p>
        <p class="hint">Revision {selected.revision} · Author {selected.author_id}</p>
        <div class="targets">
          {#each selected.targets as target}<span>{target.node.replace(/^db:/,'')} <small>({target.connection_id})</small></span>{/each}
        </div>
        <pre class="approved-script">{selected.script}</pre>
        {#if selected.content_hash}
          <details>
            <summary>Approval binding</summary>
            <p>Executor: {selected.executor_id}</p>
            <code>{selected.content_hash}</code>
            <p>Approval covers this revision, target set, current resource policies and executor credentials. Changes require fresh review.</p>
          </details>
        {/if}
        {#if ['draft','validated','rejected'].includes(selected.status) && owner && can('change_submit')}
          <label>Executor
            <select aria-label="Executor" bind:value={executor}>
              <option value="">Choose an eligible executor</option>
              {#each executors as user}<option value={user.id}>{user.display_name || user.username}</option>{/each}
            </select>
          </label>
          <div class="actions">
            <button class="btn" disabled={busy||!executor.trim()} onclick={()=>void perform('validate',()=>changesApi.validate(selected!.id,selected!.revision,executor.trim()))}>{label('validate','Validate for executor')}</button>
            {#if selected.status==='validated'}
              <button class="btn primary" disabled={busy} onclick={()=>action('submit')}>{label('submit','Submit for review')}</button>
            {/if}
          </div>
        {/if}
        {#if selected.status==='awaiting_review' && can('change_approve')}
          <label>Review note<textarea dir="auto" bind:value={note} rows="2" placeholder="Decision and reason"></textarea></label>
          <div class="actions">
            <button class="btn primary" disabled={busy||!independent} onclick={()=>action('approve')}>{label('approve','Approve revision')}</button>
            <button class="btn danger" disabled={busy||!note.trim()} onclick={()=>action('reject')}>{label('reject','Request revision')}</button>
          </div>
          {#if !independent}<p class="hint">An independent reviewer must approve this change.</p>{/if}
        {/if}
        {#if selected.status==='approved' && can('change_execute')}
          <p class="hint">Execution can change data or schema. Only the selected executor can run this approved script.</p>
          <button class="btn primary" disabled={busy || selected.executor_id!==auth.me?.id} onclick={()=>action('execute')}>{label('execute','Execute approved revision')}</button>
        {/if}
        <div class="actions">
          {#if owner && ['draft','validated','awaiting_review','approved','rejected'].includes(selected.status) && can('change_submit')}
            <button class="btn" disabled={busy} onclick={revise}>Edit revision</button>
            <button class="btn danger" disabled={busy} onclick={()=>action('cancel')}>{label('cancel','Cancel change')}</button>
          {/if}
          {#if selected.status==='running' && can('change_execute')}
            <button class="btn danger" disabled={busy} onclick={()=>action('cancel')}>{label('cancel','Request cancellation')}</button>
          {/if}
        </div>
        {#if detail.attempts.length}
          <h4>Target attempts</h4>
          {#each detail.attempts as attempt}
            <article>
              <strong>{attempt.node?.replace(/^db:/,'')}</strong><Badge tone={badgeTone(runStatus(attempt.state).tone)} label={runStateLabel(attempt.state)} />
              <p>{attempt.summary}</p>
              {#if attempt.state==='outcome_unknown' && selected.status==='outcome_unknown' && can('change_execute')}
                <p class="hint">Inspect the target database before recording an outcome. This does not replay SQL.</p>
                <label>Reconciliation evidence<textarea dir="auto" bind:value={note} rows="2" placeholder="Describe the schema/data checks you performed"></textarea></label>
                <div class="actions">
                  <button class="btn" disabled={busy||note.trim().length<10} onclick={()=>void perform('rec-succeeded',()=>changesApi.reconcile(selected!.id,selected!.revision,attempt.id,'succeeded',note))}>{label('rec-succeeded','Record applied')}</button>
                  <button class="btn" disabled={busy||note.trim().length<10} onclick={()=>void perform('rec-failed',()=>changesApi.reconcile(selected!.id,selected!.revision,attempt.id,'failed',note))}>{label('rec-failed','Record not applied')}</button>
                  <button class="btn" disabled={busy||note.trim().length<10} onclick={()=>void perform('rec-partial',()=>changesApi.reconcile(selected!.id,selected!.revision,attempt.id,'partially_applied',note))}>{label('rec-partial','Record partially applied')}</button>
                </div>
              {/if}
            </article>
          {/each}
        {/if}
        <details>
          <summary>History ({detail.history.length})</summary>
          {#each detail.history as event}
            <p>{new Date(event.created_at).toLocaleString()} · {sentenceCase(event.action)} · {event.actor_id}{event.real_actor_id!==event.actor_id ? ` (via ${event.real_actor_id})` : ''}</p>
          {/each}
        </details>
      {:else if !loading && !loadError}
        <EmptyState
          icon="db"
          title={changes.length ? changesSummary : 'No database changes yet'}
          body={changes.length ? 'Select one to review its script, approvals and execution history.' : 'A change carries a reviewed SQL script from draft to approved run.'}
        />
      {/if}
    </main>
  </div>
</div>
<style>
  .changes {
    padding: 16px;
    min-width: 0;
    color: var(--text);
  }
  h2,
  h3,
  h4,
  p {
    margin: 0 0 10px;
  }
  .toolbar,
  .title,
  .actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .toolbar > div {
    flex: 1;
  }
  .toolbar p,
  .hint,
  small {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .layout {
    display: grid;
    grid-template-columns: 230px minmax(0, 1fr);
    gap: 16px;
    margin-top: 16px;
  }
  nav {
    border-inline-end: 1px solid var(--border);
    padding-inline-end: 12px;
    max-height: 65vh;
    overflow: auto;
  }
  nav .item {
    display: block;
    width: 100%;
    text-align: start;
    margin-bottom: 6px;
    overflow-wrap: anywhere;
    padding: 6px 10px;
    cursor: pointer;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-m);
  }
  nav .item:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  nav span {
    display: block;
    margin-top: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  nav .item.active {
    border-color: var(--accent);
    background: var(--hover);
  }
  main {
    min-width: 0;
    max-height: 68vh;
    overflow: auto;
    padding-inline-end: 4px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 10px 0;
    font-size: var(--fs-s);
  }
  input,
  textarea,
  select {
    width: 100%;
    box-sizing: border-box;
    padding: 8px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  textarea {
    resize: vertical;
  }
  .script,
  .approved-script {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .approved-script {
    padding: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    max-height: 300px;
    overflow: auto;
  }
  .actions {
    margin: 12px 0;
  }
  .targets {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .target-row {
    display: flex;
    gap: 8px;
    align-items: end;
    flex-wrap: wrap;
  }
  .target-row label {
    flex: 1;
    min-width: 130px;
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px;
  }
  legend {
    font-size: var(--fs-s);
  }
  .error {
    padding: 10px;
    border: 1px solid var(--danger);
    border-radius: var(--radius-s);
    white-space: pre-wrap;
    margin: 10px 0;
    overflow-wrap: anywhere;
  }
  article {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px;
    margin: 8px 0;
  }
  article :global(.badge) {
    margin-inline-start: 8px;
  }
  article p {
    margin-top: 8px;
  }
  details {
    margin: 12px 0;
    font-size: var(--fs-s);
  }
  details code {
    display: block;
    overflow-wrap: anywhere;
  }
  @media (max-width: 640px) {
    .layout {
      grid-template-columns: 1fr;
    }
    nav {
      max-height: 150px;
      border-inline-end: 0;
      border-block-end: 1px solid var(--border);
      padding: 0 0 8px;
    }
    main {
      max-height: none;
    }
    .changes {
      padding: 10px;
    }
  }
</style>
