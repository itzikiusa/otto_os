<script lang="ts">
  // Brand Kit → Typography: the three font roles (display / body / mono, each a
  // local family stack — no web-font CDN) and the type scale (size / line
  // height / weight per style), each rendered as a live specimen in the kit's
  // own font (user content).
  import type { BrandDoc, BrandFontRole } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import { askTokenName, tokenMenu } from './edit';
  import { FONT_ROLES, fontRoleForStyle, renameKey, tokenLabel, uniqueName } from './tokens';

  interface Props {
    doc: BrandDoc;
    readonly?: boolean;
  }
  let { doc = $bindable(), readonly = false }: Props = $props();

  const ROLE_LABEL: Record<BrandFontRole, string> = { display: 'Display', body: 'Body', mono: 'Mono' };
  const styles = $derived(Object.entries(doc.type ?? {}));

  function stackFor(style: string): string {
    const role = fontRoleForStyle(style);
    return doc.font?.[role]?.$value || doc.font?.display?.$value || doc.font?.body?.$value || 'system-ui, sans-serif';
  }

  function sample(style: string): string {
    const role = fontRoleForStyle(style);
    if (role === 'mono') return 'const points = order.total * 2;';
    if (role === 'display') {
      const v = doc.voice?.do?.find((x) => x.trim().length > 12);
      return v ? v.trim() : 'Make something people remember.';
    }
    return 'Every token here reaches every studio — sites, graphics, frames and 3D read the same names.';
  }

  function setFont(role: BrandFontRole, value: string): void {
    const cur = doc.font[role];
    if (cur) cur.$value = value;
    else doc.font[role] = { $value: value };
  }

  function setWeights(role: BrandFontRole, text: string): void {
    const cur = doc.font[role];
    if (!cur) return;
    const ws = text
      .split(/[\s,]+/)
      .map((x) => Number(x))
      .filter((n) => Number.isInteger(n) && n >= 1 && n <= 1000);
    if (ws.length) cur.weights = ws.slice(0, 9);
    else delete cur.weights;
  }

  function removeFont(role: BrandFontRole): void {
    const next = { ...doc.font };
    delete next[role];
    doc.font = next;
  }

  function num(e: Event): number {
    return Number((e.currentTarget as HTMLInputElement).value);
  }

  function setStyle(name: string, key: 'size' | 'line' | 'weight', value: number): void {
    const s = doc.type?.[name];
    if (s && Number.isFinite(value)) s[key] = value;
  }

  async function addStyle(): Promise<void> {
    const taken = Object.keys(doc.type ?? {});
    const name = await askTokenName('Add text style', taken, { placeholder: uniqueName('h3', taken), confirmLabel: 'Add' });
    if (!name) return;
    doc.type = { ...(doc.type ?? {}), [name]: { size: 24, line: 32, weight: 600 } };
  }

  async function renameStyle(from: string): Promise<void> {
    const to = await askTokenName('Rename text style', Object.keys(doc.type ?? {}), { initial: from, confirmLabel: 'Rename' });
    if (to && doc.type) doc.type = renameKey(doc.type, from, to);
  }

  function removeStyle(name: string): void {
    const next = { ...(doc.type ?? {}) };
    delete next[name];
    doc.type = next;
  }
</script>

