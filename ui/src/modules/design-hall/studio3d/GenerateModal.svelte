<script lang="ts">
  // ✨ Generate → "Text → 3D model" / "Image → 3D model". A provider picker
  // (proposal §5.4): the local path runs one Otto agent turn on this scene
  // (Blender MCP tool calls ask first); Tripo / Meshy are opt-in cloud
  // providers, disabled until a Keychain key exists, each showing where the
  // prompt goes and what it costs. Nothing is sent before the person presses
  // Generate, and cloud providers never run without a key.
  import { untrack } from 'svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { ApiError } from '../../../lib/api/client';
  import { assistArtifact, createArtifact } from '../../../lib/api/design';
  import type { DesignArtifact, DesignArtifactFormat, DesignAssistTurn } from '../../../lib/api/types';
  import {
    GEN3D_PROVIDERS,
    defaultProvider,
    localPrompt,
    unavailableReason,
    type Gen3dContext,
    type Gen3dKind,
    type Gen3dProviderId,
    type Gen3dQuality,
  } from './providers';

  interface Props {
    kind: Gen3dKind;
    artifact: DesignArtifact;
    ctx: Gen3dContext;
    onclose: () => void;
    /** The agent turn started (the studio shows it until it lands). */
    onstarted: (turn: DesignAssistTurn, label: string) => void;
  }
  let { kind, artifact, ctx, onclose, onstarted }: Props = $props();

  let prompt = $state('');
  let quality = $state<Gen3dQuality>('standard');
  // Preselect once (the picker is the person's choice after that).
  let providerId = $state<Gen3dProviderId>(untrack(() => defaultProvider(kind, ctx).id));
  let image = $state<File | null>(null);
  let imageUrl = $state<string | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  const provider = $derived(GEN3D_PROVIDERS.find((p) => p.id === providerId) ?? GEN3D_PROVIDERS[0]);
  const blocked = $derived(unavailableReason(provider, kind, ctx));
  const ready = $derived(!blocked && !busy && (kind === 'text' ? prompt.trim().length > 2 : !!image));

  $effect(() => () => {
    if (imageUrl) URL.revokeObjectURL(imageUrl);
  });

  function pickImage(e: Event): void {
    const f = (e.currentTarget as HTMLInputElement).files?.[0] ?? null;
    if (f && !/^image\/(png|jpeg|webp)$/.test(f.type)) {
      error = 'Pick a PNG, JPEG or WebP image.';
      return;
    }
    if (f && f.size > 10 * 1024 * 1024) {
      error = 'That image is over 10 MB.';
      return;
    }
    error = null;
    if (imageUrl) URL.revokeObjectURL(imageUrl);
    image = f;
    imageUrl = f ? URL.createObjectURL(f) : null;
  }

  function b64(f: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const fr = new FileReader();
      fr.onerror = () => reject(fr.error);
      fr.onload = () => resolve(String(fr.result).split(',')[1] ?? '');
      fr.readAsDataURL(f);
    });
  }

  async function generate(): Promise<void> {
    if (!ready || provider.where !== 'local') return;
    busy = true;
    error = null;
    try {
      // Image → 3D: the image becomes a reference design in the project so the
      // agent's brief carries it as [R1] (refs/R1.png) — nothing leaves the Mac.
      let refId: string | null = null;
      if (kind === 'image' && image) {
        const fmt = (image.type === 'image/jpeg' ? 'jpeg' : image.type === 'image/webp' ? 'webp' : 'png') as DesignArtifactFormat;
        const res = await createArtifact({
          workspace_id: artifact.workspace_id,
          project_id: artifact.project_id ?? undefined,
          studio: 'graphics',
          format: fmt,
          title: `${image.name.replace(/\.[a-z0-9]+$/i, '') || 'Reference'} (3D reference)`,
          content_b64: await b64(image),
          tags: ['3d-reference'],
          message: `Reference image for ${artifact.title}`,
        });
        refId = res.artifact.id;
      }
      const turn = await assistArtifact(artifact.id, {
        prompt: localPrompt(kind, prompt, quality, refId ? 'R1' : null),
        mode: 'refine',
        references: refId ? [refId] : undefined,
      });
      onstarted(turn, kind === 'text' ? `Modelling “${prompt.trim().slice(0, 40)}”` : 'Modelling from the image');
      onclose();
    } catch (e) {
      error =
        e instanceof ApiError && e.status === 409
          ? 'Otto is already working on this design — wait for that turn to finish.'
          : e instanceof Error
            ? e.message
            : String(e);
      toasts.error('Couldn’t start the generation', error);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={kind === 'text' ? 'Text → 3D model' : 'Image → 3D model'} width={560} {onclose}>
  <div class="gen" data-testid="gen3d-modal">
    {#if kind === 'text'}
      <label class="gfield">
        <span class="lbl">Describe the model</span>
        <textarea rows="3" bind:value={prompt} placeholder="A small gift box with a violet ribbon" maxlength="1200" data-testid="gen3d-prompt"></textarea>
      </label>
    {:else}
      <div class="gfield">
        <span class="lbl">Reference image</span>
        <div class="img-row">
          {#if imageUrl}
            <img class="preview" src={imageUrl} alt="The reference you picked" />
          {/if}
          <label class="btn small">
            <Icon name="image" size={12} /> {image ? 'Change image…' : 'Choose image…'}
            <input class="file" type="file" accept="image/png,image/jpeg,image/webp" onchange={pickImage} />
          </label>
        </div>
        <label class="gfield">
          <span class="lbl">Notes (optional)</span>
          <input type="text" bind:value={prompt} placeholder="Keep the proportions; use Brand violet" maxlength="600" />
        </label>
      </div>
    {/if}

    <fieldset class="providers">
      <legend class="lbl">Provider</legend>
      {#each GEN3D_PROVIDERS as p (p.id)}
        {@const why = unavailableReason(p, kind, ctx)}
        <label class="prov" class:off={!!why && p.where === 'cloud'} class:on={providerId === p.id}>
          <input type="radio" name="gen3d-provider" value={p.id} bind:group={providerId} disabled={p.where === 'cloud' && !!why} />
          <span class="prov-main">
            <span class="prov-name">
              {p.label}
              {#if p.where === 'cloud'}<span class="pill">opt-in</span>{/if}
            </span>
            <span class="prov-note">{p.privacy}</span>
            {#if p.cost}<span class="prov-note">Cost: {p.cost}</span>{/if}
            {#if why && p.where === 'cloud'}<span class="prov-why"><Icon name="lock" size={11} /> {why}</span>{/if}
          </span>
        </label>
      {/each}
    </fieldset>

    <label class="gfield inline">
      <span class="lbl">Quality</span>
      <select bind:value={quality}>
        <option value="draft">Draft — quick blockout</option>
        <option value="standard">Standard — cleaner model</option>
      </select>
    </label>

    {#if blocked && provider.where === 'local'}
      <p class="note warn" role="status"><Icon name="warning" size={12} /> <span>{blocked}</span></p>
    {:else}
      <p class="note"><Icon name="info" size={12} /> <span>The result lands as a new <strong>agent</strong> version of this scene — review it, keep it or restore the previous one.</span></p>
    {/if}
    {#if error}<p class="note err" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={!ready} onclick={() => void generate()} data-testid="gen3d-go">
      {busy ? 'Starting…' : 'Generate'}
    </button>
  {/snippet}
</Modal>

<style>
  .gen {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .gfield {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .gfield.inline {
    flex-direction: row;
    align-items: center;
    gap: 10px;
  }
  .lbl {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  textarea,
  input[type='text'],
  select {
    font: inherit;
    font-size: var(--fs-m);
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
  }
  textarea {
    resize: vertical;
  }
  .img-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .preview {
    width: 72px;
    height: 72px;
    object-fit: cover;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
  }
  .file {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
  }
  .providers {
    border: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .providers legend {
    margin-block-end: 6px;
  }
  .prov {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 9px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    cursor: pointer;
  }
  .prov.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .prov.off {
    cursor: not-allowed;
    color: var(--text-dim);
  }
  .prov input {
    margin-block-start: 3px;
    accent-color: var(--accent);
  }
  .prov-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .prov-name {
    font-size: var(--fs-m);
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .pill {
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 0 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .prov-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .prov-why {
    font-size: var(--fs-xs);
    color: var(--warning);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .note {
    margin: 0;
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note.warn {
    color: var(--warning);
  }
  .note.err {
    color: var(--danger);
  }
</style>
