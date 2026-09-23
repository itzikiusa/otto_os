<script lang="ts">
  // Brand Kit → Spacing & radius: the space scale (drawn to size, capped for
  // the page) and the corner radii, each with an inline px value.
  import type { BrandDoc } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import { askDescription, askTokenName, tokenMenu } from './edit';
  import { renameKey, uniqueName } from './tokens';

  interface Props {
    doc: BrandDoc;
    readonly?: boolean;
  }
  let { doc = $bindable(), readonly = false }: Props = $props();

  type Group = 'space' | 'radius';
  const space = $derived(Object.entries(doc.space ?? {}));
  const radius = $derived(Object.entries(doc.radius ?? {}));

  function set(group: Group, name: string, e: Event): void {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    const tok = doc[group][name];
    if (tok && Number.isFinite(v)) tok.$value = v;
  }

  async function add(group: Group): Promise<void> {
    const taken = Object.keys(doc[group]);
    const name = await askTokenName(group === 'space' ? 'Add space step' : 'Add radius', taken, {
      placeholder: uniqueName(group === 'space' ? 'md' : 'card', taken),
      confirmLabel: 'Add',
    });
    if (name) doc[group] = { ...doc[group], [name]: { $value: group === 'space' ? 16 : 12 } };
  }

  async function rename(group: Group, from: string): Promise<void> {
    const to = await askTokenName('Rename token', Object.keys(doc[group]), { initial: from, confirmLabel: 'Rename' });
    if (to) doc[group] = renameKey(doc[group], from, to);
  }

  async function describe(group: Group, name: string): Promise<void> {
    const d = await askDescription(name, doc[group][name]?.$description);
    if (d && doc[group][name]) doc[group][name].$description = d;
  }

  function remove(group: Group, name: string): void {
    const next = { ...doc[group] };
    delete next[name];
    doc[group] = next;
  }

  function menu(e: MouseEvent, group: Group, name: string): void {
    tokenMenu(e, group, name, { rename: () => void rename(group, name), describe: () => void describe(group, name), remove: () => remove(group, name) }, readonly);
  }
</script>

<section id="brand-space" class="bsec" aria-labelledby="brand-space-h">
  <div class="sec-h">
    <h2 id="brand-space-h">Spacing &amp; radius</h2>
    <span class="meta">px values; studios snap layout to this scale</span>
  </div>

  <div class="card">
    <h3 class="section-title">Space</h3>
    <div class="scale" data-testid="brand-space">
      {#each space as [name, tok] (name)}
        <div class="step">
          <i class="box" style:width={`${Math.min(Math.max(tok.$value, 2), 64)}px`} style:height={`${Math.min(Math.max(tok.$value, 2), 64)}px`} aria-hidden="true"></i>
          <div class="lbl">
            <span class="mono nm">{name}</span>
            <button class="icon-btn tiny" aria-label="More for space {name}" title="More" onclick={(e) => menu(e, 'space', name)}><Icon name="more" size={12} /></button>
          </div>
          <input class="input num mono" type="number" min="0" max="10000" value={tok.$value} oninput={(e) => set('space', name, e)} disabled={readonly} aria-label="Space {name} in px" />
        </div>
      {:else}
        <p class="empty">No space steps yet.</p>
      {/each}
      {#if !readonly}
        <button class="add" onclick={() => add('space')}><Icon name="plus" size={12} /> Add step</button>
      {/if}
    </div>

    <h3 class="section-title">Radius</h3>
    <div class="radii" data-testid="brand-radius">
      {#each radius as [name, tok] (name)}
        <div class="rad">
          <i class="tile" style:border-radius={`${Math.min(Math.max(tok.$value, 0), 32)}px`} aria-hidden="true"></i>
          <div class="lbl">
            <span class="mono nm">radius-{name}</span>
            <button class="icon-btn tiny" aria-label="More for radius {name}" title="More" onclick={(e) => menu(e, 'radius', name)}><Icon name="more" size={12} /></button>
          </div>
          <input class="input num mono" type="number" min="0" max="10000" value={tok.$value} oninput={(e) => set('radius', name, e)} disabled={readonly} aria-label="Radius {name} in px" />
        </div>
      {:else}
        <p class="empty">No radii yet.</p>
      {/each}
      {#if !readonly}
        <button class="add" onclick={() => add('radius')}><Icon name="plus" size={12} /> Add radius</button>
      {/if}
    </div>
  </div>
</section>

<style>
  .bsec {
    display: flex;
    flex-direction: column;
    gap: 12px;
    scroll-margin-top: 12px;
  }
  .sec-h {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .meta {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 14px 16px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .section-title {
    margin: 4px 0 0;
  }
  .scale,
  .radii {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 16px;
  }
  .step,
  .rad {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    min-width: 72px;
  }
  .box {
    display: block;
    background: var(--accent-soft);
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: 0;
  }
  .tile {
    display: block;
    width: 56px;
    height: 44px;
    background: var(--surface-2);
    border: 1.5px solid var(--border-strong);
  }
  .lbl {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .nm {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tiny {
    width: 20px;
    height: 20px;
  }
  .num {
    width: 72px;
    height: 24px;
    font-size: var(--fs-s);
    text-align: center;
  }
  .add {
    align-self: center;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 8px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .add:hover {
    color: var(--text);
    background: var(--hover);
  }
  .empty {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
