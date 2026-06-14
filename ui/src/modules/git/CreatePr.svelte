<script lang="ts">
  // Create-PR sheet: source/target branch, title, markdown description.
  // The message can be drafted by an agent from the branch diff, and the branch
  // is pushed automatically (with --set-upstream) right before the PR is opened.
  import Modal from '../../lib/components/Modal.svelte';
  import { api } from '../../lib/api/client';
  import type { BranchInfo, DraftPrResp, PrSummary } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
    onclose: () => void;
    oncreated: (pr: PrSummary) => void;
  }
  let { repoId, onclose, oncreated }: Props = $props();

  let branches: BranchInfo[] = $state([]);
  let source = $state('');
  let target = $state('');
  let title = $state('');
  let description = $state('');
  let busy = $state(false);
  let drafting = $state(false);
  // What the Create button is doing, for the label.
  let phase: '' | 'pushing' | 'creating' = $state('');

  $effect(() => {
    void api
      .get<BranchInfo[]>(`/repos/${repoId}/branches`)
      .then((b) => {
        branches = b;
        if (source === '') source = b.find((x) => x.is_current)?.name ?? b[0]?.name ?? '';
        if (target === '') {
          target =
            b.find((x) => x.name === 'develop' || x.name === 'main' || x.name === 'master')?.name ??
            b.find((x) => x.name !== source)?.name ??
            b[0]?.name ??
            '';
        }
      })
      .catch(() => (branches = []));
  });

  // Ask an agent to draft the title + description from the branch's diff vs the
  // target. Fills the form; the user reviews/edits before creating.
  async function draftWithAgent(): Promise<void> {
    if (!target) {
      toasts.warn('Pick a target branch first');
      return;
    }
    drafting = true;
    try {
      const d = await api.post<DraftPrResp>(`/repos/${repoId}/pr/draft`, { base: target });
      title = d.title;
      description = d.description;
      if (d.source_branch) source = d.source_branch;
      toasts.info('Draft ready', 'Review and edit before creating.');
    } catch (e) {
      toasts.error('Draft failed', e instanceof Error ? e.message : String(e));
    } finally {
      drafting = false;
    }
  }

  async function create(): Promise<void> {
    busy = true;
    try {
      // Push the source branch first (sets upstream for a fresh branch) so the
      // provider can find it; non-fatal if it's already up to date.
      phase = 'pushing';
      try {
        await api.post(`/repos/${repoId}/push`);
      } catch (e) {
        // "Everything up-to-date" surfaces as a normal push; a real failure
        // (e.g. auth) should stop us before trying to open the PR.
        toasts.error('Push failed', e instanceof Error ? e.message : String(e));
        return;
      }
      phase = 'creating';
      const pr = await api.post<PrSummary>(`/repos/${repoId}/prs`, {
        title: title.trim(),
        description,
        source_branch: source,
        target_branch: target,
      });
      toasts.success('Pull request created', `#${pr.number} ${pr.title}`);
      oncreated(pr);
    } catch (e) {
      toasts.error('Create failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
      phase = '';
    }
  }
</script>

<Modal title="New Pull Request" width={520} {onclose}>
  <div class="row" style="gap: 12px; margin-bottom: 12px">
    <div class="field grow" style="margin: 0">
      <label for="pr-src">Source</label>
      <select id="pr-src" class="input" bind:value={source}>
        {#each branches as b (b.name)}<option value={b.name}>{b.name}</option>{/each}
      </select>
    </div>
    <span class="dim" style="margin-top: 16px">→</span>
    <div class="field grow" style="margin: 0">
      <label for="pr-tgt">Target</label>
      <select id="pr-tgt" class="input" bind:value={target}>
        {#each branches as b (b.name)}<option value={b.name}>{b.name}</option>{/each}
      </select>
    </div>
  </div>

  <div class="draft-row">
    <button class="btn small ghost" disabled={drafting || busy || target === ''} onclick={draftWithAgent}>
      {#if drafting}
        <span class="spinner-xs"></span>Drafting…
      {:else}
        <Icon name="zap" size={11} /> Draft message with agent
      {/if}
    </button>
    <span class="dim draft-hint">Generates the title + description from your branch diff vs {target || 'target'}.</span>
  </div>

  <div class="field">
    <label for="pr-title">Title</label>
    <input id="pr-title" class="input" bind:value={title} />
  </div>

  <div class="field">
    <label for="pr-desc">Description <span class="dim">(markdown)</span></label>
    <textarea id="pr-desc" class="input" rows="6" bind:value={description}></textarea>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button
      class="btn primary"
      disabled={busy || drafting || title.trim() === '' || source === '' || target === '' || source === target}
      onclick={create}
    >
      {phase === 'pushing' ? 'Pushing…' : phase === 'creating' ? 'Creating…' : 'Create Pull Request'}
    </button>
  {/snippet}
</Modal>

<style>
  .draft-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .draft-hint {
    font-size: 11px;
  }
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    vertical-align: middle;
    margin-right: 4px;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
