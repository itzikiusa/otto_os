<script lang="ts">
  // Brand Kit → Colors: one card per colour token with a colour input, the hex,
  // its CSS variable and live WCAG contrast badges (vs white and the kit's ink),
  // plus "Try a primary" presets and "Rule from team" chips from learned rules.
  // The swatches are the kit's own colours (user content), set inline.
  import type { BrandDoc, DesignLearnedRule } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import { router } from '../../../lib/router.svelte';
  import { askDescription, askTokenName, tokenMenu } from './edit';
  import { contrast, contrastLevel, cssVarName, inkOf, normalizeHex, tokenLabel, uniqueName, renameKey } from './tokens';

  interface Props {
    doc: BrandDoc;
    readonly?: boolean;
    rules?: DesignLearnedRule[];
  }
  let { doc = $bindable(), readonly = false, rules = [] }: Props = $props();

  const PRESETS: [string, string][] = [
    ['#5B3DF5', 'Violet'],
    ['#2F6FED', 'Signal blue'],
    ['#0F9D8A', 'Teal'],
    ['#E0457B', 'Berry'],
    ['#1D1D1F', 'Graphite'],
  ];

  const ink = $derived(inkOf(doc));
  const entries = $derived(Object.entries(doc.color ?? {}));
  const primaryName = $derived(doc.color?.primary ? 'primary' : (entries[0]?.[0] ?? null));

  function setColor(name: string, value: string): void {
    const tok = doc.color[name];
    if (tok) tok.$value = value;
  }

  function onPick(name: string, e: Event): void {
    setColor(name, (e.currentTarget as HTMLInputElement).value.toUpperCase());
  }

  function badge(fg: string | null, bg: string, label: string): { text: string; tone: string; title: string } {
    const r = fg ? contrast(fg, bg) : null;
    if (r == null) return { text: `${label} —`, tone: 'fail', title: 'Not a hex colour yet' };
    const lvl = contrastLevel(r);
    const tone = lvl === 'AAA' || lvl === 'AA' ? 'pass' : lvl === 'AA-large' ? 'mid' : 'fail';
    const sub =
      lvl === 'fail' ? 'Below 3:1 — not for text or icons' : lvl === 'AA-large' ? 'Only for text 24 px and larger, and icons' : 'Passes WCAG AA for body text';
    return { text: `${label} ${r.toFixed(1)}`, tone, title: `${label}: ${r.toFixed(2)}:1. ${sub}` };
  }

  async function add(): Promise<void> {
    const name = await askTokenName('Add colour', Object.keys(doc.color), { placeholder: uniqueName('brand', Object.keys(doc.color)), confirmLabel: 'Add' });
    if (!name) return;
    doc.color[name] = { $value: '#888888' };
  }

  async function rename(from: string): Promise<void> {
    const to = await askTokenName('Rename colour', Object.keys(doc.color), { initial: from, confirmLabel: 'Rename' });
    if (to) doc.color = renameKey(doc.color, from, to);
  }

  async function describe(name: string): Promise<void> {
    const d = await askDescription(name, doc.color[name]?.$description);
    if (d == null || !doc.color[name]) return;
    if (d) doc.color[name].$description = d;
    else delete doc.color[name].$description;
  }

  function remove(name: string): void {
    const next = { ...doc.color };
    delete next[name];
    doc.color = next;
  }

  function preset(hex: string): void {
    if (!primaryName) {
      doc.color.primary = { $value: hex };
      return;
    }
    setColor(primaryName, hex);
  }
</script>

