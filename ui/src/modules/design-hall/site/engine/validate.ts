// Site Studio — the `otto-site` v1 validator (mirrors
// crates/otto-design/src/site/validate.rs, which gates every save: an agent's
// invalid edit is refused with the same messages). Structural rules only —
// copy is never judged here; accessibility and brand checks live in audit.ts.

import { ITEM_BLOCKS, SECTION_BLOCKS } from './catalog';
import type { SiteIssue } from './types';

export const SITE_LIMITS = {
  pages: 50,
  sectionsPerPage: 200,
  blocksPerSection: 100,
  nodes: 5000,
  text: 20_000,
  listItems: 100,
  issues: 20,
} as const;

export const MOTIONS = ['none', 'fade-up', 'scroll-reveal', 'parallax', 'tilt-hover'];
export const SPACINGS = ['s', 'm', 'l', 'xl'];
export const ALIGNS = ['left', 'center'];
export const MIN_HEIGHTS = ['auto', '80vh', '100vh'];
export const BREAKPOINTS = ['desktop', 'tablet', 'mobile'];
export const STACKS = ['media-first', 'media-last'];
export const GRADIENT_PRESETS = ['soft', 'primary', 'ink', 'sunset'];

const ID_RE = /^[A-Za-z0-9_-]{1,64}$/;
const SLUG_RE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const URI_RE = /^otto:\/\/design\/[A-Za-z0-9_-]{1,64}(?:@(?:approved|latest|v[1-9][0-9]*))?(?:#[A-Za-z0-9_:.-]{1,128})?$/;
const HEX_RE = /^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;
const TOKEN_RE = /^token:[a-z0-9-]+(?:\.[a-z0-9-]+){1,3}$/;
const DATA_IMAGE = /^data:image\/(png|jpeg|gif|webp);base64,[A-Za-z0-9+/=]+$/;

/** Keys that hold a link / an image / a video (checked for unsafe schemes). */
function urlKind(key: string): 'href' | 'src' | 'video' | null {
  if (key === 'href' || key.endsWith('_href') || key === 'form_action') return 'href';
  if (key === 'image' || key === 'poster') return 'src';
  return null;
}

function scheme(v: string): string | null {
  const probe = v.replace(/[\u0000- ]/g, '').toLowerCase();
  if (probe.startsWith('//') || probe.startsWith('\\')) return '//';
  const m = /^([a-z][a-z0-9+.-]*):/.exec(probe);
  return m ? m[1] : null;
}

/** Why a URL-ish value is refused (null = fine). */
export function urlProblem(kind: 'href' | 'src' | 'video', v: string): string | null {
  const s = v.trim();
  if (!s) return null;
  if (kind === 'href' && s.startsWith('page:')) return ID_RE.test(s.slice(5)) ? null : 'page links look like page:<page id>';
  if (s.startsWith('otto://')) return kind !== 'href' && URI_RE.test(s) ? null : 'not a valid otto://design/<id>[@approved|@latest|@vN] reference here';
  const sc = scheme(s);
  if (sc === null) return null; // relative / #anchor
  if (kind === 'href' && (sc === 'http' || sc === 'https' || sc === 'mailto' || sc === 'tel')) return null;
  if (kind === 'src' && sc === 'https') return null;
  if (kind === 'src' && sc === 'data' && DATA_IMAGE.test(s)) return null;
  if (kind === 'video' && sc === 'https') return null;
  return `the ${sc === '//' ? 'protocol-relative' : sc + ':'} scheme is not allowed`;
}

function isObj(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === 'object' && !Array.isArray(v);
}

/** Validate a parsed document. Empty result = valid. */
export function validateSite(doc: unknown): SiteIssue[] {
  const issues: SiteIssue[] = [];
  const add = (path: string, message: string) => {
    if (issues.length < SITE_LIMITS.issues) issues.push({ path, message });
  };
  if (!isObj(doc)) {
    add('', 'the document must be a JSON object');
    return issues;
  }
  if (doc.type !== 'otto-site') add('type', 'must be "otto-site"');
  if (doc.version !== 1) add('version', 'must be 1');
  if (doc.title !== undefined && (typeof doc.title !== 'string' || doc.title.length > 300)) add('title', 'must be a string of at most 300 characters');
  if (doc.brand !== undefined && doc.brand !== '' && (typeof doc.brand !== 'string' || !URI_RE.test(doc.brand))) {
    add('brand', 'must be an otto://design/<brand kit id>[@approved|@latest|@vN] reference');
  }
  if (doc.settings !== undefined) {
    if (!isObj(doc.settings)) add('settings', 'must be an object');
    else
      for (const k of ['domain', 'lang', 'description']) {
        const v = doc.settings[k];
        if (v !== undefined && (typeof v !== 'string' || v.length > 1000)) add(`settings.${k}`, 'must be a string of at most 1000 characters');
      }
  }
  if (doc.pages === undefined) return issues;
  if (!Array.isArray(doc.pages)) {
    add('pages', 'must be an array');
    return issues;
  }
  if (doc.pages.length > SITE_LIMITS.pages) add('pages', `at most ${SITE_LIMITS.pages} pages`);
  const pageIds = new Set<string>();
  const slugs = new Set<string>();
  const nodeIds = new Set<string>();
  let nodes = 0;
  doc.pages.forEach((page: unknown, pi: number) => {
    const pp = `pages[${pi}]`;
    if (!isObj(page)) return add(pp, 'must be an object');
    if (typeof page.id !== 'string' || !ID_RE.test(page.id)) add(`${pp}.id`, 'must be 1–64 letters, digits, _ or -');
    else if (pageIds.has(page.id)) add(`${pp}.id`, `duplicate page id "${page.id}"`);
    else pageIds.add(page.id);
    if (page.title !== undefined && (typeof page.title !== 'string' || page.title.length > 200)) add(`${pp}.title`, 'must be a string of at most 200 characters');
    if (page.description !== undefined && (typeof page.description !== 'string' || page.description.length > 1000)) add(`${pp}.description`, 'must be a string of at most 1000 characters');
    if (page.slug !== undefined) {
      if (typeof page.slug !== 'string' || (page.slug !== '' && (!SLUG_RE.test(page.slug) || page.slug.length > 64))) {
        add(`${pp}.slug`, 'must be empty (home) or lowercase words joined by -');
      } else if (page.slug !== '' && slugs.has(page.slug)) add(`${pp}.slug`, `duplicate slug "${page.slug}"`);
      else if (page.slug !== '') slugs.add(page.slug);
      if (page.slug === 'index') add(`${pp}.slug`, '"index" is reserved for the home page');
    }
    if (page.sections === undefined) return;
    if (!Array.isArray(page.sections)) return add(`${pp}.sections`, 'must be an array');
    if (page.sections.length > SITE_LIMITS.sectionsPerPage) add(`${pp}.sections`, `at most ${SITE_LIMITS.sectionsPerPage} sections per page`);
    page.sections.forEach((sec: unknown, si: number) => {
      const sp = `${pp}.sections[${si}]`;
      if (!isObj(sec)) return add(sp, 'must be an object');
      nodes++;
      checkId(sec.id, `${sp}.id`, nodeIds, add);
      if (typeof sec.block !== 'string' || !SECTION_BLOCKS.includes(sec.block)) {
        add(`${sp}.block`, `unknown section block ${JSON.stringify(sec.block)} (known: ${SECTION_BLOCKS.join(', ')})`);
      }
      if (sec.name !== undefined && (typeof sec.name !== 'string' || sec.name.length > 120)) add(`${sp}.name`, 'must be a string of at most 120 characters');
      if (sec.hidden !== undefined && typeof sec.hidden !== 'boolean') add(`${sp}.hidden`, 'must be true or false');
      if (sec.derived_from !== undefined && (typeof sec.derived_from !== 'string' || !URI_RE.test(sec.derived_from))) {
        add(`${sp}.derived_from`, 'must be an otto://design/<site>@v<n>#<section> reference');
      }
      checkProps(sec.props, `${sp}.props`, add);
      checkStyle(sec.style, `${sp}.style`, add);
      checkResponsive(sec.responsive, `${sp}.responsive`, add);
      if (sec.blocks === undefined) return;
      if (!Array.isArray(sec.blocks)) return add(`${sp}.blocks`, 'must be an array');
      if (sec.blocks.length > SITE_LIMITS.blocksPerSection) add(`${sp}.blocks`, `at most ${SITE_LIMITS.blocksPerSection} blocks per section`);
      sec.blocks.forEach((b: unknown, bi: number) => {
        const bp = `${sp}.blocks[${bi}]`;
        if (!isObj(b)) return add(bp, 'must be an object');
        nodes++;
        checkId(b.id, `${bp}.id`, nodeIds, add);
        if (typeof b.block !== 'string' || !ITEM_BLOCKS.includes(b.block)) {
          add(`${bp}.block`, `unknown block ${JSON.stringify(b.block)} (known: ${ITEM_BLOCKS.join(', ')})`);
        }
        checkProps(b.props, `${bp}.props`, add, b.block === 'embed/3d' || b.block === 'embed/image');
      });
    });
  });
  if (nodes > SITE_LIMITS.nodes) add('pages', `at most ${SITE_LIMITS.nodes} sections and blocks in total`);
  return issues;
}

function checkId(id: unknown, path: string, seen: Set<string>, add: (p: string, m: string) => void): void {
  if (typeof id !== 'string' || !ID_RE.test(id)) add(path, 'must be 1–64 letters, digits, _ or -');
  else if (seen.has(id)) add(path, `duplicate id "${id}" (section and block ids are unique per document)`);
  else seen.add(id);
}

function checkProps(props: unknown, path: string, add: (p: string, m: string) => void, media = false): void {
  if (props === undefined) return;
  if (!isObj(props)) return add(path, 'must be an object');
  for (const [k, v] of Object.entries(props)) {
    const p = `${path}.${k}`;
    if (typeof v === 'string') {
      if (v.length > SITE_LIMITS.text) add(p, `at most ${SITE_LIMITS.text} characters`);
      const kind = k === 'src' ? (media ? 'src' : 'video') : urlKind(k);
      if (kind) {
        const why = urlProblem(kind, v);
        if (why) add(p, why);
      }
    } else if (typeof v === 'number' || typeof v === 'boolean') {
      continue;
    } else if (Array.isArray(v)) {
      if (v.length > SITE_LIMITS.listItems) add(p, `at most ${SITE_LIMITS.listItems} entries`);
      v.forEach((x, i) => {
        if (typeof x === 'string') {
          if (x.length > 2000) add(`${p}[${i}]`, 'at most 2000 characters');
        } else if (isObj(x) && typeof x.label === 'string' && (x.href === undefined || typeof x.href === 'string')) {
          const why = typeof x.href === 'string' ? urlProblem('href', x.href) : null;
          if (why) add(`${p}[${i}].href`, why);
        } else add(`${p}[${i}]`, 'list entries are strings or {label, href} links');
      });
    } else if (v !== null) add(p, 'props are strings, numbers, booleans or lists');
  }
}

function checkEnum(o: Record<string, unknown>, key: string, allowed: string[], path: string, add: (p: string, m: string) => void): void {
  const v = o[key];
  if (v === undefined || v === '') return;
  if (typeof v !== 'string' || !allowed.includes(v)) add(`${path}.${key}`, `must be one of ${allowed.join(', ')}`);
}

function checkStyle(style: unknown, path: string, add: (p: string, m: string) => void): void {
  if (style === undefined) return;
  if (!isObj(style)) return add(path, 'must be an object');
  const bg = style.background;
  if (bg !== undefined && bg !== '') {
    const ok =
      typeof bg === 'string' &&
      (HEX_RE.test(bg) || TOKEN_RE.test(bg) || (bg.startsWith('gradient:') && GRADIENT_PRESETS.includes(bg.slice(9))));
    if (!ok) add(`${path}.background`, 'must be token:<path>, gradient:<soft|primary|ink|sunset> or #hex');
  }
  checkEnum(style, 'spacing', SPACINGS, path, add);
  checkEnum(style, 'align', ALIGNS, path, add);
  checkEnum(style, 'motion', MOTIONS, path, add);
  checkEnum(style, 'min_height', MIN_HEIGHTS, path, add);
}

function checkResponsive(r: unknown, path: string, add: (p: string, m: string) => void): void {
  if (r === undefined) return;
  if (!isObj(r)) return add(path, 'must be an object');
  if (r.hide !== undefined) {
    if (!Array.isArray(r.hide) || r.hide.some((x) => typeof x !== 'string' || !BREAKPOINTS.includes(x))) {
      add(`${path}.hide`, `must be a list of ${BREAKPOINTS.join(', ')}`);
    }
  }
  checkEnum(r, 'stack', STACKS, path, add);
  checkEnum(r, 'mobile_align', ALIGNS, path, add);
}
