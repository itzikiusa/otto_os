<script lang="ts">
  import { confirmer } from '../confirm.svelte';
  import { toasts } from '../toast.svelte';
  import { untrack } from 'svelte';
  import { accessApi } from '../api/access';
  import { api } from '../api/client';
  import { ws } from '../stores/workspace.svelte';
  import { auth } from '../stores/auth.svelte';
  import { resourceAccess } from '../stores/resource-access.svelte';
  import { accessOperations, operationLabel, scopeHint } from '../access-options';
  import type {
    ResourceKind,
    AccessPolicy,
    AccessSubjects,
    AccessRule,
    AccessPreview,
    EffectiveAccess,
    Connection,
  } from '../api/types';
  let { kind, resourceId }: { kind: ResourceKind; resourceId: string } = $props();
  let policy = $state<AccessPolicy | null>(null);
  let original = $state('');
  let subjects = $state<AccessSubjects>({ users: [], groups: [], roles: [] });
  let credentials = $state<Connection[]>([]);
  let preview = $state<AccessPreview | null>(null);
  let previewSnapshot = $state('');
  let error = $state('');
  let notice = $state('');
  let busy = $state(false);
  let loading = $state(true);
  let tab = $state<'rules' | 'effective'>('rules');
  let userId = $state('');
  let child = $state('');
  let effective = $state<EffectiveAccess | null>(null);
  let generation = 0;
  let effectiveGeneration = 0;
  const dirty = $derived(policy !== null && JSON.stringify(policy) !== original);
  const previewCurrent = $derived(preview !== null && JSON.stringify(policy) === previewSnapshot);
  async function load(k: ResourceKind, id: string) {
    const g = ++generation;
    loading = true;
    policy = null;
    error = '';
    preview = null;
    effective = null;
    subjects = { users: [], groups: [], roles: [] };
    credentials = [];
    try {
      const [p, s] = await Promise.all([accessApi.policy(k, id), accessApi.subjects(k, id)]);
      if (g !== generation) return;
      policy = p;
      original = JSON.stringify(p);
      subjects = s;
      userId = s.users[0]?.id ?? '';
      if (k === 'connection' && auth.isRoot) {
        try {
          const profiles = await api.get<Connection[]>(`/workspaces/${ws.currentId}/connections`);
          if (g === generation && auth.isRoot) credentials = profiles;
        } catch {
          credentials = [];
        }
      }
    } catch (e) {
      if (g === generation) error = String(e);
    } finally {
      if (g === generation) loading = false;
    }
  }
  $effect(() => {
    const k = kind,
      id = resourceId;
    untrack(() => {
      void load(k, id);
    });
    return () => {
      generation++;
      effectiveGeneration++;
    };
  });
  async function reload() {
    if (
      dirty &&
      !(await confirmer.ask('Discard unsaved access edits and reload the saved policy?', {
        title: 'Reload access',
        confirmLabel: 'Reload saved access',
      }))
    )
      return;
    await load(kind, resourceId);
  }
  $effect(() => {
    const k = kind, id = resourceId;
    return resourceAccess.subscribe((change) => {
      if (change.type === 'reset' && change.identity) void load(k, id);
    });
  });

  function addRule() {
    if (!policy) return;
    policy.rules.push({
      id: crypto.randomUUID(),
      subject_kind: 'group',
      subject_id: subjects.groups[0]?.id ?? '',
      effect: 'allow',
      operations: [],
      children: null,
      grantable_operations: [],
      credential_connection_id: null,
    });
  }
  function toggle(
    rule: AccessRule,
    field: 'operations' | 'grantable_operations',
    op: string,
    checked: boolean,
  ) {
    rule[field] = checked ? [...rule[field], op] : rule[field].filter((x) => x !== op);
  }
  function applyRole(rule: AccessRule, roleId: string) {
    const role = subjects.roles.find((r) => r.id === roleId);
    if (!role) return;
    rule.operations = [...role.operations];
    rule.grantable_operations = rule.effect === 'allow' ? [...role.grantable_operations] : [];
  }
  async function review() {
    if (!policy) return;
    busy = true;
    error = '';
    notice = '';
    preview = null;
    const snapshot = JSON.stringify(policy);
    try {
      const result = await accessApi.preview(JSON.parse(snapshot));
      if (snapshot === JSON.stringify(policy)) {
        preview = result;
        previewSnapshot = snapshot;
      }
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function save() {
    if (!policy || !previewCurrent || preview?.issues.length) return;
    busy = true;
    error = '';
    try {
      const saved = await accessApi.save(JSON.parse(JSON.stringify(policy)), preview?.token);
      policy = saved;
      original = JSON.stringify(saved);
      preview = null;
      notice = 'Access saved.';
      toasts.success('Access saved');
      resourceAccess.invalidate();
    } catch (e) {
      error = String(e);
      preview = null;
    } finally {
      busy = false;
    }
  }
  async function inspect() {
    const g = ++effectiveGeneration;
    effective = null;
    error = '';
    if (!userId) return;
    try {
      const result = await accessApi.effective(kind, resourceId, userId, child.trim() || undefined);
      if (g === effectiveGeneration) effective = result;
    } catch (e) {
      if (g === effectiveGeneration) error = String(e);
    }
  }
  function changedOperations(
    before: EffectiveAccess['operations'],
    after: EffectiveAccess['operations'],
  ) {
    return Object.keys({ ...before, ...after }).filter(
      (op) => before[op]?.allowed !== after[op]?.allowed,
    );
  }
</script>

<section class="access-editor" aria-label="Resource access">
  <header>
    <div>
      <h3>Access</h3>
      <p>Page access, resource visibility, then allowed operations. Deny rules always win.</p>
    </div>
    <button class="btn small" disabled={busy || loading} onclick={reload}
      >Reload saved access</button
    >
  </header>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if loading}<p>Loading access…</p>
  {:else if policy}
    <nav aria-label="Access views">
      <button class="btn" class:primary={tab === 'rules'} onclick={() => (tab = 'rules')}
        >Rules</button
      ><button
        class="btn"
        class:primary={tab === 'effective'}
        onclick={() => {
          tab = 'effective';
          void inspect();
        }}>Effective access</button
      >
    </nav>
    {#if tab === 'rules'}
      <label class="mode"
        >Access mode<select
          aria-label="Access mode"
          bind:value={policy.mode}
          disabled={!auth.isRoot || busy}
          ><option value="legacy">Legacy feature permissions</option><option value="enforced"
            >Enforced resource permissions</option
          ></select
        ></label
      >
      <p class="hint">
        {policy.mode === 'legacy'
          ? 'Legacy mode uses existing feature permissions. Configure rules and review the comparison before activation.'
          : 'Only matching grants allow access. Root is the owner exception. Feature, workspace, token and native permissions still apply.'}
      </p>
      <fieldset disabled={busy}>
        {#each policy.rules as rule, i (rule.id)}
          <article class="rule" aria-label={`Rule ${i + 1}`}>
            <div class="rule-head">
              <strong>Rule {i + 1}</strong><button
                class="btn small"
                onclick={() => {
                  if (policy) policy.rules = policy.rules.filter((r) => r.id !== rule.id);
                }}>Remove rule</button
              >
            </div>
            <div class="fields">
              <label
                >Subject type<select
                  aria-label="Subject type"
                  bind:value={rule.subject_kind}
                  onchange={() => (rule.subject_id = '')}
                  ><option value="group">Group</option><option value="user">User</option></select
                ></label
              >
              <label
                >Subject<select aria-label="Subject" bind:value={rule.subject_id}
                  ><option value="">Choose {rule.subject_kind}</option
                  >{#if rule.subject_kind === 'group'}{#each subjects.groups as g}<option
                        value={g.id}>{g.name}</option
                      >{/each}{:else}{#each subjects.users as u}<option value={u.id}
                        >{u.display_name} (@{u.username})</option
                      >{/each}{/if}</select
                ></label
              >
              <label
                >Effect<select
                  aria-label="Effect"
                  bind:value={rule.effect}
                  onchange={() => {
                    if (rule.effect === 'deny') {
                      rule.grantable_operations = [];
                      rule.credential_connection_id = null;
                    }
                  }}><option value="allow">Allow</option><option value="deny">Deny</option></select
                ></label
              >
            </div>
            <label
              >Scope<select
                aria-label="Scope"
                value={rule.children === null ? 'all' : 'selected'}
                onchange={(e) => (rule.children = e.currentTarget.value === 'all' ? null : [])}
                ><option value="all">All children, including future children</option><option
                  value="selected">Only named children</option
                ></select
              ></label
            >
            {#if rule.children !== null}<label
                >Named children<textarea
                  rows="3"
                  value={rule.children.join('\n')}
                  onchange={(e) =>
                    (rule.children = [
                      ...new Set(
                        e.currentTarget.value
                          .split('\n')
                          .map((x) => x.trim())
                          .filter(Boolean),
                      ),
                    ])}
                  placeholder={scopeHint[kind]}></textarea></label
              >
              <p class="hint">{scopeHint[kind]} New children are excluded.</p>{/if}
            <label
              >Copy role preset<select
                aria-label="Copy role preset"
                value=""
                onchange={(e) => {
                  applyRole(rule, e.currentTarget.value);
                  e.currentTarget.value = '';
                }}
                ><option value="">Choose preset…</option
                >{#each subjects.roles.filter((r) => r.kind === kind) as role}<option
                    value={role.id}>{role.name}</option
                  >{/each}</select
              ></label
            >
            <p class="hint">
              Unchecked operations inherit other rules. Use a Deny rule to restrict access. Presets
              copy operations; later preset edits do not change this rule.
            </p>
            <div class="operations">
              {#each accessOperations[kind] as op}<label
                  ><input
                    type="checkbox"
                    checked={rule.operations.includes(op)}
                    onchange={(e) => toggle(rule, 'operations', op, e.currentTarget.checked)}
                  />{operationLabel(op)}</label
                >{/each}
            </div>
            {#if rule.effect === 'allow'}
              <details>
                <summary>Delegation ceiling</summary>
                <p class="hint">
                  Operations this subject may grant to others within this rule’s scope. This does
                  not grant the operation itself.
                </p>
                <div class="operations">
                  {#each accessOperations[kind] as op}<label
                      ><input
                        type="checkbox"
                        checked={rule.grantable_operations.includes(op)}
                        onchange={(e) =>
                          toggle(rule, 'grantable_operations', op, e.currentTarget.checked)}
                      />{operationLabel(op)}</label
                    >{/each}
                </div>
              </details>
              {#if kind === 'connection'}<label
                  >Native credential profile<select
                    aria-label="Native credential profile"
                    bind:value={rule.credential_connection_id}
                    disabled={!auth.isRoot}
                    ><option value={null}>Connection’s saved credentials</option
                    >{#each credentials.filter((c) => c.id !== resourceId) as c}<option value={c.id}
                        >{c.name}</option
                      >{/each}{#if rule.credential_connection_id && !credentials.some((c) => c.id === rule.credential_connection_id)}<option
                        value={rule.credential_connection_id}>Assigned profile</option
                      >{/if}</select
                  ></label
                >
                <p class="hint">
                  Use a saved connection with restricted database credentials for this scope. Native
                  privileges are verified before execution.
                </p>{/if}
            {/if}
          </article>
        {/each}
        {#if policy.rules.length === 0}<p>
            No rules. Enforced mode allows only root until grants are added.
          </p>{/if}
        <button class="btn" onclick={addRule}>Add rule</button>
      </fieldset>
      <div class="actions">
        <button class="btn" disabled={busy || !dirty} onclick={review}>Review changes</button
        ><button
          class="btn primary"
          disabled={busy || !dirty || !previewCurrent || !!preview?.issues.length}
          onclick={save}
          >{policy.mode === 'enforced' && JSON.parse(original).mode === 'legacy'
            ? 'Activate enforced access'
            : 'Save access'}</button
        >
      </div>
      {#if previewCurrent && preview}<section class="preview" aria-label="Access comparison">
          <h4>Review access changes</h4>
          {#each preview.issues as issue}<p class="error">{issue}</p>{/each}
          <p class="hint">
            Comparison includes existing users and every named child scope in the policy.
          </p>
          {#each preview.changes as change}{@const ops = changedOperations(
              change.before,
              change.after,
            )}{#if ops.length}<div>
                <strong>{change.display_name}</strong>
                <ul>
                  {#each ops as op}<li>
                      {operationLabel(op)}: {change.before[op]?.allowed ? 'Allowed' : 'Denied'} → {change
                        .after[op]?.allowed
                        ? 'Allowed'
                        : 'Denied'}
                    </li>{/each}
                </ul>
              </div>{/if}{#each change.children ?? [] as scope}{@const childOps = changedOperations(
                scope.before,
                scope.after,
              )}{#if childOps.length}<div>
                  <strong>{change.display_name} · {scope.child}</strong>
                  <ul>
                    {#each childOps as op}<li>
                        {operationLabel(op)}: {scope.before[op]?.allowed ? 'Allowed' : 'Denied'} → {scope
                          .after[op]?.allowed
                          ? 'Allowed'
                          : 'Denied'}
                      </li>{/each}
                  </ul>
                </div>{/if}{/each}{/each}{#if preview.changes.every((c) => changedOperations(c.before, c.after).length === 0 && (c.children ?? []).every((s) => changedOperations(s.before, s.after).length === 0))}<p
            >
              No effective operation changes for existing users.
            </p>{/if}
        </section>{/if}
    {:else}
      <div class="fields">
        <label
          >User<select
            aria-label="User"
            bind:value={userId}
            onchange={() => {
              effective = null;
            }}
            ><option value="">Choose user</option>{#each subjects.users as u}<option value={u.id}
                >{u.display_name} (@{u.username})</option
              >{/each}</select
          ></label
        ><label
          >Child scope<input
            bind:value={child}
            oninput={() => {
              effective = null;
            }}
            placeholder={scopeHint[kind]}
          /></label
        ><button class="btn" disabled={!userId} onclick={inspect}>Check access</button>
      </div>
      <p class="hint">
        Saved permissions for this user and scope. Native credential and operation approval checks
        also run when executing.
      </p>
      {#if effective}<div class="decisions">
          {#each Object.entries(effective.operations) as [op, decision]}<div class="decision">
              <strong>{operationLabel(op)}</strong><span class:allowed={decision.allowed}
                >{decision.allowed ? 'Allowed' : 'Denied'}</span
              >
              <p>{decision.reason}</p>
              {#if decision.matched_rule_ids.length}<small
                  >Matching rules: {decision.matched_rule_ids
                    .map((id) => {
                      const index = policy?.rules.findIndex((r) => r.id === id) ?? -1;
                      return index < 0 ? id : `Rule ${index + 1}`;
                    })
                    .join(', ')}</small
                >{/if}
            </div>{/each}
        </div>{/if}
    {/if}
  {/if}
</section>

<style>
  .access-editor {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    padding: 16px;
    color: var(--text);
  }
  header {
    display: flex;
    align-items: start;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  h3,
  h4,
  p {
    margin: 0;
  }
  h3 {
    font-size: 17px;
  }
  header p,
  .hint {
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.5;
    max-width: 78ch;
  }
  nav,
  .actions,
  .rule-head {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .rule-head {
    justify-content: space-between;
  }
  fieldset {
    border: 0;
    padding: 0;
    margin: 0;
    min-width: 0;
  }
  .rule {
    border-block-start: 1px solid var(--border);
    padding: 16px 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .fields {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    align-items: end;
  }
  .fields > label {
    flex: 1;
    min-width: 140px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
  }
  input,
  select,
  textarea {
    max-width: 100%;
    min-width: 0;
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    padding: 7px;
    border-radius: var(--radius-s);
  }
  textarea {
    resize: vertical;
    width: 100%;
    box-sizing: border-box;
  }
  .operations {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
    gap: 9px;
  }
  .operations label {
    flex-direction: row;
    align-items: center;
  }
  .operations input {
    flex-shrink: 0;
  }
  .error {
    color: var(--status-exited, #ed635c);
    overflow-wrap: anywhere;
  }
  .preview {
    padding: 14px;
    background: var(--surface-2);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .decision {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 5px;
    border-block-end: 1px solid var(--border);
    padding: 10px 0;
    font-size: 12px;
  }
  .decision p,
  .decision small {
    grid-column: 1/-1;
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .allowed {
    color: var(--status-working, #36a66f);
  }
  details summary {
    cursor: pointer;
    font-size: 12px;
  }
  details[open] {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .mode {
    max-width: 360px;
  }
</style>
