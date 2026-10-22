<script lang="ts">
  // Create / edit / duplicate an MCP profile: a name, the tool groups it is
  // built from, and per-tool overrides (a tool its groups leave out can be
  // added, one they include can be removed). "All tools" is the Full shape —
  // every tool minus the removed ones.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { mcpCpApi } from '../../lib/api/mcp';
  import type { McpProfile, McpToolGroup } from '../../lib/api/types';
  import {
    draftToReq,
    profileAllows,
    profileTools,
    toggleGroup,
    toggleTool,
    validateDraft,
    type ProfileDraft,
  } from '../../lib/mcpProfiles';

  interface Props {
    /** The profile being edited; null creates (from `initial`). */
    editing: McpProfile | null;
    initial: ProfileDraft;
    profiles: McpProfile[];
    groups: McpToolGroup[];
    onclose: () => void;
    onsaved: (p: McpProfile) => void;
  }
  let { editing, initial, profiles, groups, onclose, onsaved }: Props = $props();

  // `initial` arrives as a $state proxy, which structuredClone refuses (DataCloneError).
  // svelte-ignore state_referenced_locally
  let draft = $state<ProfileDraft>($state.snapshot(initial) as ProfileDraft);
  let busy = $state(false);
  let touched = $state(false);
  let open = $state<Record<string, boolean>>({});
  const error = $derived(validateDraft(draft, profiles, editing?.id ?? null));
  const count = $derived(profileTools(groups, draft).length);

  const inGroup = (g: McpToolGroup) => draft.all_tools || draft.groups.includes(g.id);

  async function save(): Promise<void> {
    touched = true;
    if (error || busy) return;
    busy = true;
    try {
      const saved = editing
        ? await mcpCpApi.cpUpdateProfile(editing.id, draftToReq(draft))
        : await mcpCpApi.cpCreateProfile(draftToReq(draft));
      toasts.success(editing ? 'Profile saved' : 'Profile created', saved.name);
      onsaved(saved);
      onclose();
    } catch (e) {
      toastError(editing ? 'Couldn’t save the profile' : 'Couldn’t create the profile', e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={editing ? `Edit ${editing.name}` : 'New MCP profile'} width={560} {onclose}>
  <div class="field">
    <label for="mcp-profile-name">Name</label>
    <input id="mcp-profile-name" class="input" dir="auto" maxlength="60" bind:value={draft.name} aria-invalid={touched && !!error} aria-describedby="mcp-profile-err" />
  </div>
  <div class="field">
    <label for="mcp-profile-desc">Description <span class="dim">(optional)</span></label>
    <input id="mcp-profile-desc" class="input" dir="auto" bind:value={draft.description} placeholder="What sessions use this for" />
  </div>
  <label class="toggle-row">
    <input type="checkbox" bind:checked={draft.all_tools} data-testid="mcp-profile-all" />
    <span><strong>All tools</strong> <span class="dim">— every Otto tool, minus any you remove below</span></span>
  </label>

  <div class="field">
    <span class="field-label" id="mcp-profile-groups">Tool groups <span class="dim">· {count} tools listed</span></span>
    <ul class="groups" aria-labelledby="mcp-profile-groups">
      {#each groups as g (g.id)}
        <li class="group">
          <div class="g-row">
            <label class="g-pick" title={g.description}>
              <input
                type="checkbox"
                checked={inGroup(g)}
                disabled={draft.all_tools}
                data-testid="mcp-profile-group-{g.id}"
                onchange={() => (draft = toggleGroup(draft, g.id))}
              />
              <span class="g-name">{g.label}</span>
              <span class="dim g-count">{g.tools.length}{g.prefixes.length ? '+' : ''}</span>
            </label>
            <button
              type="button"
              class="icon-btn"
              aria-expanded={!!open[g.id]}
              aria-label="{open[g.id] ? 'Hide' : 'Show'} {g.label} tools"
              title="{open[g.id] ? 'Hide' : 'Show'} tools"
              onclick={() => (open = { ...open, [g.id]: !open[g.id] })}
            ><Icon name={open[g.id] ? 'chevronDown' : 'chevronRight'} noflip size={12} /></button>
          </div>
          {#if open[g.id]}
            <p class="dim small g-desc">{g.description}{g.prefixes.length ? ` Also every ${g.prefixes.map((p) => p + '*').join(', ')} tool.` : ''}</p>
            <ul class="tools">
              {#each g.tools as t (t)}
                <li>
                  <label class="t-pick mono">
                    <input type="checkbox" checked={profileAllows(groups, draft, t)} onchange={() => (draft = toggleTool(groups, draft, t))} />
                    {t}
                  </label>
                </li>
              {/each}
            </ul>
          {/if}
        </li>
      {/each}
    </ul>
    <span class="hint">A profile only narrows what an agent sees — permissions and approvals still apply.</span>
  </div>
  <p id="mcp-profile-err" class="err" role="status">{touched ? error : ''}</p>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={busy || (touched && !!error)} onclick={() => void save()} data-testid="mcp-profile-save">
      {busy ? 'Saving…' : editing ? 'Save' : 'Create profile'}
    </button>
  {/snippet}
</Modal>

<style>
  .dim {
    color: var(--text-dim);
    font-weight: 400;
  }
  .small {
    font-size: var(--fs-xs);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .toggle-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin-bottom: 12px;
    font-size: var(--fs-s);
  }
  .groups,
  .tools {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .groups {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    max-height: 46vh;
    overflow-y: auto;
  }
  .group {
    padding: 4px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .group:last-child {
    border-bottom: none;
  }
  .g-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .g-pick {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1 1 auto;
    min-width: 0;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .g-count {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .g-desc {
    margin-block: 2px 4px;
    margin-inline: 24px 0;
  }
  .tools {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 2px 10px;
    padding-block: 0 6px;
    padding-inline: 24px 0;
  }
  .t-pick {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .err {
    margin: 0;
    min-height: 1em;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
