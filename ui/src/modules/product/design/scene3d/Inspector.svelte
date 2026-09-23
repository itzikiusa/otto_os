<script lang="ts">
  // Scene inspector — the game-studio right panel. With nothing selected it shows
  // the SCENE (background, grid) and CAMERA panels; an object gets Transform +
  // Material (+ Text / Model) + Notes; a light gets its light panel; a group gets
  // its members. Every field patches the JSON doc through `ops.ts` and emits
  // `onchange(newDoc)` live (no debounce here — the arena owns the autosave).
  //
  // v2: a physical-material preset grid, the brand kit's colour swatches (a
  // click stores `token:color.<name>`, so the scene follows the kit), the
  // physical sliders (clearcoat / transmission / sheen), a box corner radius,
  // and — when a non-default STATE is showing — transform edits go to that
  // state's overrides (`editState`), exactly like the viewport gizmo.
  import NumberDrag from './NumberDrag.svelte';
  import { LIGHT_TYPES, type Scene3dDoc, type Scene3dMaterial, type Vec3 } from './types';
  import {
    findNode,
    nodeLabel,
    parentGroup,
    patchLight,
    rename,
    setCamera,
    setMaterial,
    setMaterialPreset,
    setNotes,
    setRadius,
    setScene,
    setStateOverride,
    setText,
    setTransform,
    setVisible,
    summarize,
  } from './ops';
  import { MATERIAL_PRESETS, resolveMaterial } from './presets';
  import { colorLabel, isTokenRef, resolveColor, type BrandSwatch } from './tokens';
  import { findState } from './states';

  interface Props {
    doc: Scene3dDoc;
    selectedId?: string | null;
    onchange: (doc: Scene3dDoc) => void;
    readonly?: boolean;
    /** v2: the brand kit's colours (Design Hall passes the scene's kit). */
    swatches?: BrandSwatch[];
    /** v2: the kit's display name for the Material panel ("Acme Brand Kit v4"). */
    brandName?: string | null;
    /** v2: the brand document tokens resolve against (for the resolved hex readout). */
    brand?: unknown;
    /** v2: transform edits of the selected object go to this state's overrides. */
    editState?: string | null;
  }
  let {
    doc,
    selectedId = $bindable<string | null>(null),
    onchange,
    readonly = false,
    swatches = [],
    brandName = null,
    brand = null,
    editState = null,
  }: Props = $props();

  const sel = $derived(findNode(doc, selectedId));
  const obj = $derived(sel?.kind === 'object' ? sel.node : null);
  const light = $derived(sel?.kind === 'light' ? sel.node : null);
  const group = $derived(sel?.kind === 'group' ? sel.node : null);
  const parent = $derived(selectedId ? parentGroup(doc, selectedId) : null);
  const mat = $derived<Scene3dMaterial>(obj?.material ?? {});
  /** Preset defaults folded in — what the sliders show when a field isn't set. */
  const resolved = $derived(resolveMaterial(obj?.material, (r, fb) => resolveColor(r, brand, fb)));
  const stateEdit = $derived(obj && editState ? findState(doc, editState) : null);
  /** The transform the rows show: the state's override over the base. */
  const shownTransform = $derived.by(() => {
    if (!obj) return null;
    const ov = stateEdit?.overrides?.[obj.id];
    return { position: ov?.position ?? obj.position, rotation: ov?.rotation ?? obj.rotation, scale: ov?.scale ?? obj.scale };
  });
  function emitTransform(patch: { position?: Vec3; rotation?: Vec3; scale?: Vec3 }): void {
    if (!obj) return;
    emit(stateEdit ? setStateOverride(doc, stateEdit.id, obj.id, patch) : setTransform(doc, obj.id, patch));
  }

  // Collapsible panels remember their state for the session.
  let open = $state<Record<string, boolean>>({ transform: true, material: true, light: true, scene: true, camera: true, notes: false, model: true, text: true, group: true });
  function toggle(k: string): void {
    open = { ...open, [k]: !open[k] };
  }

  function emit(next: Scene3dDoc): void {
    if (readonly || next === doc) return;
    onchange(next);
  }
  function vec(v: Vec3, i: number, n: number): Vec3 {
    const out: Vec3 = [v[0], v[1], v[2]];
    out[i] = n;
    return out;
  }
  const axis = ['X', 'Y', 'Z'];

  function onMat<K extends keyof Scene3dMaterial>(k: K, v: Scene3dMaterial[K] | undefined): void {
    if (!obj) return;
    emit(setMaterial(doc, obj.id, { [k]: v } as Partial<Scene3dMaterial>));
  }
  function inputVal(e: Event): string {
    return (e.currentTarget as HTMLInputElement).value;
  }
  function inputChecked(e: Event): boolean {
    return (e.currentTarget as HTMLInputElement).checked;
  }
