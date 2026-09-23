// Site Studio — the HTML renderer (document → semantic HTML + site.css classes).
//
// One renderer serves the canvas (`editable: true` adds `data-os-*` hooks for
// selection and inline editing, plus editor-only link badges) and the in-app
// preview (`editable: false`). The static export is rendered by the Rust twin
// (crates/otto-design/src/site/render.rs) — it emits the same markup as this
// file with `editable: false`; change both together.
//
// Everything user- or agent-authored is escaped; links go through `safeHref`
// (http(s), mailto, tel, relative, `#anchor`, `page:<id>` — anything else
// becomes `#`) and images through `safeSrc`. No script is ever emitted: motion
// presets are pure CSS.

import { sectionDef } from './catalog';
import { ARROW_PATH, SITE_ICONS } from './icons';
import { resolveBackground, themeDeclarations, type Theme } from './theme';
import type { SiteBlock, SiteDoc, SiteLink, SitePage, SiteProps, SiteSection } from './types';

export interface EmbedInfo {
  title: string;
  seq: number | null;
  /** `follow_approved` | `follow_latest` | `pinned`. */
  policy: string;
  /** A poster image URL (the 3D artifact's thumbnail), if any. */
  poster: string | null;
  broken?: boolean;
}

export interface RenderCtx {
  theme: Theme;
  /** Canvas mode: selection/edit hooks + editor-only badges. */
  editable?: boolean;
  /** `page:<id>` → URL (export: `tiers.html`; canvas: `#`). */
  pageHref?: (pageId: string) => string;
  /** The home page URL (logos). */
  homeHref?: string;
  /** `otto://design/…` image → URL, or null when unresolved. */
  asset?: (uri: string) => string | null;
  /** `otto://design/…` 3D artifact → what the embed shows. */
  embed?: (uri: string) => EmbedInfo | null;
}

// ── Primitives ───────────────────────────────────────────────────────────────

export function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Escaped text with line breaks kept (`\n` → `<br>`). */
export function escLines(s: string): string {
  return esc(s).replace(/\r?\n/g, '<br>');
}

/** Link sanitizer — see the file header. */
export function safeHref(raw: string, ctx?: Pick<RenderCtx, 'pageHref'>): string {
  const s = raw.trim();
  if (!s) return '#';
  if (s.startsWith('page:')) return ctx?.pageHref ? ctx.pageHref(s.slice(5)) : '#';
  // Browsers ignore tabs/newlines/spaces inside a scheme ("java\tscript:").
  const probe = s.replace(/[\u0000- ]/g, '').toLowerCase();
  if (/^(https?:|mailto:|tel:)/.test(probe)) return s;
  if (/^[a-z][a-z0-9+.-]*:/.test(probe) || probe.startsWith('//') || probe.startsWith('\\')) return '#';
  return s;
}

const DATA_IMAGE = /^data:image\/(png|jpeg|gif|webp);base64,[a-z0-9+/=]+$/i;

/** Image sanitizer: otto:// (resolved by the host), https, data:image, relative. */
export function safeSrc(raw: string, ctx?: Pick<RenderCtx, 'asset'>): string | null {
  const s = raw.trim();
  if (!s) return null;
  if (s.startsWith('otto://design/')) return ctx?.asset ? ctx.asset(s) : null;
  const probe = s.replace(/[\u0000- ]/g, '').toLowerCase();
  if (probe.startsWith('https:')) return s;
  if (probe.startsWith('data:')) return DATA_IMAGE.test(s) ? s : null;
  if (/^[a-z][a-z0-9+.-]*:/.test(probe) || probe.startsWith('//') || probe.startsWith('\\')) return null;
  return s;
}

/** Video sources: https or relative only. */
export function safeVideo(raw: string): string | null {
  const s = raw.trim();
  if (!s) return null;
  const probe = s.replace(/[\u0000- ]/g, '').toLowerCase();
  if (probe.startsWith('https:')) return s;
  if (/^[a-z][a-z0-9+.-]*:/.test(probe) || probe.startsWith('//') || probe.startsWith('\\')) return null;
  return s;
}

