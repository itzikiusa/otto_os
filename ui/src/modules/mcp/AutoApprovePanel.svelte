<script lang="ts">
  // Auto-approve rules: the explicit, opt-in list of mutating otto.* tools that
  // run WITHOUT a per-call approval — per tool or per category, everywhere / in
  // one workspace / for one agent session. Off by default; every auto-approved
  // call is still audited ("Auto approved", naming the rule). Irreversible
  // tools need a per-tool rule plus a second confirmation.
  import Icon from '../../lib/components/Icon.svelte';
  import { toastError } from '../../lib/toastError';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { McpAutoApproveList, McpAutoApproveRule } from '../../lib/api/types';
  import { mcpCpExtraApi } from './cp-api';
  import AutoApproveForm from './AutoApproveForm.svelte';

  interface Props {
    isMcpAdmin: boolean;
    /** Bumped by the parent after it changed rules (catalog switches). */
    revision: number;
    /** Tell the parent the rules changed (its catalog badges reload). */
    onchange: () => void;
  }
  let { isMcpAdmin, revision, onchange }: Props = $props();

  let data = $state<McpAutoApproveList | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let formOpen = $state(false);

  const rules = $derived(data?.rules ?? []);
  const activeCount = $derived(rules.filter((r) => r.enabled).length);

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      data = await mcpCpExtraApi.autoApproveRules();
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void revision;
    void load();
  });

  function targetLabel(rule: McpAutoApproveRule): string {
    return rule.target_kind === 'category' ? `${rule.target} writes` : `otto.${rule.target}`;
  }

  function scopeLabel(rule: McpAutoApproveRule): string {
    if (rule.scope === 'workspace') return `Workspace · ${rule.workspace_name ?? rule.workspace_id}`;
    if (rule.scope === 'session') return `Session · ${rule.session_title ?? rule.session_id}`;
    return 'Everywhere';
  }

  async function setEnabled(rule: McpAutoApproveRule, enabled: boolean, input: HTMLInputElement): Promise<void> {
    if (
      enabled &&
      !(await confirmer.ask(
        `Turn “${rule.name}” back on? ${targetLabel(rule)} will run without asking (${scopeLabel(rule).toLowerCase()}). Every call is still audited.`,
        { title: 'Auto-approve', confirmLabel: 'Turn on', danger: true },
      ))
    ) {
      input.checked = false;
      return;
    }
    busy = rule.id;
    try {
      await mcpCpExtraApi.updateAutoApprove(rule.id, { enabled });
      await load();
      onchange();
    } catch (e) {
      input.checked = rule.enabled;
      toastError('Couldn’t update the rule', e);
    } finally {
      busy = null;
    }
  }

  async function remove(rule: McpAutoApproveRule): Promise<void> {
    if (
      !(await confirmer.ask(`Delete “${rule.name}”? ${targetLabel(rule)} will ask before each call again.`, {
        title: 'Delete auto-approve rule',
        confirmLabel: 'Delete',
        danger: true,
      }))
    )
      return;
    busy = rule.id;
    try {
      await mcpCpExtraApi.deleteAutoApprove(rule.id);
      toasts.success('Rule deleted', `${targetLabel(rule)} asks again`);
      await load();
      onchange();
    } catch (e) {
      toastError('Couldn’t delete the rule', e);
    } finally {
      busy = null;
    }
  }
</script>

<section class="panel" data-testid="mcp-auto-approve-panel">
  <div class="head">
    <div class="title">
      <Icon name="shield" size={14} />
      <h4 class="sec">Auto-approve</h4>
      {#if activeCount}<span class="count">{activeCount} on</span>{/if}
    </div>
    <button
      class="btn small"
      data-testid="mcp-auto-approve-new"
      disabled={!isMcpAdmin || !data}
      onclick={() => (formOpen = true)}
    >
      <Icon name="plus" size={12} /> New rule
    </button>
  </div>
  <p class="muted small intro">
    Mutating tools ask a person before each call. A rule lets one tool — or every mutating tool in a
    category — run without asking, everywhere, in one workspace, or for one agent session. Off by
    default; every auto-approved call is still in <em>Activity → Audit</em> as “Auto approved”, naming
    the rule. Irreversible tools (merging a PR, cluster actions, producing to a live queue or topic,
    arbitrary HTTP requests, hard deletes) are never covered by a category and need a second
    confirmation per tool.
  </p>

  <LoadState what="auto-approve rules" {loading} error={loadError} empty={!!data && rules.length === 0} rows={2} onretry={() => void load()}>
    {#snippet emptyView()}
      <EmptyState
        icon="shield"
        title="No auto-approve rules"
        body="Every mutating call asks a person first."
      />
    {/snippet}
    <ul class="rules">
      {#each rules as rule (rule.id)}
        <li class="rule" class:off={!rule.enabled} data-testid={`mcp-auto-approve-rule-${rule.target}`}>
          <label class="switch" title={rule.enabled ? 'On — runs without asking' : 'Off — asks before each call'}>
            <input
              type="checkbox"
              checked={rule.enabled}
              aria-label={`${rule.name} enabled`}
              disabled={!isMcpAdmin || busy === rule.id}
              onchange={(event) => void setEnabled(rule, event.currentTarget.checked, event.currentTarget)}
            />
          </label>
          <span class="meta">
            <span class="name">{rule.name}</span>
            <span class="chips">
              <span class="aa-chip mono">{targetLabel(rule)}</span>
              <span class="aa-chip">{scopeLabel(rule)}</span>
              {#if rule.allow_irreversible}<span class="aa-chip danger">Irreversible allowed</span>{/if}
              {#if rule.irreversible && !rule.allow_irreversible}<span class="aa-chip">Inactive — irreversible not confirmed</span>{/if}
            </span>
          </span>
          <button
            class="icon-btn"
            aria-label={`Delete ${rule.name}`}
            title="Delete rule"
            disabled={!isMcpAdmin || busy === rule.id}
            onclick={() => void remove(rule)}
          >
            <Icon name="trash" size={13} />
          </button>
        </li>
      {/each}
    </ul>
  </LoadState>
</section>

{#if formOpen && data}
  <AutoApproveForm
    catalog={data.categories}
    onclose={() => (formOpen = false)}
    onsaved={() => {
      void load();
      onchange();
    }}
  />
{/if}

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 14px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
  }
  .sec {
    margin: 0;
    font-size: var(--fs-s);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .count {
    font-size: var(--fs-xs);
    color: var(--warning);
    background: var(--warning-soft);
    border-radius: 999px;
    padding: 0 6px;
  }
  .intro {
    margin: 0;
  }
  .rules {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .rule {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .rule:last-child {
    border-bottom: none;
  }
  .rule.off .name {
    color: var(--text-dim);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .name {
    font-size: var(--fs-m);
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .aa-chip {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 1px 6px;
    overflow-wrap: anywhere;
  }
  .aa-chip.danger {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
</style>