</script>

{#snippet panelHead(key: string, title: string, meta?: string)}
  <button class="s3d-ph" onclick={() => toggle(key)} aria-expanded={open[key] !== false}>
    <span class="s3d-ph-chev" class:closed={open[key] === false}>▾</span>
    <span class="s3d-ph-title">{title}</span>
    {#if meta}<span class="s3d-ph-meta">{meta}</span>{/if}
  </button>
{/snippet}

{#snippet vec3Row(label: string, v: Vec3, step: number, digits: number, on: (nv: Vec3) => void, unit?: string)}
  <div class="s3d-field">
    <span class="s3d-flabel">{label}</span>
    <div class="s3d-vec">
      {#each [0, 1, 2] as i (i)}
        <NumberDrag label={axis[i]} value={v[i]} {step} {digits} {unit} disabled={readonly} onchange={(n) => on(vec(v, i, n))} />
      {/each}
    </div>
  </div>
{/snippet}

{#snippet rangeRow(label: string, value: number, min: number, max: number, step: number, on: (n: number | undefined) => void, resettable = true)}
  <div class="s3d-field">
    <span class="s3d-flabel">{label}</span>
    <input class="s3d-range" type="range" {min} {max} {step} {value} disabled={readonly} aria-label={label} oninput={(e) => on(Number(inputVal(e)))} />
    <NumberDrag label="" value={value} step={step} {min} {max} digits={2} disabled={readonly} onchange={(n) => on(n)} />
    {#if resettable}
      <button class="s3d-reset" title="Reset to default" aria-label="Reset {label}" disabled={readonly} onclick={() => on(undefined)}>×</button>
    {/if}
  </div>
{/snippet}

{#snippet colorRow(label: string, value: string | undefined, fallback: string, on: (c: string | undefined) => void)}
  <div class="s3d-field">
    <span class="s3d-flabel">{label}</span>
    <input class="s3d-color" type="color" value={value ?? fallback} disabled={readonly} aria-label={label} oninput={(e) => on(inputVal(e))} />
    <input class="s3d-hex" type="text" value={value ?? ''} placeholder={fallback} maxlength="7" spellcheck="false" disabled={readonly} aria-label="{label} hex" onchange={(e) => {
      const t = inputVal(e).trim().toLowerCase();
      if (!t) on(undefined);
      else if (/^#[0-9a-f]{6}$/.test(t)) on(t);
      else (e.currentTarget as HTMLInputElement).value = value ?? '';
    }} />
    <button class="s3d-reset" title="Reset to default" aria-label="Reset {label}" disabled={readonly} onclick={() => on(undefined)}>×</button>
  </div>
{/snippet}

<div class="s3d-inspector">
  {#if !sel}
    <div class="s3d-head">
      <div class="s3d-head-name">Scene</div>
      <div class="s3d-head-sub">{summarize(doc)} · select something to edit it</div>
    </div>

    <section class="s3d-panel">
      {@render panelHead('scene', 'Scene')}
      {#if open.scene !== false}
        <div class="s3d-panel-body">
          {@render colorRow('Background', doc.background, '#0f172a', (c) => emit(setScene(doc, { background: c ?? null })))}
          <label class="s3d-field s3d-check">
            <span class="s3d-flabel">Grid</span>
            <input type="checkbox" checked={doc.grid ?? true} disabled={readonly} onchange={(e) => emit(setScene(doc, { grid: inputChecked(e) }))} />
            <span class="s3d-hint">Editor only — Play hides it</span>
          </label>
        </div>
      {/if}
    </section>

    <section class="s3d-panel">
      {@render panelHead('camera', 'Camera', 'used by Play, the agent and Blender')}
      {#if open.camera !== false}
        <div class="s3d-panel-body">
          {@render vec3Row('Position', doc.camera.position, 0.05, 2, (v) => emit(setCamera(doc, { position: v })), 'm')}
          {@render vec3Row('Target', doc.camera.target, 0.05, 2, (v) => emit(setCamera(doc, { target: v })), 'm')}
          {@render rangeRow('FOV', doc.camera.fov, 10, 120, 1, (n) => emit(setCamera(doc, { fov: n ?? 50 })), false)}
          <div class="s3d-field">
            <span class="s3d-flabel">Clip</span>
            <div class="s3d-vec">
              <NumberDrag label="near" value={doc.camera.near ?? 0.1} step={0.01} min={0.001} digits={3} disabled={readonly} onchange={(n) => emit(setCamera(doc, { near: n }))} />
              <NumberDrag label="far" value={doc.camera.far ?? 500} step={1} min={1} digits={0} disabled={readonly} onchange={(n) => emit(setCamera(doc, { far: n }))} />
            </div>
          </div>
          <div class="s3d-hint">Tip: frame your view in the viewport, then <em>View → camera</em> to store it here.</div>
        </div>
      {/if}
    </section>
  {:else}
    <div class="s3d-head">
      <input
        class="s3d-head-name-input"
        value={nodeLabel(sel)}
        disabled={readonly}
        aria-label="Name"
        onchange={(e) => emit(rename(doc, sel.node.id, inputVal(e)))}
      />
      <div class="s3d-head-sub">
        <span class="s3d-kind">{sel.kind === 'object' ? sel.node.type : sel.kind === 'light' ? `${sel.node.type} light` : 'group'}</span>
        <code class="s3d-id" title="id (what the agent references)">{sel.node.id}</code>
        {#if parent}<span>· in <strong>{parent.name}</strong></span>{/if}
      </div>
      <label class="s3d-check s3d-vis">
        <input type="checkbox" checked={sel.node.visible !== false} disabled={readonly} onchange={(e) => emit(setVisible(doc, sel.node.id, inputChecked(e)))} />
        <span>Visible</span>
      </label>
    </div>

    {#if obj}
      <section class="s3d-panel">
        {@render panelHead('transform', 'Transform')}
        {#if open.transform !== false}
          <div class="s3d-panel-body">
            {#if stateEdit}
              <div class="s3d-hint s3d-state-hint" data-testid="s3d-state-edit">Editing the <strong>{stateEdit.name ?? stateEdit.id}</strong> state — changes here tween from the base pose.</div>
            {/if}
            {@render vec3Row('Position', shownTransform!.position, 0.01, 3, (v) => emitTransform({ position: v }), 'm')}
            {@render vec3Row('Rotation', shownTransform!.rotation, 0.5, 1, (v) => emitTransform({ rotation: v }), '°')}
            {@render vec3Row('Scale', shownTransform!.scale, 0.01, 3, (v) => emitTransform({ scale: v }))}
            {#if obj.type === 'box'}
              {@render rangeRow('Corners', obj.radius ?? 0, 0, 0.5, 0.005, (n) => emit(setRadius(doc, obj.id, n ?? 0)))}
            {/if}
            <div class="s3d-row-btns">
              <button class="s3d-mini" disabled={readonly} onclick={() => emit(setTransform(doc, obj.id, { position: [0, obj.type === 'plane' ? 0 : 0.5, 0] }))}>Reset position</button>
              <button class="s3d-mini" disabled={readonly} onclick={() => emit(setTransform(doc, obj.id, { rotation: obj.type === 'plane' ? [-90, 0, 0] : [0, 0, 0] }))}>Reset rotation</button>
              <button class="s3d-mini" disabled={readonly} onclick={() => emit(setTransform(doc, obj.id, { scale: [1, 1, 1] }))}>Reset scale</button>
              <button class="s3d-mini" disabled={readonly} title="Rest the unit on the floor (y = half height × scale)" onclick={() => emit(setTransform(doc, obj.id, { position: [obj.position[0], obj.type === 'plane' ? 0 : 0.5 * obj.scale[1], obj.position[2]] }))}>Snap to floor</button>
            </div>
          </div>
        {/if}
      </section>

      {#if obj.type === 'gltf'}
        <section class="s3d-panel">
          {@render panelHead('model', 'Model')}
          {#if open.model !== false}
            <div class="s3d-panel-body">
              <div class="s3d-field">
                <span class="s3d-flabel">Attachment</span>
                <code class="s3d-id s3d-grow" title="attachment_id — loaded through the authed attachment route">{obj.attachment_id}</code>
              </div>
              <div class="s3d-hint">Materials come from the GLB itself. Re-import to replace the model.</div>
            </div>
          {/if}
        </section>
      {:else}
        {#if obj.type === 'text'}
          <section class="s3d-panel">
            {@render panelHead('text', 'Text')}
            {#if open.text !== false}
              <div class="s3d-panel-body">
                <input class="s3d-text" value={obj.text ?? obj.name} maxlength="500" disabled={readonly} aria-label="Text" oninput={(e) => emit(setText(doc, obj.id, inputVal(e)))} />
                <div class="s3d-hint">Drawn on a 2 × 0.5 m quad; scale it like any object. Colour comes from the material.</div>
              </div>
            {/if}
          </section>
        {/if}
        <section class="s3d-panel">
          {@render panelHead('material', 'Material', brandName ?? undefined)}
          {#if open.material !== false}
            <div class="s3d-panel-body">
              <div class="s3d-presets" role="radiogroup" aria-label="Material preset">
                {#each MATERIAL_PRESETS as p (p.id)}
                  {@const tint = p.id === 'brushed-metal' || p.id === 'frosted-glass' ? p.swatch : resolved.color}
                  <button
                    role="radio"
                    aria-checked={mat.preset === p.id}
                    class="s3d-preset {p.id}"
                    class:on={mat.preset === p.id}
                    title={p.hint}
                    disabled={readonly}
                    data-testid="s3d-preset-{p.id}"
                    onclick={() => emit(setMaterialPreset(doc, obj.id, mat.preset === p.id ? null : p.id))}
                  >
                    <span class="s3d-ball" style:--ball={tint}></span>
                    <span class="s3d-plabel">{p.label}</span>
                  </button>
                {/each}
              </div>
              {#if swatches.length}
                <div class="s3d-field s3d-swatch-row">
                  <span class="s3d-flabel">Brand</span>
                  <div class="s3d-swatches" role="radiogroup" aria-label="Brand colours">
                    {#each swatches.slice(0, 12) as sw (sw.ref)}
                      <button
                        role="radio"
                        aria-checked={mat.color === sw.ref}
                        class="s3d-swatch"
                        class:on={mat.color === sw.ref}
                        style:--sw={sw.value}
                        title="{colorLabel(sw.ref)} · {sw.value}"
                        aria-label="{colorLabel(sw.ref)} {sw.value}"
                        disabled={readonly}
                        onclick={() => onMat('color', sw.ref)}
                      ></button>
                    {/each}
                  </div>
                </div>
              {/if}
              {#if isTokenRef(mat.color)}
                <div class="s3d-field">
                  <span class="s3d-flabel">Color</span>
                  <span class="s3d-token" title="Follows the brand kit: changing the kit recolours this object">
                    <span class="s3d-dot" style:--sw={resolved.color}></span>{colorLabel(mat.color)} <code>{resolved.color}</code>
                  </span>
                  <button class="s3d-reset" title="Detach from the brand kit (keep {resolved.color})" aria-label="Detach colour from the brand kit" disabled={readonly} onclick={() => onMat('color', resolved.color)}>×</button>
                </div>
              {:else}
                {@render colorRow('Color', mat.color, resolved.color, (c) => onMat('color', c))}
              {/if}
              {@render rangeRow('Roughness', mat.roughness ?? resolved.roughness, 0, 1, 0.01, (n) => onMat('roughness', n))}
              {@render rangeRow('Metalness', mat.metalness ?? resolved.metalness, 0, 1, 0.01, (n) => onMat('metalness', n))}
              {@render rangeRow('Clearcoat', mat.clearcoat ?? resolved.clearcoat, 0, 1, 0.01, (n) => onMat('clearcoat', n))}
              {@render rangeRow('Glass', mat.transmission ?? resolved.transmission, 0, 1, 0.01, (n) => onMat('transmission', n))}
              {@render rangeRow('Sheen', mat.sheen ?? resolved.sheen, 0, 1, 0.01, (n) => onMat('sheen', n))}
              {@render rangeRow('Opacity', mat.opacity ?? 1, 0, 1, 0.01, (n) => onMat('opacity', n))}
              {@render colorRow('Emissive', mat.emissive, '#000000', (c) => onMat('emissive', c))}
              <label class="s3d-field s3d-check">
                <span class="s3d-flabel">Wireframe</span>
                <input type="checkbox" checked={mat.wireframe ?? false} disabled={readonly} onchange={(e) => onMat('wireframe', inputChecked(e) || undefined)} />
              </label>
            </div>
          {/if}
        </section>
      {/if}
    {/if}

    {#if light}
      <section class="s3d-panel">
        {@render panelHead('light', 'Light')}
        {#if open.light !== false}
          <div class="s3d-panel-body">
            <div class="s3d-field">
              <span class="s3d-flabel">Type</span>
              <select class="s3d-select" value={light.type} disabled aria-label="Light type" title="Delete and add a new light to change its type">
                {#each LIGHT_TYPES as t (t)}<option value={t}>{t}</option>{/each}
              </select>
            </div>
            {@render rangeRow('Intensity', light.intensity ?? 1, 0, light.type === 'ambient' || light.type === 'hemisphere' ? 3 : 10, 0.05, (n) => emit(patchLight(doc, light.id, { intensity: n })))}
            {@render colorRow('Color', light.color, '#ffffff', (c) => emit(patchLight(doc, light.id, { color: c })))}
            {#if light.type === 'hemisphere'}
              {@render colorRow('Ground', light.ground_color, '#334155', (c) => emit(patchLight(doc, light.id, { ground_color: c })))}
            {/if}
            {#if light.type !== 'ambient' && light.type !== 'hemisphere'}
              {@render vec3Row('Position', light.position ?? [5, 10, 5], 0.05, 2, (v) => emit(setTransform(doc, light.id, { position: v })), 'm')}
            {/if}
            {#if light.type === 'directional' || light.type === 'spot'}
              {@render vec3Row('Target', light.target ?? [0, 0, 0], 0.05, 2, (v) => emit(patchLight(doc, light.id, { target: v })), 'm')}
            {/if}
            {#if light.type === 'spot'}
              {@render rangeRow('Angle', light.angle ?? 30, 1, 90, 1, (n) => emit(patchLight(doc, light.id, { angle: n })))}
            {/if}
            {#if light.type === 'spot' || light.type === 'point'}
              {@render rangeRow('Distance', light.distance ?? 0, 0, 100, 0.5, (n) => emit(patchLight(doc, light.id, { distance: n })))}
            {/if}
            {#if light.type !== 'ambient' && light.type !== 'hemisphere'}
              <label class="s3d-field s3d-check">
                <span class="s3d-flabel">Shadows</span>
                <input type="checkbox" checked={light.shadow ?? false} disabled={readonly} onchange={(e) => emit(patchLight(doc, light.id, { shadow: inputChecked(e) || undefined }))} />
              </label>
            {/if}
          </div>
        {/if}
      </section>
    {/if}

    {#if group}
      <section class="s3d-panel">
        {@render panelHead('group', 'Group', `${group.children.length} member${group.children.length === 1 ? '' : 's'}`)}
        {#if open.group !== false}
          <div class="s3d-panel-body">
            {#if group.children.length}
              <ul class="s3d-members">
                {#each group.children as cid (cid)}
                  {@const c = findNode(doc, cid)}
                  {#if c}
                    <li><button class="s3d-link" onclick={() => (selectedId = cid)}>{nodeLabel(c)}</button> <span class="s3d-hint">{c.kind === 'object' ? c.node.type : c.kind}</span></li>
                  {/if}
                {/each}
              </ul>
            {:else}
              <div class="s3d-hint">Empty group. Right-click a node in the hierarchy → <em>Move to {group.name}</em>.</div>
            {/if}
            <div class="s3d-hint">Groups organise the hierarchy; they carry no transform of their own.</div>
          </div>
        {/if}
      </section>
    {/if}

    <section class="s3d-panel">
      {@render panelHead('notes', 'Notes', sel.node.notes ? `${sel.node.notes.length}` : undefined)}
      {#if open.notes !== false}
        <div class="s3d-panel-body">
          <textarea
            class="s3d-notes"
            rows="4"
            maxlength="4000"
            placeholder="Design intent, review comments, TODOs for the agent…"
            value={sel.node.notes ?? ''}
            disabled={readonly}
            aria-label="Notes"
            oninput={(e) => emit(setNotes(doc, sel.node.id, (e.currentTarget as HTMLTextAreaElement).value))}
          ></textarea>
          <div class="s3d-hint">Saved in the scene file, so the assistant reads them too.</div>
        </div>
      {/if}
    </section>
  {/if}
</div>

<style>
  .s3d-inspector {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    overflow-y: auto;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .s3d-head {
    padding: 10px 10px 8px;
    border-bottom: 1px solid var(--border);
    display: grid;
    gap: 4px;
  }
  .s3d-head-name {
    font-weight: 600;
    font-size: var(--fs-m);
  }
  .s3d-head-name-input {
    font-weight: 600;
    font-size: var(--fs-m);
    padding: 3px 6px;
    border: 1px solid transparent;
    border-radius: var(--radius-s, 5px);
    background: transparent;
    color: var(--text);
    width: 100%;
  }
  .s3d-head-name-input:hover:not(:disabled),
  .s3d-head-name-input:focus {
    border-color: var(--border);
    background: var(--bg);
    outline: none;
  }
  .s3d-head-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .s3d-kind {
    text-transform: capitalize;
  }
  .s3d-id {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--surface-2);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .s3d-grow {
    flex: 1 1 auto;
    min-width: 0;
  }
  .s3d-vis {
    margin-top: 2px;
  }
  .s3d-panel {
    border-bottom: 1px solid var(--border);
  }
  .s3d-ph {
    appearance: none;
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 10px;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    font: 600 var(--fs-xs) var(--font-ui);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: pointer;
    text-align: start;
  }
  .s3d-ph:hover {
    color: var(--text);
  }
  .s3d-ph-chev {
    display: inline-block;
    transition: transform 0.12s ease;
    width: 10px;
  }
  .s3d-ph-chev.closed {
    transform: rotate(-90deg);
  }
  .s3d-ph-meta {
    margin-left: auto;
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
    opacity: 0.8;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .s3d-panel-body {
    display: grid;
    gap: 6px;
    padding: 2px 10px 10px;
  }
  .s3d-field {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .s3d-flabel {
    flex: 0 0 62px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .s3d-vec {
    display: flex;
    gap: 4px;
    flex: 1 1 auto;
    min-width: 0;
  }
  .s3d-range {
    flex: 1 1 auto;
    min-width: 40px;
    accent-color: var(--accent);
  }
  .s3d-field :global(.nd) {
    flex: 0 0 64px;
  }
  .s3d-vec :global(.nd) {
    flex: 1 1 0;
  }
  .s3d-color {
    width: 28px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 5px);
    background: transparent;
    cursor: pointer;
  }
  .s3d-hex,
  .s3d-text,
  .s3d-select {
    flex: 1 1 auto;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 5px);
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .s3d-hex {
    font-family: var(--font-mono);
    max-width: 84px;
  }
  .s3d-reset {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    width: 18px;
    height: 22px;
    cursor: pointer;
    border-radius: 4px;
    font-size: var(--fs-m);
    line-height: 1;
    flex-shrink: 0;
  }
  .s3d-reset:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }
  .s3d-check {
    cursor: pointer;
  }
  .s3d-check input {
    accent-color: var(--accent);
    margin: 0;
  }
  .s3d-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.4;
  }
  .s3d-row-btns {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .s3d-mini {
    appearance: none;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    border-radius: var(--radius-s, 5px);
    font: var(--fs-xs) var(--font-ui);
    padding: 3px 7px;
    cursor: pointer;
  }
  .s3d-mini:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .s3d-mini:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .s3d-notes {
    width: 100%;
    resize: vertical;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 5px);
    background: var(--bg);
    color: var(--text);
    font: var(--fs-s)/1.45 var(--font-ui);
    box-sizing: border-box;
  }
  .s3d-members {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
  }
  .s3d-link {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    padding: 0;
    cursor: pointer;
    font-size: var(--fs-s);
  }
  .s3d-link:hover {
    text-decoration: underline;
  }
  /* v2 material presets: a 5-up grid of lit "balls" (the swatch colour in a radial highlight). */
  .s3d-presets {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 6px;
  }
  .s3d-preset {
    appearance: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    padding: 8px 2px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text-dim);
    cursor: pointer;
    min-width: 0;
  }
  .s3d-preset:hover:not(:disabled) {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .s3d-preset.on {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .s3d-preset:focus-visible,
  .s3d-swatch:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .s3d-ball {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: radial-gradient(circle at 35% 30%, color-mix(in srgb, white 85%, var(--ball)) 0%, var(--ball) 45%, color-mix(in srgb, black 55%, var(--ball)) 100%);
    box-shadow: 0 1px 2px color-mix(in srgb, black 25%, transparent);
  }
  .s3d-preset.brushed-metal .s3d-ball {
    background: radial-gradient(circle at 35% 30%, white 0%, var(--ball) 40%, color-mix(in srgb, black 50%, var(--ball)) 100%);
  }
  .s3d-preset.frosted-glass .s3d-ball {
    background: radial-gradient(circle at 35% 30%, white 0%, color-mix(in srgb, var(--ball) 70%, transparent) 55%, color-mix(in srgb, var(--ball) 40%, transparent) 100%);
    border: 1px solid var(--border);
  }
  .s3d-preset.matte-paper .s3d-ball {
    background: radial-gradient(circle at 40% 35%, color-mix(in srgb, white 40%, var(--ball)) 0%, var(--ball) 70%, color-mix(in srgb, black 25%, var(--ball)) 100%);
  }
  .s3d-plabel {
    font-size: var(--fs-xs);
    line-height: 1.15;
    text-align: center;
    overflow-wrap: anywhere;
  }
  .s3d-swatches {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .s3d-swatch {
    appearance: none;
    width: 24px;
    height: 24px;
    padding: 0;
    border-radius: var(--radius-s);
    border: 1px solid var(--border-strong);
    background: var(--sw);
    cursor: pointer;
  }
  .s3d-swatch.on {
    box-shadow: 0 0 0 2px var(--bg), 0 0 0 4px var(--accent);
  }
  .s3d-token {
    flex: 1 1 auto;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .s3d-token code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .s3d-dot {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--sw);
    border: 1px solid var(--border);
    flex-shrink: 0;
  }
  .s3d-state-hint {
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--info-soft);
    color: var(--text);
  }
</style>