export function str(p: SiteProps | undefined, key: string): string {
  const v = p?.[key];
  if (typeof v === 'string') return v;
  if (typeof v === 'number' && Number.isFinite(v)) return String(v);
  return '';
}
export function flag(p: SiteProps | undefined, key: string, dflt = false): boolean {
  const v = p?.[key];
  return typeof v === 'boolean' ? v : dflt;
}
export function lines(p: SiteProps | undefined, key: string): string[] {
  const v = p?.[key];
  if (Array.isArray(v)) return v.filter((x): x is string => typeof x === 'string' && x.trim() !== '');
  if (typeof v === 'string') return v.split(/\r?\n/).map((x) => x.trim()).filter(Boolean);
  return [];
}
export function links(p: SiteProps | undefined, key: string): SiteLink[] {
  const v = p?.[key];
  if (!Array.isArray(v)) return [];
  const out: SiteLink[] = [];
  for (const x of v) {
    if (x && typeof x === 'object' && typeof (x as SiteLink).label === 'string') {
      out.push({ label: (x as SiteLink).label, href: typeof (x as SiteLink).href === 'string' ? (x as SiteLink).href : '#' });
    }
  }
  return out;
}

/** Initials for an avatar ("Priya Natarajan" → "PN"). */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const chars = parts.slice(0, 2).map((w) => Array.from(w)[0] ?? '');
  return chars.join('').toUpperCase();
}

function svg(path: string, cls = 'os-ico'): string {
  return `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true"><path d="${path}"/></svg>`;
}

/** A feature icon: a named glyph, else up to 3 characters of text. */
function icon(name: string): string {
  const key = name.trim();
  if (!key) return '';
  const path = SITE_ICONS[key];
  if (path) return `<span class="os-icon" aria-hidden="true">${svg(path)}</span>`;
  return `<span class="os-icon os-icon--text" aria-hidden="true">${esc(Array.from(key).slice(0, 3).join(''))}</span>`;
}

// ── Render state (edit hooks) ────────────────────────────────────────────────

interface R {
  ctx: RenderCtx;
  /** ` data-os-edit="key"` in the canvas, else ''. */
  ed: (key: string) => string;
  /** ` data-os-block="id"` in the canvas, else ''. */
  blk: (b: SiteBlock) => string;
}

function mk(ctx: RenderCtx): R {
  const on = !!ctx.editable;
  return {
    ctx,
    ed: (key) => (on ? ` data-os-edit="${esc(key)}"` : ''),
    blk: (b) => (on ? ` data-os-block="${esc(b.id)}"` : ''),
  };
}

function textEl(r: R, tag: string, cls: string, p: SiteProps, key: string, multi = false): string {
  const v = str(p, key);
  if (!v.trim()) return '';
  return `<${tag} class="${cls}"${r.ed(key)}>${multi ? escLines(v) : esc(v)}</${tag}>`;
}

function button(r: R, p: SiteProps, labelKey: string, hrefKey: string, kind: 'primary' | 'ghost', extra = ''): string {
  const label = str(p, labelKey);
  if (!label.trim()) return '';
  const arrow = kind === 'ghost' ? svg(ARROW_PATH, 'os-ico os-ico--arrow') : '';
  const cls = `os-btn os-btn--${kind}${extra ? ' ' + extra : ''}`;
  return `<a class="${cls}" href="${esc(safeHref(str(p, hrefKey), r.ctx))}"><span${r.ed(labelKey)}>${esc(label)}</span>${arrow}</a>`;
}

function actions(r: R, p: SiteProps): string {
  const a = button(r, p, 'primary_label', 'primary_href', 'primary');
  const b = button(r, p, 'secondary_label', 'secondary_href', 'ghost');
  return a || b ? `<div class="os-actions">${a}${b}</div>` : '';
}

/** Kicker + h2 + subheading (empty → nothing). */
function head(r: R, p: SiteProps, h: 'h1' | 'h2' = 'h2'): string {
  const k = textEl(r, 'p', 'os-kicker', p, 'eyebrow');
  const t = textEl(r, h, h === 'h1' ? 'os-h1' : 'os-h2', p, 'heading', true);
  const s = textEl(r, 'p', 'os-sub', p, 'subheading', true);
  return k || t || s ? `<header class="os-head">${k}${t}${s}</header>` : '';
}

function items(s: SiteSection, block?: string): SiteBlock[] {
  return (s.blocks ?? []).filter((b) => !block || b.block === block);
}

function iStyle(i: number): string {
  return ` style="--os-i: ${i}"`;
}

