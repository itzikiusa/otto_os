<script lang="ts">
  // "New brand kit": a name and one of three starter kits (each a complete,
  // contrast-checked otto-brand/1 document). Creates the kit as a draft in the
  // current workspace — nothing leaves the Mac.
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { createArtifact } from '../../../lib/api/design';
  import { STARTER_KITS, type StarterKit } from './starters';
  import { brandPalette, serializeBrandDoc } from './tokens';

  interface Props {
    workspaceId: string;
    projectId?: string | null;
    onclose: () => void;
    oncreated: (id: string) => void;
  }
  let { workspaceId, projectId = null, onclose, oncreated }: Props = $props();

  let name = $state('');
  let starter = $state<StarterKit['id']>('vivid');
  let busy = $state(false);

  async function create(): Promise<void> {
    const title = name.trim() || 'Brand kit';
    const kit = STARTER_KITS.find((k) => k.id === starter) ?? STARTER_KITS[0];
    busy = true;
    try {
      const res = await createArtifact({
        workspace_id: workspaceId,
        studio: 'brand',
        format: 'otto-brand',
        title,
        content: serializeBrandDoc(kit.build(title)),
        message: `Started from the “${kit.label}” starter kit`,
        ...(projectId ? { project_id: projectId } : {}),
      });
      oncreated(res.artifact.id);
    } catch (e) {
      toasts.error('Couldn’t create the brand kit', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="New brand kit" width={620} {onclose}>
  <form
    class="nk"
    onsubmit={(e) => {
      e.preventDefault();
      void create();
    }}
    data-testid="brand-new-modal"
  >
    <label class="field">
      <span>Name</span>
      <input class="input" bind:value={name} placeholder="Acme brand" maxlength="200" />
    </label>
    <fieldset>
      <legend>Start from</legend>
      <div class="kits">
        {#each STARTER_KITS as k (k.id)}
          {@const doc = k.build(k.label)}
          {@const pal = brandPalette(doc)}
          <label class="kit" class:on={starter === k.id}>
            <input type="radio" name="starter" value={k.id} bind:group={starter} />
            <span class="sws" aria-hidden="true">
              {#each [pal.primary, pal.accent, pal.ink, pal.surfaceAlt] as c, i (i)}<i style:background={c}></i>{/each}
            </span>
            <b>{k.label}</b>
            <span class="blurb">{k.blurb}</span>
          </label>
        {/each}
      </div>
    </fieldset>
    <p class="hint"><Icon name="info" size={13} /> Every colour, type size and spacing step can be changed after. Studios read the kit once you link it.</p>
    <button type="submit" hidden aria-hidden="true" tabindex="-1"></button>
  </form>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={() => void create()} disabled={busy} data-testid="brand-new-create">{busy ? 'Creating…' : 'Create brand kit'}</button>
  {/snippet}
</Modal>

<style>
  .nk {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
  }
  legend {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin-block-end: 6px;
    padding: 0;
  }
  .kits {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: 10px;
  }
  .kit {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    cursor: pointer;
    position: relative;
  }
  .kit:hover {
    border-color: var(--border-strong);
  }
  .kit.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .kit input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .kit:focus-within {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .sws {
    display: flex;
    gap: 4px;
  }
  .sws i {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 1px solid var(--border);
  }
  .blurb {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.4;
  }
  .hint {
    margin: 0;
    display: flex;
    gap: 6px;
    align-items: flex-start;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .hint > :global(svg) {
    color: var(--info);
    flex: none;
    margin-block-start: 2px;
  }
</style>
