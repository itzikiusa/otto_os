<script lang="ts">
  // Native 3D Asset Generator Modal. Otto Forge (local) is THE generator: with
  // nothing else set up there is no provider picker, and a Mac without Forge gets
  // one install step instead of a form. Tencent Hunyuan3D-Omni (multi-modal
  // conditioning: point/voxel/bbox/pose), Microsoft TRELLIS.2 (PBR O-Voxel GLB),
  // user-added custom HTTP/CLI providers and Local Otto + Blender MCP wait behind
  // a closed "Advanced" disclosure until one is set up (enabled and ready).
  import { onDestroy, untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { ApiError } from '../../../lib/api/client';
  import { createArtifact } from '../../../lib/api/design';
  import * as mapi from '../../../lib/api/media';
  import { loadErrorText } from '../../../lib/loadError';
  import { pollWhileVisible, type Poller } from '../../../lib/poll';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { startAssist } from '../assist/api';
  import Gen3dProviderList from './Gen3dProviderList.svelte';
  import Provider3DManagerModal from './Provider3DManagerModal.svelte';
  import {
    OTTO_FORGE_ID,
    licenseWarningFor,
    localPrompt,
    pickDefault3dProvider,
    split3dProviders,
    type Gen3dContext,
    type Gen3dKind,
    type Gen3dQuality,
  } from './providers';
  import type {
    DesignArtifact,
    DesignArtifactFormat,
    DesignAssistTurn,
    GameAssetPreset,
    Media3dControlType,
    Media3dJob,
    Media3dOutputFormat,
    Media3dPipelineStage,
    Media3dProvider,
  } from '../../../lib/api/types';

  interface Props {
    kind: Gen3dKind;
    artifact: DesignArtifact;
    ctx: Gen3dContext;
    /** Pipeline stage (`generate` default, or `gameready` / `rig` / `animate` on the current 3D model). */
    stage?: Media3dPipelineStage;
    sourceVersionId?: string | null;
    /** Pre-filled prompt (e.g. from "Regenerate with Polygon Feedback"). */
    initialPrompt?: string;
    onclose: () => void;
    onstarted: (turn: DesignAssistTurn, label: string) => void;
    /** Called when a native 3D job produces a new or updated Design Hall artifact. */
    onartifactready?: (artifactId: string) => void;
  }
  let {
    kind,
    artifact,
    ctx,
    stage = 'generate',
    sourceVersionId = null,
    initialPrompt = '',
    onclose,
    onstarted,
    onartifactready,
  }: Props = $props();

  const LOCAL_ID = 'local-blender';
  const canAdmin = $derived(auth.can('design', 'admin'));

  let providers = $state<Media3dProvider[]>([]);
  let presets = $state<GameAssetPreset[]>([]);
  let loadingMeta = $state(true);
  let selectedId = $state<string>(OTTO_FORGE_ID);
  let manageOpen = $state(false);

  // Inputs
  let prompt = $state(untrack(() => initialPrompt));
  let title = $state('');
  let image = $state<File | null>(null);
  let imageUrl = $state<string | null>(null);
  let dragOver = $state(false);
  let quality = $state<Gen3dQuality>('standard');
  let presetId = $state<string>('stylized_character');
  let polyBudget = $state<number>(25_000);
  let outputFormat = $state<Media3dOutputFormat>('glb');
  let seed = $state<string>('');
  let updateCurrentArtifact = $state(untrack(() => artifact.format === 'glb' || artifact.format === 'gltf' || artifact.format === 'obj'));

  // Hunyuan3D-Omni multi-modal controls
  let controlType = $state<Media3dControlType>('none');
  let controlFile = $state<File | null>(null);
  let bboxW = $state(1.0);
  let bboxH = $state(1.0);
  let bboxD = $state(1.0);
  let useEma = $state(true);
  let flashvdm = $state(true);

  // TRELLIS.2 PBR controls
  let trellisRes = $state<number>(1024);
  let trellisTex = $state<number>(2048);
  let trellisPbr = $state(true);
  let trellisRemesh = $state(true);
  let steps = $state<number>(12);
  let guidance = $state<number>(7.5);

  // Job state
  let busy = $state(false);
  let error = $state<string | null>(null);
  let activeJob = $state<Media3dJob | null>(null);
  let poller: Poller | null = null;

  const isStageTransform = $derived(stage !== 'generate');
  const stageTitle = $derived(
    stage === 'gameready'
      ? 'Make Game-Ready (Retopo / Bake)'
      : stage === 'rig'
        ? 'Rig 3D Skeleton'
        : stage === 'animate'
          ? 'Animate 3D Model'
          : 'Generate 3D asset',
  );
  const visibleProviders = $derived(
    isStageTransform
      ? providers.filter((p) => p.capabilities?.supported_stages?.includes(stage))
      : providers,
  );
  const split = $derived(split3dProviders(visibleProviders));
  /** No picker when Forge is the only set-up provider: Advanced lists Forge first then, to come back to it. */
  const advancedPicks = $derived(split.shown.length > 1 ? split.advanced : [...split.shown, ...split.advanced]);
  const selectedBackend = $derived(visibleProviders.find((p) => p.id === selectedId) ?? null);
  const forgeMissing = $derived(!loadingMeta && selectedBackend?.id === OTTO_FORGE_ID && selectedBackend.enabled && !selectedBackend.ready);
  const isLocal = $derived(!isStageTransform && selectedId === LOCAL_ID);
  const selectedLicenseWarn = $derived(selectedBackend ? licenseWarningFor(selectedBackend) : null);
  const canSubmit = $derived.by(() => {
    if (busy || activeJob) return false;
    if (!ctx.canEdit) return false;
    if (isLocal) {
      if (ctx.busy || ctx.dirty) return false;
      return kind === 'text' ? prompt.trim().length > 2 : !!image;
    }
    if (!selectedBackend || !selectedBackend.ready || !selectedBackend.enabled) return false;
    if (isStageTransform) return true;
    return prompt.trim().length > 0 || !!image;
  });

  async function loadProvidersAndPresets(): Promise<void> {
    loadingMeta = true;
    try {
      const [ps, pr] = await Promise.all([mapi.list3dProviders(), mapi.list3dPresets()]);
      providers = ps;
      presets = pr;
      const pool = stage !== 'generate' ? ps.filter((p) => p.capabilities?.supported_stages?.includes(stage)) : ps;
      const picked = pickDefault3dProvider(pool);
      if (selectedId === OTTO_FORGE_ID || !pool.some((p) => p.id === selectedId)) selectedId = picked;
    } catch {
      /* keep local Otto + Blender fallback usable */
    } finally {
      loadingMeta = false;
    }
  }

  $effect(() => {
    untrack(() => void loadProvidersAndPresets());
  });

  onDestroy(() => {
    poller?.stop();
    if (imageUrl) URL.revokeObjectURL(imageUrl);
  });

  function applyPreset(p: GameAssetPreset): void {
    presetId = p.id;
    polyBudget = p.poly_budget;
    trellisRes = p.resolution;
    trellisTex = p.texture_size;
    outputFormat = p.output_format;
  }

  function acceptImageFile(f: File | null): void {
    if (f && !/^image\/(png|jpeg|webp)$/.test(f.type)) {
      error = 'Pick a PNG, JPEG or WebP image.';
      return;
    }
    if (f && f.size > 12 * 1024 * 1024) {
      error = 'That image is over 12 MB.';
      return;
    }
    error = null;
    if (imageUrl) URL.revokeObjectURL(imageUrl);
    image = f;
    imageUrl = f ? URL.createObjectURL(f) : null;
  }

  function onDrop(e: DragEvent): void {
    e.preventDefault();
    dragOver = false;
    acceptImageFile(e.dataTransfer?.files?.[0] ?? null);
  }

  function fileToB64(f: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const fr = new FileReader();
      fr.onerror = () => reject(fr.error);
      fr.onload = () => resolve(String(fr.result).split(',')[1] ?? '');
      fr.readAsDataURL(f);
    });
  }

  function startJobPoll(jobId: string): void {
    poller?.stop();
    poller = pollWhileVisible(
      async () => {
        try {
          const j = await mapi.get3dJob(jobId);
          activeJob = j;
          if (j.status === 'succeeded') {
            poller?.stop();
            poller = null;
            toasts.success('3D model generated', j.title);
            if (j.artifact_id) onartifactready?.(j.artifact_id);
            onclose();
          } else if (j.status === 'failed' || j.status === 'cancelled') {
            poller?.stop();
            poller = null;
            if (j.status === 'failed') error = j.error || '3D generation job failed.';
            else toasts.info('Generation canceled');
            activeJob = null;
          }
        } catch (e) {
          error = loadErrorText(e);
          if (e instanceof ApiError && (e.status === 401 || e.status === 403 || e.status === 404)) {
            poller?.stop();
            poller = null;
            activeJob = null;
          }
          return false;
        }
      },
      { ms: 1500, immediate: false },
    );
  }

  async function cancelActive(): Promise<void> {
    if (!activeJob) return;
    try {
      await mapi.cancel3dJob(activeJob.id);
      poller?.stop();
      poller = null;
      activeJob = null;
      toasts.info('Generation canceled');
    } catch (e) {
      toastError('Couldn’t cancel the job', e);
    }
  }

  async function generate(): Promise<void> {
    if (!canSubmit) return;
    busy = true;
    error = null;
    try {
      if (isLocal) {
        let refId: string | null = null;
        if (image) {
          const fmt = (image.type === 'image/jpeg' ? 'jpeg' : image.type === 'image/webp' ? 'webp' : 'png') as DesignArtifactFormat;
          const res = await createArtifact({
            workspace_id: artifact.workspace_id,
            project_id: artifact.project_id ?? undefined,
            studio: 'graphics',
            format: fmt,
            title: `${image.name.replace(/\.[a-z0-9]+$/i, '') || 'Reference'} (3D reference)`,
            content_b64: await fileToB64(image),
            tags: ['3d-reference'],
            message: `Reference image for ${artifact.title}`,
          });
          refId = res.artifact.id;
        }
        const turn = await startAssist(artifact.id, {
          prompt: localPrompt(kind, prompt, quality, refId ? 'R1' : null),
          mode: 'refine',
          references: refId ? [refId] : undefined,
        });
        onstarted(turn, prompt.trim() ? `Modeling “${prompt.trim().slice(0, 40)}”` : 'Modeling from the image');
        onclose();
        return;
      }

      const p = selectedBackend!;
      const params: Record<string, unknown> = { steps, guidance_scale: guidance };
      if (p.kind === 'hunyuan3d_omni') {
        params.control_type = controlType;
        params.use_ema = useEma;
        params.flashvdm = flashvdm;
        if (controlType === 'bbox') params.bbox = [bboxW, bboxH, bboxD];
        if (controlType !== 'none' && controlType !== 'bbox' && controlFile) {
          params.control_input_b64 = await fileToB64(controlFile);
          params.control_filename = controlFile.name;
        }
      } else if (p.kind === 'trellis_2') {
        params.resolution = trellisRes;
        params.texture_size = trellisTex;
        params.decimation_target = polyBudget;
        params.pbr = trellisPbr;
        params.remesh = trellisRemesh;
      }

      const parsedSeed = seed.trim() ? Number(seed.trim()) : null;
      const job = await mapi.start3dJob({
        workspace_id: artifact.workspace_id,
        project_id: artifact.project_id ?? null,
        stage,
        source_artifact_id: isStageTransform ? artifact.id : null,
        source_version_id: isStageTransform ? sourceVersionId : null,
        target_artifact_id: isStageTransform || updateCurrentArtifact ? artifact.id : null,
        title: title.trim() || (isStageTransform ? `${artifact.title} (${stage})` : prompt.trim().slice(0, 48) || `${artifact.title} (3D)`),
        provider_id: p.id,
        prompt: prompt.trim() || null,
        image_b64: image ? await fileToB64(image) : null,
        image_mime: image?.type || null,
        output_format: outputFormat,
        preset: presetId || null,
        poly_budget: polyBudget,
        seed: Number.isFinite(parsedSeed) ? parsedSeed : null,
        params,
        auto_save_artifact: true,
      });
      activeJob = job;
      if (job.status === 'succeeded') {
        toasts.success('3D model generated', job.title);
        if (job.artifact_id) onartifactready?.(job.artifact_id);
        onclose();
      } else {
        startJobPoll(job.id);
      }
    } catch (e) {
      error =
        e instanceof ApiError && e.status === 409
          ? 'Another generation is already running on this design.'
          : loadErrorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={stageTitle} width={620} {onclose}>
  <div class="g3" data-testid="gen3d-modal">
    {#if activeJob}
      <div class="g3-job" data-testid="gen3d-job-progress">
        <div class="g3-jhead">
          <strong>Generating “{activeJob.title}”</strong>
          <Badge tone="info" label={activeJob.stage || activeJob.status} />
        </div>
        <div class="g3-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={activeJob.progress_pct} aria-label="3D generation progress">
          <span class="g3-fill" style:inline-size="{Math.max(6, activeJob.progress_pct)}%"></span>
        </div>
        <p class="g3-dim">Progress: {activeJob.progress_pct}% · Automatically saves as a versioned 3D asset in Design Hall.</p>
      </div>
    {:else}
      {#if split.shown.length > 1}
        <Gen3dProviderList providers={split.shown} bind:selectedId legend="Choose 3D generator provider" />
      {/if}

      {#if split.shown.length === 1 && selectedId === OTTO_FORGE_ID && selectedBackend?.ready}
        <p class="g3-dim">Runs on this Mac with Otto Forge.</p>
      {/if}

      {#if forgeMissing}
        <div class="g3-setup" data-testid="gen3d-forge-missing" use:announceShown={'Otto Forge isn’t installed'}>
          <strong>Otto Forge isn’t installed</strong>
          <p class="g3-dim">It generates game-ready models on this Mac. Run this in its folder (~/otto-forge, or put its <code>bin/</code> on PATH), then open Generate again. It downloads about 24 GB.</p>
          <code class="g3-cmd">./install.sh --with-models</code>
        </div>
      {:else}
      {#if selectedLicenseWarn}
        <p class="g3-warn" data-testid="gen3d-license-notice" use:announceShown={selectedLicenseWarn}>
          <Icon name="warning" size={12} /> {selectedLicenseWarn}
        </p>
      {/if}

      {#if !isLocal && presets.length > 0}
        <div class="g3-field">
          <span class="k">Game-asset preset</span>
          <div class="g3-presets" role="group" aria-label="Game asset presets">
            {#each presets as pr (pr.id)}
              <button
                type="button"
                class="btn small"
                class:on={presetId === pr.id}
                aria-pressed={presetId === pr.id}
                title={pr.description}
                onclick={() => applyPreset(pr)}
              >{pr.label}</button>
            {/each}
          </div>
        </div>
      {/if}

      <div class="g3-field">
        <label class="k" for="g3-prompt">Prompt &amp; design brief</label>
        <textarea
          id="g3-prompt"
          dir="auto"
          rows="3"
          bind:value={prompt}
          maxlength="2400"
          placeholder="Stylized medieval lantern with weathered brass frame and glowing runes…"
          data-testid="gen3d-prompt"
        ></textarea>
      </div>

      <div
        class="g3-drop"
        class:over={dragOver}
        role="group"
        aria-label="Reference image upload"
        ondragover={(e) => { e.preventDefault(); dragOver = true; }}
        ondragleave={() => (dragOver = false)}
        ondrop={onDrop}
      >
        {#if imageUrl}
          <img class="g3-thumb" src={imageUrl} alt="Reference preview" />
        {/if}
        <div class="g3-dbody">
          <span class="k">Reference image {kind === 'image' ? '(required)' : '(optional)'}</span>
          <div class="g3-dacts">
            <label class="btn small">
              <Icon name="image" size={12} /> {image ? 'Replace image…' : 'Choose image…'}
              <input class="hidden" type="file" accept="image/png,image/jpeg,image/webp" onchange={(e) => acceptImageFile((e.currentTarget as HTMLInputElement).files?.[0] ?? null)} />
            </label>
            {#if image}
              <button type="button" class="btn small ghost" onclick={() => acceptImageFile(null)}>Clear</button>
            {/if}
          </div>
        </div>
      </div>

      {#if selectedBackend?.kind === 'hunyuan3d_omni'}
        <div class="g3-panel">
          <span class="k">Hunyuan3D-Omni multi-modal conditioning</span>
          <div class="g3-grid2">
            <div class="g3-field">
              <label class="k" for="g3-ctrl">Control modality</label>
              <select id="g3-ctrl" class="input" bind:value={controlType}>
                <option value="none">None (image / prompt only)</option>
                <option value="bbox">3D Bounding Box (W × H × D)</option>
                <option value="point">Point Cloud (.ply / .obj)</option>
                <option value="voxel">Voxel Grid (.ply / .npy)</option>
                <option value="pose">Skeleton Pose (.json)</option>
              </select>
            </div>
            <div class="g3-field">
              <label class="k" for="g3-seed">Seed (optional)</label>
              <input id="g3-seed" dir="ltr" class="input" bind:value={seed} placeholder="Random" />
            </div>
          </div>
          {#if controlType === 'bbox'}
            <div class="g3-grid3">
              <label class="g3-num"><span>W</span><input type="number" step="0.1" min="0.1" class="input" bind:value={bboxW} /></label>
              <label class="g3-num"><span>H</span><input type="number" step="0.1" min="0.1" class="input" bind:value={bboxH} /></label>
              <label class="g3-num"><span>D</span><input type="number" step="0.1" min="0.1" class="input" bind:value={bboxD} /></label>
            </div>
          {:else if controlType !== 'none'}
            <label class="btn small">
              <Icon name="file" size={12} /> {controlFile ? controlFile.name : `Attach ${controlType} file…`}
              <input class="hidden" type="file" onchange={(e) => (controlFile = (e.currentTarget as HTMLInputElement).files?.[0] ?? null)} />
            </label>
          {/if}
          <div class="g3-checks">
            <label class="g3-check"><input type="checkbox" bind:checked={useEma} /><span>Use EMA weights</span></label>
            <label class="g3-check"><input type="checkbox" bind:checked={flashvdm} /><span>FlashVDM fast decode</span></label>
          </div>
        </div>
      {:else if selectedBackend?.kind === 'trellis_2'}
        <div class="g3-panel">
          <span class="k">TRELLIS.2 O-Voxel PBR parameters</span>
          <div class="g3-grid3">
            <div class="g3-field">
              <label class="k" for="g3-res">Resolution</label>
              <select id="g3-res" class="input" bind:value={trellisRes}>
                <option value={512}>512 (Fast)</option>
                <option value={1024}>1024 (Standard)</option>
                <option value={1536}>1536 (Hero)</option>
              </select>
            </div>
            <div class="g3-field">
              <label class="k" for="g3-tex">PBR Texture</label>
              <select id="g3-tex" class="input" bind:value={trellisTex}>
                <option value={1024}>1024 px</option>
                <option value={2048}>2048 px</option>
                <option value={4096}>4096 px</option>
              </select>
            </div>
            <div class="g3-field">
              <label class="k" for="g3-poly">Target tris</label>
              <input id="g3-poly" type="number" min="500" max="500000" step="500" class="input" bind:value={polyBudget} />
            </div>
          </div>
          <div class="g3-checks">
            <label class="g3-check"><input type="checkbox" bind:checked={trellisPbr} /><span>PBR materials</span></label>
            <label class="g3-check"><input type="checkbox" bind:checked={trellisRemesh} /><span>Remesh topology</span></label>
          </div>
        </div>
      {:else if isLocal}
        <div class="g3-field">
          <label class="k" for="g3-qual">Quality</label>
          <select id="g3-qual" class="input" bind:value={quality}>
            <option value="draft">Draft — quick blockout</option>
            <option value="standard">Standard — cleaner model</option>
          </select>
        </div>
      {/if}

      {#if !isLocal}
        <div class="g3-grid2">
          <div class="g3-field">
            <label class="k" for="g3-title">Asset title</label>
            <input id="g3-title" dir="auto" class="input" bind:value={title} placeholder={artifact.title} />
          </div>
          <label class="g3-check g3-align">
            <input type="checkbox" bind:checked={updateCurrentArtifact} />
            <span>Save as new version on this design</span>
          </label>
        </div>
      {/if}

      {#if loadingMeta}<p class="g3-dim">Checking configured 3D providers…</p>{/if}
      {#if !isLocal && selectedBackend && !selectedBackend.ready}
        <p class="g3-warn" use:announceShown={selectedBackend.status_detail}>
          <Icon name="warning" size={12} /> {selectedBackend.status_detail} — Advanced › “Manage providers…” configures an endpoint or local CLI.
        </p>
      {/if}
      {/if}

      <details class="g3-adv" data-testid="gen3d-advanced">
        <summary>Advanced</summary>
        <Gen3dProviderList providers={advancedPicks} bind:selectedId legend="Other 3D generators" localId={isStageTransform ? null : LOCAL_ID} />
        <button type="button" class="btn small ghost g3-manage" onclick={() => (manageOpen = true)} data-testid="gen3d-manage-providers">
          <Icon name="gear" size={12} /> Manage providers…
        </button>
      </details>
    {/if}

    {#if error}
      <p class="g3-err" use:announceShown={{ text: error, assertive: true }}>
        <Icon name="warning" size={12} /> {error}
      </p>
    {/if}
  </div>

  {#snippet footer()}
    {#if activeJob}
      <button type="button" class="btn" onclick={() => void cancelActive()}>Cancel job</button>
    {:else if forgeMissing}
      <button type="button" class="btn" onclick={onclose}>Close</button>
    {:else}
      <button type="button" class="btn" onclick={onclose}>Cancel</button>
      <button type="button" class="btn primary" disabled={!canSubmit} onclick={() => void generate()} data-testid="gen3d-go">
        {busy ? 'Starting…' : stage === 'gameready' ? 'Make Game-Ready' : stage === 'rig' ? 'Rig model' : stage === 'animate' ? 'Animate model' : 'Generate 3D'}
      </button>
    {/if}
  {/snippet}
</Modal>

{#if manageOpen}
  <Provider3DManagerModal
    {canAdmin}
    onclose={() => (manageOpen = false)}
    onchanged={() => void loadProvidersAndPresets()}
  />
{/if}

<style>
  .g3 {
    display: flex;
    flex-direction: column;
    gap: 12px;
    /* One scroll area: the dialog body scrolls, not this column too. x clipped: touch hit
       areas poke past the edge, never a sideways scrollbar. */
    overflow-x: clip;
  }
  .g3-jhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .g3-setup {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .g3-cmd {
    align-self: flex-start;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    background: var(--bg);
    font-size: var(--fs-s);
    user-select: all;
  }
  .g3-adv summary {
    cursor: pointer;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
  }
  .g3-adv[open] summary {
    margin-block-end: 8px;
  }
  .g3-manage {
    margin-block-start: 8px;
  }
  .g3-dim {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .g3-presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .g3-presets .on {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
  }
  .g3-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .g3-field textarea {
    font: inherit;
    font-size: var(--fs-m);
    padding: 6px 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    resize: vertical;
  }
  .g3-drop {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-m);
    background: var(--bg);
  }
  .g3-drop.over {
    border-color: var(--accent-line-strong);
    background: var(--accent-faint);
  }
  .g3-thumb {
    inline-size: 56px;
    block-size: 56px;
    object-fit: cover;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
  }
  .g3-dbody {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .g3-dacts,
  .g3-checks {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
  }
  .g3-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .g3-grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .g3-grid3 {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  @media (max-width: 640px) {
    .g3-grid2,
    .g3-grid3 {
      grid-template-columns: 1fr;
    }
  }
  .g3-num {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .g3-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .g3-align {
    align-self: end;
    padding-block-end: 6px;
  }
  .g3-job {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
  }
  .g3-bar {
    block-size: 8px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .g3-fill {
    display: block;
    block-size: 100%;
    background: var(--accent-solid);
  }
  .g3-warn {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--warning);
  }
  .g3-err {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .hidden {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    opacity: 0;
    pointer-events: none;
  }
  @media (pointer: coarse) {
    .g3 :global(.input),
    .g3 textarea,
    .g3 select {
      min-block-size: 40px;
      font-size: 16px; /* ui-guards: allow */
    }
    .g3 :global(.btn) {
      min-block-size: 40px;
    }
  }
</style>