function logo(r: R, p: SiteProps): string {
  const text = str(p, 'logo');
  const home = esc(r.ctx.homeHref ?? '#');
  return `<a class="os-logo" href="${home}"><span class="os-logo__mark" aria-hidden="true"></span><span${r.ed('logo')}>${esc(text)}</span></a>`;
}

// ── Media ────────────────────────────────────────────────────────────────────

const POLICY_LABEL: Record<string, string> = {
  follow_approved: 'follows approved',
  follow_latest: 'follows latest',
  pinned: 'pinned',
};

export function embed3d(r: R, b: SiteBlock): string {
  const src = str(b.props, 'src');
  const info = src && r.ctx.embed ? r.ctx.embed(src) : null;
  const title = info?.title || str(b.props, 'label') || 'Your product in 3D';
  const alt = str(b.props, 'alt') || title;
  const cls = ['os-embed', 'os-embed-3d'];
  if (flag(b.props, 'auto_rotate', true)) cls.push('os-embed--spin');
  if (flag(b.props, 'tilt_on_hover', true)) cls.push('os-embed--tilt');
  const stage = info?.poster
    ? `<img class="os-embed-3d__poster" src="${esc(info.poster)}" alt="${esc(alt)}" loading="lazy">`
    : `<div class="os-embed-3d__art" role="img" aria-label="${esc(alt)}"><span class="os-orb"></span><span class="os-ring"></span><span class="os-card3d"><span class="os-card3d__chip"></span><span class="os-card3d__name">${esc(title)}</span></span></div>`;
  let badge = '';
  if (r.ctx.editable) {
    const text = !src
      ? 'Pick a 3D artifact'
      : info?.broken || !info
        ? 'Missing 3D artifact'
        : `${info.title}${info.seq != null ? ` · v${info.seq}` : ''} · ${POLICY_LABEL[info.policy] ?? info.policy}`;
    badge = `<span class="os-edit-badge${!src || info?.broken || !info ? ' os-edit-badge--warn' : ''}" aria-hidden="true">${esc(text)}</span>`;
  }
  const cap = textEl(r, 'figcaption', 'os-caption', b.props, 'caption');
  return `<figure class="${cls.join(' ')}"${r.blk(b)}><div class="os-embed-3d__stage">${stage}</div>${cap}${badge}</figure>`;
}

function imageFigure(r: R, b: SiteBlock, cls = 'os-embed os-embed-img', i?: number): string {
  const src = safeSrc(str(b.props, 'src'), r.ctx);
  const alt = str(b.props, 'alt');
  const img = src
    ? `<img class="os-img" src="${esc(src)}" alt="${esc(alt)}" loading="lazy">`
    : `<div class="os-img os-img--empty" role="img" aria-label="${esc(alt || 'Image placeholder')}"></div>`;
  const cap = textEl(r, 'figcaption', 'os-caption', b.props, 'caption');
  return `<figure class="${cls}"${i != null ? iStyle(i) : ''}${r.blk(b)}>${img}${cap}</figure>`;
}

function media(r: R, s: SiteSection): string {
  const allowed = sectionDef(s.block)?.media ?? [];
  const b = (s.blocks ?? []).find((x) => allowed.includes(x.block));
  if (!b) return '';
  return b.block === 'embed/3d' ? embed3d(r, b) : imageFigure(r, b);
}

/** A feature's picture for rows/bento: its image, else a big icon tile. */
function featureVisual(r: R, b: SiteBlock): string {
  const src = safeSrc(str(b.props, 'image'), r.ctx);
  if (src) return `<img class="os-img" src="${esc(src)}" alt="${esc(str(b.props, 'title'))}" loading="lazy">`;
  const ic = SITE_ICONS[str(b.props, 'icon').trim()] ?? SITE_ICONS.sparkle;
  return `<div class="os-img os-img--empty os-img--icon" aria-hidden="true">${svg(ic)}</div>`;
}

// ── Section bodies ───────────────────────────────────────────────────────────

const DECO = '<div class="os-deco" aria-hidden="true"><span class="os-deco__a"></span><span class="os-deco__b"></span></div>';

