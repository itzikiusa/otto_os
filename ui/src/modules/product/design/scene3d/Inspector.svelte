<script lang="ts">
  // Scene inspector — the game-studio right panel. With nothing selected it shows
  // the SCENE (background, grid) and CAMERA panels; an object gets Transform +
  // Material (+ Text / Model) + Notes; a light gets its light panel; a group gets
  // its members. Every field patches the JSON doc through `ops.ts` and emits
  // `onchange(newDoc)` live (no debounce here — the arena owns the autosave).
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
    setNotes,
    setScene,
    setText,
    setTransform,
    setVisible,
    summarize,
  } from './ops';

  interface Props {
    doc: Scene3dDoc;
    selectedId?: string | null;
    onchange: (doc: Scene3dDoc) => void;
    readonly?: boolean;
  }
  let { doc, selectedId = $bindable<string | null>(null), onchange, readonly = false }: Props = $props();

  const sel = $derived(findNode(doc, selectedId));
  const obj = $derived(sel?.kind === 'object' ? sel.node : null);
  const light = $derived(sel?.kind === 'light' ? sel.node : null);
  const group = $derived(sel?.kind === 'group' ? sel.node : null);
  const parent = $derived(selectedId ? parentGroup(doc, selectedId) : null);
  const mat = $derived<Scene3dMaterial>(obj?.material ?? {});

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
            {@render vec3Row('Position', obj.position, 0.01, 3, (v) => emit(setTransform(doc, obj.id, { position: v })), 'm')}
            {@render vec3Row('Rotation', obj.rotation, 0.5, 1, (v) => emit(setTransform(doc, obj.id, { rotation: v })), '°')}
            {@render vec3Row('Scale', obj.scale, 0.01, 3, (v) => emit(setTransform(doc, obj.id, { scale: v })))}
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
          {@render panelHead('material', 'Material')}
          {#if open.material !== false}
            <div class="s3d-panel-body">
              {@render colorRow('Color', mat.color, '#94a3b8', (c) => onMat('color', c))}
              {@render rangeRow('Metalness', mat.metalness ?? 0.1, 0, 1, 0.01, (n) => onMat('metalness', n))}
              {@render rangeRow('Roughness', mat.roughness ?? 0.7, 0, 1, 0.01, (n) => onMat('roughness', n))}
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
    font-size: 12px;
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
    font-size: 13px;
  }
  .s3d-head-name-input {
    font-weight: 600;
    font-size: 13px;
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
    font-size: 11px;
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
    font-size: 10.5px;
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
    font: 600 10.5px var(--font-ui);
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
    font-size: 11px;
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
    font-size: 11.5px;
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
    font-size: 13px;
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
    font-size: 10.5px;
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
    font: 10.5px var(--font-ui);
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
    font: 11.5px/1.45 var(--font-ui);
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
    color: var(--accent);
    padding: 0;
    cursor: pointer;
    font-size: 12px;
  }
  .s3d-link:hover {
    text-decoration: underline;
  }
</style>