<section id="brand-colors" class="bsec" aria-labelledby="brand-colors-h">
  <div class="sec-h">
    <h2 id="brand-colors-h">Colors</h2>
    <span class="meta">Contrast is checked against white and {ink.name ? tokenLabel(ink.name) : 'black'}</span>
    <span class="grow"></span>
    {#each rules.slice(0, 2) as r (r.key)}
      <button class="chip rule" title="An approved team rule — open it in What Otto learned" onclick={() => router.go('design/learned/rules')} data-testid="brand-rule-chip">
        <Icon name="bulb" size={12} /> Rule from team: {r.rule}
      </button>
    {/each}
    {#if rules.length > 2}<span class="chip">+{rules.length - 2} rules</span>{/if}
  </div>

  <div class="grid" data-testid="brand-colors">
    {#each entries as [name, tok] (name)}
      {@const hex = normalizeHex(tok?.$value ?? '')}
      {@const w = badge(hex, '#FFFFFF', 'on white')}
      {@const k = badge(hex, ink.hex, `on ${ink.name ? tokenLabel(ink.name).toLowerCase() : 'black'}`)}
      <div class="swc" data-testid="brand-color" data-token={name}>
        <div class="swc-c" style:background={hex ?? 'transparent'} class:invalid={!hex}>
          {#if !readonly}
            <label class="pick" title="Edit {tokenLabel(name)}">
              <input type="color" value={(hex ?? '#888888').slice(0, 7).toLowerCase()} oninput={(e) => onPick(name, e)} aria-label="Edit {tokenLabel(name)} colour" data-testid="brand-color-input" />
              <Icon name="edit" size={13} />
            </label>
          {/if}
        </div>
        <div class="swc-b">
          <div class="row">
            <b class="nm">{tokenLabel(name)}</b>
            <input
              class="hex mono"
              class:bad={!hex}
              value={tok?.$value ?? ''}
              oninput={(e) => setColor(name, e.currentTarget.value.trim())}
              disabled={readonly}
              aria-label="{tokenLabel(name)} hex value"
              spellcheck="false"
              maxlength="9"
            />
            <button class="icon-btn" aria-label="More for {tokenLabel(name)}" title="More" onclick={(e) => tokenMenu(e, 'color', name, { rename: () => void rename(name), describe: () => void describe(name), remove: () => remove(name) }, readonly)}>
              <Icon name="more" size={14} />
            </button>
          </div>
          <span class="mono var">{cssVarName('color', name)}</span>
          {#if tok?.$description}<span class="desc">{tok.$description}</span>{/if}
          <div class="cbs">
            <span class="cb {w.tone}" title={w.title}>{w.text}</span>
            {#if !(ink.name === name)}<span class="cb {k.tone}" title={k.title}>{k.text}</span>{/if}
          </div>
        </div>
      </div>
    {/each}
  </div>

  {#if !readonly}
    <div class="presets">
      <button class="btn small" onclick={add} data-testid="brand-color-add"><Icon name="plus" size={12} /> Add colour</button>
      {#if primaryName}
        <span class="sep" aria-hidden="true"></span>
        <span class="dim">Try a {tokenLabel(primaryName).toLowerCase()}</span>
        {#each PRESETS as [h, n] (h)}
          <button class="pchip" style:background={h} onclick={() => preset(h)} aria-label="Set {primaryName} to {n} {h}" title="{n} {h}"></button>
        {/each}
      {/if}
    </div>
  {/if}
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
  .grow {
    flex: 1;
  }
  .rule {
    color: var(--accent-text);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
    cursor: pointer;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rule:hover {
    background: var(--accent-soft);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
    gap: 12px;
  }
  .swc {
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    min-width: 0;
  }
  .swc-c {
    position: relative;
    height: 76px;
    border-block-end: 1px solid var(--border);
    transition: background 120ms ease-out;
  }
  .swc-c.invalid {
    background: repeating-linear-gradient(45deg, var(--surface-2), var(--surface-2) 6px, var(--surface-3) 6px, var(--surface-3) 12px) !important;
  }
  .pick {
    position: absolute;
    inset-block-start: 8px;
    inset-inline-end: 8px;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-s);
    display: grid;
    place-items: center;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }
  .pick:focus-within {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .pick input {
    position: absolute;
    inset: 0;
    opacity: 0;
    width: 100%;
    height: 100%;
    cursor: pointer;
    border: 0;
    padding: 0;
  }
  .swc-b {
    padding: 10px 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .nm {
    font-weight: 600;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hex {
    width: 84px;
    height: 22px;
    padding: 0 6px;
    font-size: var(--fs-s);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    color: var(--text);
    text-align: end;
  }
  .hex:hover:not(:disabled),
  .hex:focus {
    border-color: var(--border);
    background: var(--surface-2);
    outline: none;
  }
  .hex.bad {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 45%, transparent);
  }
  .var {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .desc {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .cbs {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-block-start: 4px;
  }
  .cb {
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 1px 6px;
    border-radius: var(--radius-s);
    font-variant-numeric: tabular-nums;
    cursor: default;
  }
  .cb.pass {
    color: var(--success);
    background: var(--success-soft);
  }
  .cb.mid {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .cb.fail {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .sep {
    width: 1px;
    height: 16px;
    background: var(--border);
    margin-inline: 4px;
  }
  .presets {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .pchip {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 2px solid var(--surface);
    box-shadow: 0 0 0 1px var(--border);
    cursor: pointer;
    padding: 0;
  }
  .pchip:hover {
    box-shadow: 0 0 0 1px var(--border-strong);
  }
  @media (prefers-reduced-motion: reduce) {
    .swc-c {
      transition: none;
    }
  }
</style>