function feature(r: R, b: SiteBlock, i: number, extra = ''): string {
  return (
    `<article class="os-card os-feature os-item${extra}"${iStyle(i)}${r.blk(b)}>` +
    icon(str(b.props, 'icon')) +
    textEl(r, 'h3', 'os-h3', b.props, 'title') +
    textEl(r, 'p', 'os-text', b.props, 'body', true) +
    `</article>`
  );
}

function tier(r: R, b: SiteBlock, i: number): string {
  const p = b.props;
  const hl = flag(p, 'highlight');
  const perks = lines(p, 'features')
    .map((x) => `<li>${svg(SITE_ICONS.check)}<span>${esc(x)}</span></li>`)
    .join('');
  return (
    `<article class="os-card os-tier os-item${hl ? ' os-tier--hl' : ''}"${iStyle(i)}${r.blk(b)}>` +
    textEl(r, 'span', 'os-badge', p, 'badge') +
    textEl(r, 'h3', 'os-tier__name', p, 'name') +
    `<p class="os-price"><span class="os-price__v"${r.ed('price')}>${esc(str(p, 'price'))}</span>` +
    (str(p, 'period') ? `<span class="os-price__p"${r.ed('period')}>${esc(str(p, 'period'))}</span>` : '') +
    `</p>` +
    textEl(r, 'p', 'os-text', p, 'blurb', true) +
    (perks ? `<ul class="os-checks">${perks}</ul>` : '') +
    button(r, p, 'cta_label', 'cta_href', hl ? 'primary' : 'ghost', 'os-btn--block') +
    `</article>`
  );
}

function person(r: R, p: SiteProps): string {
  const name = str(p, 'name');
  if (!name.trim() && !str(p, 'role').trim()) return '';
  return (
    `<figcaption class="os-person"><span class="os-avatar" aria-hidden="true">${esc(initials(name))}</span>` +
    `<span class="os-person__who">${textEl(r, 'strong', 'os-person__name', p, 'name')}${textEl(r, 'span', 'os-person__role', p, 'role')}</span></figcaption>`
  );
}

function linkList(r: R, ls: SiteLink[]): string {
  return ls.map((l) => `<a href="${esc(safeHref(l.href, r.ctx))}">${esc(l.label)}</a>`).join('');
}

