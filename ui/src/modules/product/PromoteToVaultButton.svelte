<script lang="ts">
  // 1-click "Promote to Vault…" — creates an OKF RFC/ADR markdown note in an
  // existing Vault with frontmatter back-linking the Product story, then opens it.
  import { onDestroy } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { listVaults, writeVaultNote } from '../../lib/api/vault';
  import type { Vault } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { announceShown } from '../../lib/announce.svelte';
  import type { ProductStory } from './types';
  import {
    buildOkfStoryNote,
    defaultVaultNotePath,
    normalizeVaultNotePath,
    type PromoteDocKind,
  } from './vaultPromote';

  interface Props {
    story: ProductStory;
    bodyMd?: string | null;
    planMd?: string | null;
    defaultKind?: PromoteDocKind;
  }
  let { story, bodyMd = null, planMd = null, defaultKind = 'rfc' }: Props = $props();

  let open = $state(false);
  let vaults = $state<Vault[]>([]);
  let loadingVaults = $state(false);
  let vaultsError = $state<string | null>(null);
  let selectedVaultId = $state<number | null>(null);
  let kind = $state<PromoteDocKind>('rfc');
  let notePath = $state('');
  let pathTouched = $state(false);
  let includePlan = $state(true);
  let saving = $state(false);
  let saveError = $state<string | null>(null);
  let alive = true;
  let loadSeq = 0;

  onDestroy(() => {
    alive = false;
    ++loadSeq;
  });

  async function loadVaults(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      vaults = [];
      vaultsError = 'Select a workspace first.';
      return;
    }
    const seq = ++loadSeq;
    loadingVaults = true;
    vaultsError = null;
    try {
      const list = await listVaults(wsId);
      if (!alive || seq !== loadSeq) return;
      vaults = list;
      if (!list.some((v) => v.id === selectedVaultId)) {
        selectedVaultId = list[0]?.id ?? null;
      }
    } catch (e) {
      if (!alive || seq !== loadSeq) return;
      vaultsError = loadErrorText(e);
    } finally {
      if (alive && seq === loadSeq) loadingVaults = false;
    }
  }

  function openModal(): void {
    kind = defaultKind;
    notePath = defaultVaultNotePath(story, defaultKind);
    pathTouched = false;
    includePlan = Boolean(planMd?.trim());
    saveError = null;
    open = true;
    void loadVaults();
  }

  function setKind(next: PromoteDocKind): void {
    kind = next;
    if (!pathTouched) notePath = defaultVaultNotePath(story, next);
  }

  async function submit(): Promise<void> {
    if (saving) return;
    const wsId = ws.currentId;
    if (!wsId || selectedVaultId == null) return;
    const normalized = normalizeVaultNotePath(notePath);
    if (!normalized) {
      saveError = 'Enter a valid relative markdown path (without “..”).';
      return;
    }
    saving = true;
    saveError = null;
    try {
      const content = buildOkfStoryNote({
        story,
        kind,
        bodyMd,
        planMd: includePlan ? planMd : null,
      });
      await writeVaultNote(wsId, selectedVaultId, { path: normalized, content });
      if (!alive) return;
      const targetVaultId = selectedVaultId;
      open = false;
      toasts.success(
        `Promoted to Vault ${kind.toUpperCase()}`,
        `${normalized} — back-linked to ${story.source_key || story.id}.`,
      );
      router.go(`vault/${targetVaultId}/${encodeURIComponent(normalized)}`);
    } catch (e) {
      if (!alive) return;
      saveError = loadErrorText(e);
    } finally {
      if (alive) saving = false;
    }
  }
</script>

<button
  type="button"
  class="btn small"
  onclick={openModal}
  title="Create an OKF RFC or ADR note in Vault back-linked to this story"
  data-testid="promote-to-vault-btn"
>
  <Icon name="book" size={12} /> Promote to Vault…
</button>

{#if open}
  <Modal title="Promote story to Vault" width={480} dismissable={!saving} onclose={() => (open = false)}>
    <div class="pv-form">
      <p class="pv-lead">
        Writes an OKF Markdown note with YAML frontmatter linking back to
        <strong>{story.source_key || story.title}</strong> (<code>otto://story/{story.id}</code>).
      </p>

      <LoadState
        what="vaults"
        variant="compact"
        loading={loadingVaults}
        error={vaultsError}
        empty={vaults.length === 0}
        onretry={() => void loadVaults()}
      >
        {#snippet emptyView()}
          <div class="pv-empty">
            <p class="pv-hint">No Vault is configured in this workspace yet. Add one in Vault first.</p>
            <button type="button" class="btn small" onclick={() => { open = false; router.go('vault'); }}>
              <Icon name="book" size={12} /> Open Vault
            </button>
          </div>
        {/snippet}

        <div class="pv-field">
          <label for="pv-vault">Vault</label>
          <select id="pv-vault" class="input" bind:value={selectedVaultId} disabled={saving}>
            {#each vaults as v (v.id)}
              <option value={v.id}>{v.name}</option>
            {/each}
          </select>
        </div>

        <div class="pv-field">
          <span class="pv-label" id="pv-kind-lbl">Document kind</span>
          <div class="pv-kinds" role="group" aria-labelledby="pv-kind-lbl">
            <button
              type="button"
              class="pill-toggle"
              class:on={kind === 'rfc'}
              aria-pressed={kind === 'rfc'}
              disabled={saving}
              onclick={() => setKind('rfc')}
            >RFC (Spec)</button>
            <button
              type="button"
              class="pill-toggle"
              class:on={kind === 'adr'}
              aria-pressed={kind === 'adr'}
              disabled={saving}
              onclick={() => setKind('adr')}
            >ADR (Decision)</button>
          </div>
        </div>

        <div class="pv-field">
          <label for="pv-path">Note path</label>
          <input
            id="pv-path"
            class="input"
            dir="ltr"
            bind:value={notePath}
            oninput={() => (pathTouched = true)}
            placeholder="rfcs/story-spec.md"
            disabled={saving}
          />
        </div>

        {#if planMd?.trim()}
          <label class="pv-check">
            <input type="checkbox" bind:checked={includePlan} disabled={saving} />
            <span>Include the current implementation plan</span>
          </label>
        {/if}

        {#if saveError}
          <p class="pv-err" use:announceShown={{ text: saveError, assertive: true }}>
            <Icon name="warning" size={13} /> {saveError}
          </p>
        {/if}
      </LoadState>
    </div>

    {#snippet footer()}
      <button type="button" class="btn" onclick={() => (open = false)} disabled={saving}>Cancel</button>
      <button
        type="button"
        class="btn primary"
        onclick={() => void submit()}
        disabled={saving || loadingVaults || selectedVaultId == null || !notePath.trim()}
        data-testid="promote-to-vault-confirm"
      >
        {saving ? 'Writing note…' : `Create ${kind.toUpperCase()} in Vault`}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .pv-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .pv-lead {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.45;
  }
  .pv-lead code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .pv-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .pv-field label,
  .pv-label {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text);
  }
  .pv-kinds {
    display: flex;
    gap: 8px;
  }
  .pv-check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .pv-empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }
  .pv-hint {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .pv-err {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