<section id="brand-type" class="bsec" aria-labelledby="brand-type-h">
  <div class="sec-h">
    <h2 id="brand-type-h">Typography</h2>
    <span class="meta">Local font stacks — no web-font downloads</span>
  </div>

  <div class="card fonts">
    {#each FONT_ROLES as role (role)}
      {@const f = doc.font?.[role]}
      <div class="frow" data-testid="brand-font" data-role={role}>
        <span class="aa" style:font-family={f?.$value || 'inherit'} aria-hidden="true">Aa</span>
        <div class="fmeta">
          <b>{ROLE_LABEL[role]}</b>
          <span class="mono dim">--brand-font-{role}</span>
        </div>
        {#if f}
          <input
            class="input stack mono"
            value={f.$value}
            oninput={(e) => setFont(role, e.currentTarget.value)}
            disabled={readonly}
            aria-label="{ROLE_LABEL[role]} font stack"
            spellcheck="false"
          />
          <input
            class="input weights mono"
            value={(f.weights ?? []).join(', ')}
            onchange={(e) => setWeights(role, e.currentTarget.value)}
            disabled={readonly}
            placeholder="400, 700"
            aria-label="{ROLE_LABEL[role]} weights"
            title="Weights this family ships (comma-separated)"
          />
          {#if !readonly}
            <button class="icon-btn" aria-label="Remove the {ROLE_LABEL[role].toLowerCase()} font" title="Remove" onclick={() => removeFont(role)}>
              <Icon name="x" size={14} />
            </button>
          {/if}
        {:else if !readonly}
          <button class="btn small" onclick={() => setFont(role, role === 'mono' ? 'ui-monospace, Menlo, monospace' : 'system-ui, sans-serif')}>
            <Icon name="plus" size={12} /> Add {ROLE_LABEL[role].toLowerCase()} font
          </button>
        {:else}
          <span class="dim">Not set</span>
        {/if}
      </div>
    {/each}
  </div>

  <div class="card tspec" data-testid="brand-type-scale">
    {#each styles as [name, s] (name)}
      <div class="trow" data-testid="brand-type-style" data-token={name}>
        <div class="tmeta">
          <div class="tname">
            <b>{tokenLabel(name)}</b>
            <button class="icon-btn" aria-label="More for {tokenLabel(name)}" title="More" onclick={(e) => tokenMenu(e, 'type', name, { rename: () => void renameStyle(name), remove: () => removeStyle(name) }, readonly)}>
              <Icon name="more" size={14} />
            </button>
          </div>
          <span class="mono dim">{s.size}/{s.line} · {s.weight}</span>
          {#if !readonly}
            <div class="nums">
              <label>Size <input class="input num" type="number" min="1" max="400" value={s.size} oninput={(e) => setStyle(name, 'size', num(e))} /></label>
              <label>Line <input class="input num" type="number" min="1" max="600" value={s.line} oninput={(e) => setStyle(name, 'line', num(e))} /></label>
              <label>Weight <input class="input num" type="number" min="100" max="1000" step="100" value={s.weight} oninput={(e) => setStyle(name, 'weight', num(e))} /></label>
            </div>
          {/if}
        </div>
        <p
          class="specimen"
          style:font-family={stackFor(name)}
          style:font-size={`${Math.min(Math.max(s.size, 8), 72)}px`}
          style:line-height={`${Math.min(Math.max(s.line, 8), 84)}px`}
          style:font-weight={String(s.weight)}
        >
          {sample(name)}
        </p>
      </div>
    {:else}
      <p class="empty">No text styles yet. Add display, headings and body sizes so every studio sets type the same way.</p>
    {/each}
    {#if !readonly}
      <div class="foot">
        <button class="btn small" onclick={addStyle} data-testid="brand-type-add"><Icon name="plus" size={12} /> Add text style</button>
      </div>
    {/if}
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
  .meta,
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .fonts {
    display: flex;
    flex-direction: column;
  }
  .frow {
    display: grid;
    grid-template-columns: 40px 120px minmax(0, 1fr) 110px auto;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    min-width: 0;
  }
  .frow + .frow {
    border-block-start: 1px solid var(--border);
  }
  .aa {
    font-size: var(--fs-xl);
    font-weight: 600;
    color: var(--text);
  }
  .fmeta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .fmeta .dim {
    font-size: var(--fs-xs);
  }
  .stack,
  .weights {
    min-width: 0;
    font-size: var(--fs-s);
  }
  .tspec {
    display: flex;
    flex-direction: column;
  }
  .trow {
    display: grid;
    grid-template-columns: 170px minmax(0, 1fr);
    gap: 20px;
    padding: 18px 20px;
    align-items: center;
  }
  .trow + .trow {
    border-block-start: 1px solid var(--border);
  }
  .tmeta {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .tname {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .tname b {
    font-weight: 600;
  }
  .tmeta .dim {
    font-size: var(--fs-xs);
  }
  .nums {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-block-start: 4px;
  }
  .nums label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .num {
    width: 72px;
    height: 24px;
    font-size: var(--fs-s);
  }
  .specimen {
    margin: 0;
    color: var(--text);
    letter-spacing: -0.01em;
    overflow-wrap: anywhere;
    min-width: 0;
  }
  .empty {
    margin: 0;
    padding: 18px 20px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .foot {
    padding: 10px 14px;
    border-block-start: 1px solid var(--border);
  }
  @media (max-width: 720px) {
    .frow {
      grid-template-columns: 32px minmax(0, 1fr) auto;
    }
    .frow .stack,
    .frow .weights {
      grid-column: 1 / -1;
    }
    .trow {
      grid-template-columns: minmax(0, 1fr);
      gap: 10px;
    }
  }
</style>