function body(r: R, s: SiteSection): string {
  const p = s.props ?? {};
  switch (s.block) {
    case 'nav/bar': {
      const nav = items(s, 'item/link')
        .map((b) => `<a class="os-nav__link" href="${esc(safeHref(str(b.props, 'href'), r.ctx))}"${r.blk(b)}><span${r.ed('label')}>${esc(str(b.props, 'label'))}</span></a>`)
        .join('');
      return (
        `<div class="os-wrap os-nav__bar">${logo(r, p)}` +
        (nav ? `<nav class="os-nav__links" aria-label="Main">${nav}</nav>` : '') +
        button(r, p, 'cta_label', 'cta_href', 'primary', 'os-btn--sm') +
        `</div>`
      );
    }
    case 'hero/split':
    case 'hero/centered':
    case 'hero/fullbleed':
    case 'hero/stacked': {
      const m = media(r, s);
      const text =
        `<div class="os-hero__text">` +
        (str(p, 'eyebrow').trim() ? `<p class="os-eyebrow"><span${r.ed('eyebrow')}>${esc(str(p, 'eyebrow'))}</span></p>` : '') +
        textEl(r, 'h1', 'os-h1', p, 'headline', true) +
        textEl(r, 'p', 'os-lead', p, 'subhead', true) +
        actions(r, p) +
        textEl(r, 'p', 'os-trust', p, 'trust') +
        `</div>`;
      return `${DECO}<div class="os-wrap os-hero__grid${m ? ' os-hero__grid--media' : ''}">${text}${m ? `<div class="os-hero__media">${m}</div>` : ''}</div>`;
    }
    case 'features/grid':
      return `<div class="os-wrap">${head(r, p)}<div class="os-grid">${items(s, 'item/feature').map((b, i) => feature(r, b, i)).join('')}</div></div>`;
    case 'features/alternating': {
      const rows = items(s, 'item/feature')
        .map(
          (b, i) =>
            `<div class="os-row os-item"${iStyle(i)}${r.blk(b)}><div class="os-row__media">${featureVisual(r, b)}</div>` +
            `<div class="os-row__text">${icon(str(b.props, 'icon'))}${textEl(r, 'h3', 'os-h3', b.props, 'title')}${textEl(r, 'p', 'os-text', b.props, 'body', true)}</div></div>`,
        )
        .join('');
      return `<div class="os-wrap">${head(r, p)}<div class="os-rows">${rows}</div></div>`;
    }
    case 'features/bento': {
      const cards = items(s, 'item/feature')
        .map(
          (b, i) =>
            `<article class="os-card os-bento__cell os-item${i === 0 ? ' os-bento__lead' : ''}"${iStyle(i)}${r.blk(b)}>` +
            (i === 0 ? `<div class="os-bento__visual">${featureVisual(r, b)}</div>` : icon(str(b.props, 'icon'))) +
            textEl(r, 'h3', 'os-h3', b.props, 'title') +
            textEl(r, 'p', 'os-text', b.props, 'body', true) +
            `</article>`,
        )
        .join('');
      return `<div class="os-wrap">${head(r, p)}<div class="os-bento">${cards}</div></div>`;
    }
    case 'features/steps': {
      const steps = items(s, 'item/step')
        .map(
          (b, i) =>
            `<li class="os-step os-item"${iStyle(i)}${r.blk(b)}><span class="os-step__n" aria-hidden="true">${i + 1}</span>` +
            `${textEl(r, 'h3', 'os-h3', b.props, 'title')}${textEl(r, 'p', 'os-text', b.props, 'body', true)}</li>`,
        )
        .join('');
      return `<div class="os-wrap">${head(r, p)}<ol class="os-steps">${steps}</ol></div>`;
    }
    case 'features/stats': {
      const stats = items(s, 'item/stat')
        .map(
          (b, i) =>
            `<div class="os-stat os-item"${iStyle(i)}${r.blk(b)}>${textEl(r, 'span', 'os-stat__v', b.props, 'value')}${textEl(r, 'span', 'os-stat__l', b.props, 'label')}</div>`,
        )
        .join('');
      return `${DECO}<div class="os-wrap">${head(r, p)}<div class="os-stats">${stats}</div></div>`;
    }
    case 'social/logos': {
      const list = items(s, 'item/logo')
        .map((b, i) => {
          const src = safeSrc(str(b.props, 'image'), r.ctx);
          const inner = src
            ? `<img class="os-logos__img" src="${esc(src)}" alt="${esc(str(b.props, 'name'))}" loading="lazy">`
            : `<span${r.ed('name')}>${esc(str(b.props, 'name'))}</span>`;
          return `<li class="os-logos__item os-item"${iStyle(i)}${r.blk(b)}>${inner}</li>`;
        })
        .join('');
      return `<div class="os-wrap os-logos">${textEl(r, 'p', 'os-logos__label', p, 'label')}<ul class="os-logos__list">${list}</ul></div>`;
    }
    case 'social/testimonials': {
      const cards = items(s, 'item/testimonial')
        .map(
          (b, i) =>
            `<figure class="os-card os-quote os-item"${iStyle(i)}${r.blk(b)}>${textEl(r, 'blockquote', 'os-quote__text', b.props, 'quote', true)}${person(r, b.props)}</figure>`,
        )
        .join('');
      return `<div class="os-wrap">${head(r, p)}<div class="os-quotes">${cards}</div></div>`;
    }
    case 'social/quote':
      return `<div class="os-wrap"><figure class="os-bigquote">${textEl(r, 'blockquote', 'os-bigquote__text', p, 'quote', true)}${person(r, p)}</figure></div>`;
    case 'pricing/tiers':
      return `<div class="os-wrap">${head(r, p)}<div class="os-tiers">${items(s, 'item/tier').map((b, i) => tier(r, b, i)).join('')}</div></div>`;
    case 'pricing/compare': {
      const tiers = items(s, 'item/tier');
      const rows: string[] = [];
      for (const t of tiers) for (const x of lines(t.props, 'features')) if (!rows.includes(x)) rows.push(x);
      const th = tiers
        .map(
          (b) =>
            `<th scope="col"${flag(b.props, 'highlight') ? ' class="os-hl"' : ''}${r.blk(b)}><span class="os-compare__name"${r.ed('name')}>${esc(str(b.props, 'name'))}</span>` +
            `<span class="os-compare__price">${esc(str(b.props, 'price'))}${str(b.props, 'period') ? `<small>${esc(str(b.props, 'period'))}</small>` : ''}</span></th>`,
        )
        .join('');
      const body = rows
        .map(
          (row) =>
            `<tr><th scope="row">${esc(row)}</th>` +
            tiers
              .map((b) =>
                lines(b.props, 'features').includes(row)
                  ? `<td>${svg(SITE_ICONS.check, 'os-ico os-ico--yes')}<span class="os-sr">Included</span></td>`
                  : `<td><span class="os-dash" aria-hidden="true">—</span><span class="os-sr">Not included</span></td>`,
              )
              .join('') +
            `</tr>`,
        )
        .join('');
      const foot = tiers.map((b) => `<td>${button(r, b.props, 'cta_label', 'cta_href', flag(b.props, 'highlight') ? 'primary' : 'ghost', 'os-btn--sm')}</td>`).join('');
      return (
        `<div class="os-wrap">${head(r, p)}<div class="os-table-wrap"><table class="os-compare">` +
        `<thead><tr><th scope="col"><span class="os-sr">Feature</span></th>${th}</tr></thead>` +
        `<tbody>${body}</tbody><tfoot><tr><td></td>${foot}</tr></tfoot></table></div></div>`
      );
    }
    case 'pricing/single': {
      const perks = lines(p, 'features')
        .map((x) => `<li>${svg(SITE_ICONS.check)}<span>${esc(x)}</span></li>`)
        .join('');
      return (
        `<div class="os-wrap">${head(r, p)}<article class="os-card os-single">` +
        `<div class="os-single__top">${textEl(r, 'h3', 'os-tier__name', p, 'name')}${textEl(r, 'span', 'os-badge', p, 'badge')}</div>` +
        `<p class="os-price"><span class="os-price__v"${r.ed('price')}>${esc(str(p, 'price'))}</span>` +
        (str(p, 'period') ? `<span class="os-price__p"${r.ed('period')}>${esc(str(p, 'period'))}</span>` : '') +
        `</p>` +
        (perks ? `<ul class="os-checks os-checks--2">${perks}</ul>` : '') +
        button(r, p, 'cta_label', 'cta_href', 'primary', 'os-btn--block') +
        `</article></div>`
      );
    }
    case 'faq/accordion': {
      const qa = items(s, 'item/faq')
        .map(
          (b, i) =>
            `<details class="os-qa os-item"${iStyle(i)}${r.blk(b)}><summary class="os-qa__q"><span${r.ed('question')}>${esc(str(b.props, 'question'))}</span><span class="os-qa__icon" aria-hidden="true"></span></summary>` +
            `${textEl(r, 'p', 'os-qa__a', b.props, 'answer', true)}</details>`,
        )
        .join('');
      return `<div class="os-wrap os-faq__grid"><div class="os-faq__head">${head(r, p)}</div><div class="os-faq__list">${qa}</div></div>`;
    }
    case 'faq/grid': {
      const qa = items(s, 'item/faq')
        .map(
          (b, i) =>
            `<div class="os-faqgrid__item os-item"${iStyle(i)}${r.blk(b)}>${textEl(r, 'dt', 'os-h3', b.props, 'question')}${textEl(r, 'dd', 'os-text', b.props, 'answer', true)}</div>`,
        )
        .join('');
      return `<div class="os-wrap">${head(r, p)}<dl class="os-faqgrid">${qa}</dl></div>`;
    }
    case 'cta/band':
      return (
        `${DECO}<div class="os-wrap os-ctaband">${textEl(r, 'h2', 'os-h2 os-h2--xl', p, 'headline', true)}` +
        `${textEl(r, 'p', 'os-lead', p, 'subhead', true)}${actions(r, p)}</div>`
      );
    case 'cta/split': {
      const action = str(p, 'form_action').trim();
      const okAction = /^https:\/\//i.test(action) ? action : '';
      const fid = `${s.id}-email`;
      const form =
        `<form class="os-form"${okAction ? ` method="post" action="${esc(okAction)}"` : ''}>` +
        `<label class="os-sr" for="${esc(fid)}">Email address</label>` +
        `<input class="os-input" id="${esc(fid)}" type="email" name="email" autocomplete="email" placeholder="${esc(str(p, 'placeholder'))}"${r.ctx.editable ? ' tabindex="-1"' : ''}>` +
        `<button class="os-btn os-btn--primary" type="${okAction ? 'submit' : 'button'}"><span${r.ed('button_label')}>${esc(str(p, 'button_label') || 'Submit')}</span></button>` +
        `${textEl(r, 'p', 'os-note', p, 'note')}</form>`;
      return (
        `${DECO}<div class="os-wrap os-ctasplit"><div class="os-ctasplit__text">${textEl(r, 'h2', 'os-h2', p, 'headline', true)}` +
        `${textEl(r, 'p', 'os-lead', p, 'subhead', true)}</div>${form}</div>`
      );
    }
    case 'cta/card':
      return (
        `<div class="os-wrap"><div class="os-card os-ctacard">${DECO}${textEl(r, 'p', 'os-kicker', p, 'eyebrow')}` +
        `${textEl(r, 'h2', 'os-h2', p, 'headline', true)}${textEl(r, 'p', 'os-lead', p, 'subhead', true)}` +
        `<div class="os-actions">${button(r, p, 'primary_label', 'primary_href', 'primary')}</div></div></div>`
      );
    case 'content/text': {
      const paras = str(p, 'body')
        .split(/\r?\n\s*\r?\n/)
        .map((x) => x.trim())
        .filter(Boolean)
        .map((x) => `<p>${escLines(x)}</p>`)
        .join('');
      return (
        `<div class="os-wrap os-prose">${textEl(r, 'p', 'os-kicker', p, 'eyebrow')}${textEl(r, 'h2', 'os-h2', p, 'heading', true)}` +
        (paras ? `<div class="os-prose__body"${r.ed('body')}>${paras}</div>` : '') +
        `</div>`
      );
    }
    case 'media/3d-embed': {
      const b = items(s, 'embed/3d')[0];
      return `<div class="os-wrap">${head(r, p)}<div class="os-showcase">${b ? embed3d(r, b) : ''}</div></div>`;
    }
    case 'media/video': {
      const src = safeVideo(str(p, 'src'));
      const poster = safeSrc(str(p, 'poster'), r.ctx);
      const frame = src
        ? `<video class="os-video__el" controls preload="metadata" playsinline src="${esc(src)}"${poster ? ` poster="${esc(poster)}"` : ''}></video>`
        : `<div class="os-video__empty" role="img" aria-label="Video placeholder">${poster ? `<img class="os-video__poster" src="${esc(poster)}" alt="" loading="lazy">` : ''}<span class="os-play" aria-hidden="true">${svg('M8 5v14l11-7z')}</span></div>`;
      const cap = textEl(r, 'figcaption', 'os-caption', p, 'caption');
      const h = textEl(r, 'h2', 'os-h2', p, 'heading', true);
      const sub = textEl(r, 'p', 'os-sub', p, 'subheading', true);
      return `<div class="os-wrap">${h || sub ? `<header class="os-head">${h}${sub}</header>` : ''}<figure class="os-video"><div class="os-video__frame">${frame}</div>${cap}</figure></div>`;
    }
    case 'media/gallery':
      return `<div class="os-wrap">${head(r, p)}<div class="os-gallery">${items(s, 'embed/image').map((b, i) => imageFigure(r, b, 'os-gallery__item os-item', i)).join('')}</div></div>`;
    case 'footer/columns': {
      const cols = items(s, 'item/column')
        .map(
          (b, i) =>
            `<nav class="os-footer__col os-item" aria-label="${esc(str(b.props, 'title') || 'Links')}"${iStyle(i)}${r.blk(b)}>` +
            `${textEl(r, 'h3', 'os-footer__title', b.props, 'title')}<ul>${links(b.props, 'links')
              .map((l) => `<li><a href="${esc(safeHref(l.href, r.ctx))}">${esc(l.label)}</a></li>`)
              .join('')}</ul></nav>`,
        )
        .join('');
      return (
        `<div class="os-wrap os-footer__grid"><div class="os-footer__brand">${logo(r, p)}${textEl(r, 'p', 'os-text', p, 'tagline', true)}</div>` +
        `<div class="os-footer__cols">${cols}</div>${textEl(r, 'p', 'os-legal', p, 'legal')}</div>`
      );
    }
    case 'footer/simple':
      return (
        `<div class="os-wrap os-footer__row">${logo(r, p)}` +
        `<nav class="os-footer__inline" aria-label="Footer">${linkList(r, links(p, 'links'))}</nav>${textEl(r, 'p', 'os-legal', p, 'legal')}</div>`
      );
    default:
      return r.ctx.editable
        ? `<div class="os-wrap"><p class="os-unknown">Unknown block “${esc(s.block)}” — it is kept in the document but not rendered.</p></div>`
        : '';
  }
}

