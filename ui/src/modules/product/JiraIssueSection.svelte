<script lang="ts">
  // The Jira Details sidebar of a Jira story's Overview (architecture-5-04):
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

  interface Props {
    issue: JiraIssue;
    story: ProductStory;
    /** Section key → collapsed (shared with the tab's activity cards). */
    collapsed: Record<string, boolean>;
    ontoggle: (key: string) => void;
  }
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
</script>

<!-- Inline "Edit" pencil shown next to an editable detail value. Renders only
     when editmeta marks the field editable. `current` seeds the draft. -->
{#snippet editBtn(key: string, current: string)}
  {#if jira.editableFor(key)}
    <button
      class="field-edit-btn"
      data-edit-for={key}
      title="Edit {jira.editableFor(key)!.name}"
      aria-label="Edit {jira.editableFor(key)!.name}"
      onclick={() => jira.beginEdit(jira.editableFor(key)!, current)}
    >
      <Icon name="edit" size={12} />
    </button>
  {/if}
{/snippet}

<!-- Inline editor for a single editable field (jira/IssueEditor.svelte). -->
{#snippet fieldEditor(ef: EditableField)}
  <IssueEditor editor={jira} {ef} issueKey={story?.source_key ?? ''} />
{/snippet}

  <div class="jira-section">
    {#if issueLoading}
      <Skeleton rows={6} height={36} label="Jira details" />
    {:else if issueError}
      <LoadState what="Jira details" variant="compact" error={issueError} empty onretry={() => void issue.load()} />
    {:else if issueFull}

      <!-- ── Status + Transition ──────────────────────────── -->
      <div class="jira-card">
        <div class="jira-card-header">
          <span class="jira-section-label">Status</span>
          <div class="status-control">
            <span class="status-badge">{issueFull.status}</span>
            <button
              class="change-btn"
              onclick={(e) => void jira.openTransitionMenu(e)}
              disabled={jira.transitionWorking || jira.transitionsLoading}
              title="Change the status of {story.source_key} in Jira"
              aria-haspopup="menu"
              data-testid="ov-transition-btn"
            >
              {#if jira.transitionWorking}Working…{:else if jira.transitionsLoading}Loading transitions…{:else}Transition <Icon name="chevronDown" size={12} />{/if}
            </button>
          </div>
        </div>
      </div>

      <!-- ── Assignee ─────────────────────────────────────── -->
      <div class="jira-card">
        <div class="jira-card-header">
          <span class="jira-section-label">Assignee</span>
          <div class="assignee-control">
            {#if issueFull.assignee}
              <div class="user-row">
                {#if issueFull.assignee.avatar_url}
                  <img class="avatar" src={issueFull.assignee.avatar_url} alt={issueFull.assignee.display_name} />
                {:else}
                  <span class="avatar-placeholder">{issueFull.assignee.display_name.slice(0, 1).toUpperCase()}</span>
                {/if}
                <span class="user-name">{issueFull.assignee.display_name}</span>
              </div>
            {:else}
              <span class="unassigned">Unassigned</span>
            {/if}
            <button
              class="change-btn"
              onclick={(e) => void jira.openAssigneeMenu(e)}
              disabled={jira.assigneeWorking || jira.assignablesLoading}
              title="Change the assignee of {story.source_key} in Jira"
              aria-haspopup="menu"
              data-testid="ov-assignee-btn"
            >
              {#if jira.assigneeWorking}Working…{:else if jira.assignablesLoading}Loading people…{:else}Change <Icon name="chevronDown" size={12} />{/if}
            </button>
          </div>
        </div>
      </div>

      <!-- ── Details ─────────────────────────────────────── -->
      <div class="jira-card collapsible-card">
        <button
          class="jira-coll-trigger"
          onclick={() => toggle('details')}
          aria-expanded={!collapsed.details}
        >
          <span class="coll-arrow" aria-hidden="true"><Icon name={collapsed.details ? 'chevronRight' : 'chevronDown'} size={12} /></span>
          <span class="jira-section-label">Details</span>
        </button>
        {#if !collapsed.details}
          <div class="details-grid">
            {#if issueFull.reporter}
              <span class="detail-key">Reporter</span>
              <span class="detail-val">
                {#if jira.editingField === 'reporter' && jira.editableFor('reporter')}
                  {@render fieldEditor(jira.editableFor('reporter')!)}
                {:else}
                  <div class="detail-val-row">
                    <div class="user-row-sm">
                      {#if issueFull.reporter.avatar_url}
                        <img class="avatar-sm" src={issueFull.reporter.avatar_url} alt={issueFull.reporter.display_name} />
                      {/if}
                      {issueFull.reporter.display_name}
                    </div>
                    {@render editBtn('reporter', issueFull.reporter.display_name)}
                  </div>
                {/if}
              </span>
            {/if}

            <span class="detail-key">Priority</span>
            <span class="detail-val">
              {#if jira.editingField === 'priority' && jira.editableFor('priority')}
                {@render fieldEditor(jira.editableFor('priority')!)}
              {:else}
                <div class="detail-val-row">
                  <span>{issueFull.priority ?? '—'}</span>
                  {@render editBtn('priority', issueFull.priority ?? '')}
                </div>
              {/if}
            </span>

            <!-- Story Points / Estimate — always editable when editmeta exposes a
                 numeric estimate field, even if currently empty (the "add" case). -->
            {#if jira.estimateField}
              <span class="detail-key">{jira.estimateField.name}</span>
              <span class="detail-val">
                {#if jira.editingField === jira.estimateField.key}
                  {@render fieldEditor(jira.estimateField)}
                {:else}
                  <div class="detail-val-row">
                    {#if issueFull.estimate}
                      <span class="estimate-chip">{issueFull.estimate}</span>
                    {:else}
                      <span class="detail-empty">No estimate</span>
                    {/if}
                    {@render editBtn(jira.estimateField.key, jira.rawFieldValue(jira.estimateField.key))}
                  </div>
                {/if}
              </span>
            {:else if issueFull.estimate}
              <span class="detail-key">Estimate</span>
              <span class="detail-val"><span class="estimate-chip">{issueFull.estimate}</span></span>
            {/if}

            <span class="detail-key">Labels</span>
            <span class="detail-val">
              {#if jira.editingField === 'labels' && jira.editableFor('labels')}
                {@render fieldEditor(jira.editableFor('labels')!)}
              {:else}
                <div class="detail-val-row">
                  {#if issueFull.labels && issueFull.labels.length > 0}
                    <div class="label-chips">
                      {#each issueFull.labels as lbl (lbl)}
                        <span class="label-chip">{lbl}</span>
                      {/each}
                    </div>
                  {:else}
                    <span class="detail-empty">No labels</span>
                  {/if}
                  {@render editBtn('labels', (issueFull.labels ?? []).join(', '))}
                </div>
              {/if}
            </span>

            {#each issueFull.fields.filter((f) => f.value && f.value.trim() !== '') as field (field.key)}
              {#if field.key === jira.estimateField?.key || (jira.estimateField === null && /story\s*point/i.test(field.name))}
                <!-- already rendered as the dedicated estimate row above, or matches story-points heuristic -->
              {:else}
                <span class="detail-key">{field.name}</span>
                <span class="detail-val">
                  {#if jira.editingField === field.key && jira.editableFor(field.key)}
                    {@render fieldEditor(jira.editableFor(field.key)!)}
                  {:else}
                    <div class="detail-val-row">
                      <span>{field.value}</span>
                      {@render editBtn(field.key, field.value)}
                    </div>
                  {/if}
                </span>
              {/if}
            {/each}

            <!-- Empty-field add rows: editmeta fields with no current value,
                 not already rendered by a dedicated row above. -->
            {#if jira.editmetaError}
              <LoadState
                what="editable fields"
                variant="compact"
                error={jira.editmetaError}
                onretry={() => jira.retryEditmeta()}
              />
            {/if}
            {#if jira.editmeta}
              {#each jira.editmeta.filter((ef) => {
                // Skip fields rendered by dedicated controls above.
                if (ef.key === 'priority' || ef.key === 'labels') return false;
                if (jira.estimateField && ef.key === jira.estimateField.key) return false;
                if (!jira.estimateField && /story\s*point/i.test(ef.name)) return false;
                // Skip if there's already a populated value row for this field.
                const existing = issueFull?.fields.find((f) => f.key === ef.key);
                if (existing && existing.value && existing.value.trim() !== '') return false;
                return true;
              }) as ef (ef.key)}
                <span class="detail-key detail-key-empty">{ef.name}</span>
                <span class="detail-val">
                  {#if jira.editingField === ef.key}
                    {@render fieldEditor(ef)}
                  {:else}
                    <div class="detail-val-row">
                      <span class="detail-empty">—</span>
                      <button
                        class="field-edit-btn"
                        title="Set {ef.name}"
                        aria-label="Set {ef.name}"
                        onclick={() => jira.beginEdit(ef, '')}
                      ><Icon name="plus" size={12} /></button>
                    </div>
                  {/if}
                </span>
              {/each}
            {/if}
          </div>
        {/if}
      </div>

      <!-- ── Linked Issues ───────────────────────────────── -->
      {#if issueFull.links && issueFull.links.length > 0}
        <div class="jira-card collapsible-card">
          <button
            class="jira-coll-trigger"
            onclick={() => toggle('links')}
            aria-expanded={!collapsed.links}
          >
            <span class="coll-arrow" aria-hidden="true"><Icon name={collapsed.links ? 'chevronRight' : 'chevronDown'} size={12} /></span>
            <span class="jira-section-label">Linked issues</span>
            <span class="section-count">({issueFull.links.length})</span>
          </button>
          {#if !collapsed.links}
            <div class="links-list">
              {#each issueFull.links as lnk (lnk.key)}
                <div class="link-row">
                  <span class="link-rel">{lnk.rel}</span>
                  <span class="link-key mono-sm">{lnk.key}</span>
                  <span class="link-type chip-sm">{lnk.issue_type}</span>
                  <span class="link-summary">{lnk.summary}</span>
                  <span class="link-status status-sm">{lnk.status}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      <!-- ── Development (linked branches / commits / PRs) ─── -->
      <div class="jira-card collapsible-card">
        <button
          class="jira-coll-trigger"
          onclick={() => toggle('development')}
          aria-expanded={!collapsed.development}
        >
          <span class="coll-arrow" aria-hidden="true"><Icon name={collapsed.development ? 'chevronRight' : 'chevronDown'} size={12} /></span>
          <span class="jira-section-label">Development</span>
          {#if devStatus}
            <span class="section-count">
              ({devStatus.branches.length + devStatus.commits.length + devStatus.pull_requests.length})
            </span>
          {/if}
        </button>
        {#if !collapsed.development}
          <div class="dev-body">
            {#if devLoading || devError}
              <LoadState what="development info" variant="compact" loading={devLoading} error={devError} empty onretry={() => { issue.devLoaded = false; void issue.loadDev(); }} />
            {:else if devStatus && (devStatus.branches.length || devStatus.commits.length || devStatus.pull_requests.length)}
              {#if devStatus.pull_requests.length}
                <div class="dev-group">
                  <span class="dev-group-label">Pull requests</span>
                  {#each devStatus.pull_requests as pr (pr.repo + ':' + pr.id)}
                    <a
                      class="dev-row"
                      href={pr.url}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      <span class="dev-pr-status status-sm">{pr.status}</span>
                      <span class="dev-pr-name">{pr.name}</span>
                      <span class="dev-repo chip-sm">{pr.repo}</span>
                    </a>
                  {/each}
                </div>
              {/if}
              {#if devStatus.branches.length}
                <div class="dev-group">
                  <span class="dev-group-label">Branches</span>
                  {#each devStatus.branches as b (b.repo + ':' + b.name)}
                    <a
                      class="dev-row"
                      href={b.url}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      <span class="dev-branch-name mono-sm">{b.name}</span>
                      <span class="dev-repo chip-sm">{b.repo}</span>
                    </a>
                  {/each}
                </div>
              {/if}
              {#if devStatus.commits.length}
                <div class="dev-group">
                  <span class="dev-group-label">Commits</span>
                  {#each devStatus.commits as c (c.repo + ':' + c.id)}
                    <a
                      class="dev-row"
                      href={c.url}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      <span class="dev-commit-id mono-sm">{c.id.slice(0, 8)}</span>
                      <span class="dev-commit-msg">{c.message}</span>
                      <span class="dev-repo chip-sm">{c.repo}</span>
                    </a>
                  {/each}
                </div>
              {/if}
            {:else}
              <div class="comments-empty">
                No linked development info — connect GitHub/Bitbucket in Jira to see
                branches, commits and PRs here.
              </div>
            {/if}
          </div>
        {/if}
      </div>

    {/if}
  </div>

<style>

  /* ── Jira section (right column) ──────────────────────────── */
  .jira-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* ── Jira card ─────────────────────────────────────────────── */
  .jira-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 14px;
    background: var(--surface);
  }
  .collapsible-card {
    padding: 0;
  }
  .jira-card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .jira-section-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }

  /* ── Collapsible trigger ───────────────────────────────────── */
  .jira-coll-trigger {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    background: none;
    border: none;
    padding: 10px 14px;
    cursor: pointer;
    text-align: start;
    color: var(--text-dim);
    font-size: var(--fs-s);
    border-radius: var(--radius-s);
    transition: background var(--dur-fast);
  }
  .jira-coll-trigger:hover {
    background: var(--hover);
  }
  .coll-arrow {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }
  .section-count {
    font-size: var(--fs-xs);
    font-weight: 400;
    color: var(--text-dim);
    margin-inline-start: 2px;
  }

  /* ── Status control ────────────────────────────────────────── */
  .status-control {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status-badge {
    font-size: var(--fs-s);
    font-weight: 600;
    padding: 2px 10px;
    border-radius: 999px;
    /* A Jira workflow status is a value, not a selection: neutral pill. */
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
  }

  /* ── Assignee control ──────────────────────────────────────── */
  .assignee-control {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .user-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .user-row-sm {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
  }
  .avatar {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }
  .avatar-sm {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }
  .avatar-placeholder {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--accent-soft-strong);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .user-name {
    font-size: var(--fs-s);
    color: var(--text);
  }
  .unassigned {
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-style: italic;
  }

  /* ── Status / assignee pickers (menus are the global ctxMenu) ── */
  .change-btn {
    height: 24px;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--dur-fast), color var(--dur-fast);
  }
  .change-btn:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .change-btn:disabled {
    opacity: var(--disabled-opacity);
    cursor: not-allowed;
  }

  /* ── Details grid ──────────────────────────────────────────── */
  .details-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    padding: 0 14px 12px;
    align-items: start;
  }
  .detail-key {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
    padding-top: 1px;
  }
  .detail-key-empty {
    opacity: 0.65;
  }
  .detail-val {
    font-size: var(--fs-s);
    color: var(--text);
    line-height: 1.4;
  }
  .label-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .label-chip {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }

  /* ── Editable detail fields ────────────────────────────────── */
  .detail-val-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .detail-empty {
    color: var(--text-dim);
    font-style: italic;
  }
  .field-edit-btn {
    flex-shrink: 0;
    display: inline-grid;
    place-items: center;
    width: 20px;
    height: 20px;
    line-height: 1;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--dur-fast), background var(--dur-fast), color var(--dur-fast);
  }
  .detail-val-row:hover .field-edit-btn,
  .detail-val-row:focus-within .field-edit-btn,
  .field-edit-btn:focus-visible {
    opacity: 1;
  }
  /* Touch: no hover to reveal it, so it stays visible. */
  @media (hover: none) {
    .field-edit-btn {
      opacity: 1;
    }
  }
  .field-edit-btn:hover {
    background: var(--hover);
    color: var(--text);
  }

  /* ── Links ─────────────────────────────────────────────────── */
  .links-list {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 0 14px 10px;
  }
  .link-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .link-rel {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    min-width: 70px;
  }
  .link-key {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    font-family: var(--font-mono);
  }
  .chip-sm {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text-dim);
  }
  .link-summary {
    font-size: var(--fs-s);
    color: var(--text);
    flex: 1;
    min-width: 120px;
  }
  .status-sm {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .mono-sm {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }

  /* ── Development (branches / commits / PRs) ────────────────── */
  .dev-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 0 14px 10px;
  }
  .dev-group {
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .dev-group-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    padding: 6px 0 2px;
  }
  .dev-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
    text-decoration: none;
    color: inherit;
  }
  .dev-row:hover {
    background: var(--accent-faint);
  }
  .dev-pr-name,
  .dev-commit-msg {
    font-size: var(--fs-s);
    color: var(--text);
    flex: 1;
    min-width: 120px;
  }
  .dev-branch-name,
  .dev-commit-id {
    color: var(--accent-text);
  }
  .dev-pr-status {
    text-transform: uppercase;
    letter-spacing: .06em;
  }
  .dev-repo {
    margin-inline-start: auto;
  }
  .comments-empty {
    padding: 10px 14px 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-style: italic;
  }
  /* ── Estimate chip ─────────────────────────────────────────── */
  .estimate-chip {
    display: inline-block;
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--warning) 15%, transparent);
    color: var(--warning);
    border: 1px solid color-mix(in srgb, var(--warning) 30%, transparent);
  }
</style>
