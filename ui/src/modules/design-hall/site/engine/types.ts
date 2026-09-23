// Site Studio — the `otto-site` v1 document (pages → sections → blocks).
//
// The Rust twin lives in crates/otto-design/src/site/ (schema, validator,
// renderer, exporter); both sides render the SAME class structure against
// the same stylesheet (site.css, kept byte-identical — a unit test checks),
// so what the canvas shows is what the static export ships.
//
// Contract: docs/contracts/api.md § Site Studio; docs/features/design-hall.md.

/** Breakpoints (the canvas device widths: 1280 / 834 / 390). */
export type SiteBreakpoint = 'desktop' | 'tablet' | 'mobile';

/** Curated motion presets (all CSS, all off under prefers-reduced-motion). */
export type SiteMotion = 'none' | 'fade-up' | 'scroll-reveal' | 'parallax' | 'tilt-hover';

export type SiteSpacing = 's' | 'm' | 'l' | 'xl';
export type SiteAlign = 'left' | 'center';
export type SiteMinHeight = 'auto' | '80vh' | '100vh';

/**
 * Section style. Every value is optional (`""`/absent = the block's default).
 * `background`: `token:<path>` (a brand-kit colour, e.g. `token:color.primary`),
 * `gradient:<preset>` (soft · primary · ink · sunset) or a raw `#hex`
 * (allowed, flagged "off-brand" in the inspector).
 */
export interface SiteStyle {
  background?: string;
  spacing?: SiteSpacing | '';
  align?: SiteAlign | '';
  motion?: SiteMotion | '';
  min_height?: SiteMinHeight | '';
}

export interface SiteResponsive {
  /** Breakpoints this section is hidden at. */
  hide?: SiteBreakpoint[];
  /** Split layouts on mobile: media above or below the text (default below). */
  stack?: 'media-first' | 'media-last' | '';
  /** Text alignment override on mobile. */
  mobile_align?: SiteAlign | '';
}

/** A link inside a list prop (`links`). */
export interface SiteLink {
  label: string;
  href: string;
}

/** Prop values are plain JSON: copy, flags, numbers and link lists. */
export type SitePropValue = string | number | boolean | string[] | SiteLink[];
export type SiteProps = Record<string, SitePropValue>;

/** A child block of a section (a feature card, a tier, a FAQ row, an embed…). */
export interface SiteBlock {
  id: string;
  block: string;
  props: SiteProps;
}

export interface SiteSection {
  id: string;
  /** `<family>/<variant>`, e.g. `hero/split` (see catalog.ts). */
  block: string;
  /** Layer name (defaults to the block's label). */
  name?: string;
  props: SiteProps;
  style?: SiteStyle;
  responsive?: SiteResponsive;
  blocks?: SiteBlock[];
  /** Provenance of a section inserted from another site ("From your library"):
   *  `otto://design/<site>@v<seq>#<section>` → a `derived_from` link. */
  derived_from?: string;
  /** Hidden everywhere (kept in the document, never rendered or exported). */
  hidden?: boolean;
}

export interface SitePage {
  id: string;
  title: string;
  /** URL slug; `""` = the home page (`index.html`). */
  slug: string;
  description?: string;
  sections: SiteSection[];
}

export interface SiteSettings {
  /** Shown in the editor's URL bar and used for the export's canonical host. */
  domain?: string;
  lang?: string;
  description?: string;
}

export interface SiteDoc {
  type: 'otto-site';
  version: 1;
  title?: string;
  /** `otto://design/<brand kit>[@…]` — extracted as a `uses_tokens` link. */
  brand?: string;
  settings?: SiteSettings;
  pages: SitePage[];
  /** Forward-compatible extras are preserved verbatim. */
  [k: string]: unknown;
}

/** One problem the validator found (`path` is JSON-ish: `pages[0].sections[2].block`). */
export interface SiteIssue {
  path: string;
  message: string;
}