// ── Sections, pages, documents ───────────────────────────────────────────────

/** The `<section>` classes for a section (tone, spacing, motion, responsive…). */
export function sectionClasses(s: SiteSection, t: Theme): { classes: string[]; bgCss: string | null } {
  const [fam, variant = ''] = s.block.split('/');
  const st = s.style ?? {};
  const bg = resolveBackground(st.background, t);
  const c = ['os-sec', `os-${fam}`, `os-${fam}--${variant}`, `os-tone-${bg.tone}`];
  c.push(`os-space-${st.spacing || 'm'}`, `os-align-${st.align || 'left'}`);
  if (st.motion && st.motion !== 'none') c.push(`os-motion-${st.motion}`);
  if (bg.preset) c.push(`os-bg-${bg.preset}`);
  if (st.min_height === '80vh') c.push('os-minh-80');
  if (st.min_height === '100vh') c.push('os-minh-100');
  for (const bp of s.responsive?.hide ?? []) c.push(`os-hide-${bp}`);
  if (s.responsive?.stack === 'media-first') c.push('os-stack-first');
  if (s.responsive?.mobile_align) c.push(`os-malign-${s.responsive.mobile_align}`);
  return { classes: c, bgCss: bg.css };
}

export function renderSection(s: SiteSection, ctx: RenderCtx): string {
  if (s.hidden && !ctx.editable) return '';
  const r = mk(ctx);
  const { classes, bgCss } = sectionClasses(s, ctx.theme);
  if (s.hidden) classes.push('os-hidden');
  const style = bgCss ? ` style="--os-bg: ${esc(bgCss)}"` : '';
  const hook = ctx.editable ? ` data-os-section="${esc(s.id)}"` : '';
  const tag = s.block.startsWith('nav/') ? 'header' : s.block.startsWith('footer/') ? 'footer' : 'section';
  return `<${tag} id="${esc(s.id)}" class="${classes.join(' ')}"${style}${hook}>${body(r, s)}</${tag}>`;
}

export function renderPageBody(page: SitePage, ctx: RenderCtx): string {
  return page.sections.map((s) => renderSection(s, ctx)).join('\n');
}

/** The declarations block the export puts first in its stylesheet. */
export function themeCss(t: Theme): string {
  return `.os-site {\n${themeDeclarations(t)
    .map(([k, v]) => `  ${k}: ${v};`)
    .join('\n')}\n}\n`;
}

/** Page-level rules only a standalone document needs (never injected into the app). */
export function documentCss(t: Theme): string {
  return `html { -webkit-text-size-adjust: 100%; }\nbody { margin: 0; background: ${t.surface}; }\n`;
}

export function pageFileName(doc: SiteDoc, page: SitePage): string {
  return page === doc.pages[0] || !page.slug ? 'index.html' : `${page.slug}.html`;
}

export function pageTitle(doc: SiteDoc, page: SitePage): string {
  const site = (doc.title ?? '').trim();
  if (page === doc.pages[0] || !page.title.trim()) return site || page.title || 'Site';
  return site ? `${page.title} · ${site}` : page.title;
}

/** A full standalone HTML document for one page (in-app preview). */
export function renderDocument(doc: SiteDoc, page: SitePage, ctx: RenderCtx, css: string): string {
  const lang = esc((doc.settings?.lang || 'en').slice(0, 16));
  const desc = (page.description || doc.settings?.description || '').trim();
  return (
    `<!doctype html>\n<html lang="${lang}">\n<head>\n<meta charset="utf-8">\n` +
    `<meta name="viewport" content="width=device-width, initial-scale=1">\n` +
    `<title>${esc(pageTitle(doc, page))}</title>\n` +
    (desc ? `<meta name="description" content="${esc(desc)}">\n` : '') +
    `<meta name="generator" content="Otto Site Studio">\n<style>\n${css}</style>\n</head>\n<body>\n` +
    `<div class="os-site">\n${renderPageBody(page, { ...ctx, editable: false })}\n</div>\n</body>\n</html>\n`
  );
}
