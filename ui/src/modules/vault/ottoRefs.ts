import type { MarkedExtension } from 'marked';
import { escapeHtml } from '../../lib/html.ts';

export type OttoRefKind = 'story' | 'design' | 'canvas';

export interface OttoRef {
  kind: OttoRefKind;
  id: string;
  selector: string | null;
  fragment: string | null;
  raw: string;
  route: string;
  defaultLabel: string;
}

const OTTO_URI_RE =
  /^otto:\/\/(story|design|canvas)\/([A-Za-z0-9._:-]+)(?:@([A-Za-z0-9._-]+))?(?:#([A-Za-z0-9._:/-]+))?$/;

const OTTO_INLINE_RE =
  /otto:\/\/(?:story|design|canvas)\/[A-Za-z0-9._:-]+(?:@[A-Za-z0-9._-]+)?(?:#[A-Za-z0-9._:/-]+)?/g;

function targetAppName(kind: OttoRefKind): string {
  if (kind === 'story') return 'Product';
  if (kind === 'design') return 'Design Hall';
  return 'Canvas';
}

function kindBadgeLabel(kind: OttoRefKind): string {
  if (kind === 'story') return 'Story';
  if (kind === 'design') return 'Design';
  return 'Canvas';
}

/** Parse an `otto://story/<id>`, `otto://design/<id>[@sel][#node]`, or `otto://canvas/<id>` reference. */
export function parseOttoRef(raw: string): OttoRef | null {
  const trimmed = raw.trim();
  const m = OTTO_URI_RE.exec(trimmed);
  if (!m) return null;
  const kind = m[1] as OttoRefKind;
  const id = m[2];
  if (id.includes('..')) return null;
  const selector = m[3] ?? null;
  const fragment = m[4] ?? null;
  const route =
    kind === 'story'
      ? `product/${encodeURIComponent(id)}`
      : kind === 'design'
        ? `design/a/${encodeURIComponent(id)}`
        : 'canvas';
  const suffix = selector ? `@${selector}` : '';
  return {
    kind,
    id,
    selector,
    fragment,
    raw: trimmed,
    route,
    defaultLabel: `${id}${suffix}`,
  };
}

/** Render an `OttoRef` as a safe clickable anchor pill (passes `lib/sanitize.ts` allowlist). */
export function renderOttoRefPill(ref: OttoRef, label?: string | null): string {
  const kindBadge = kindBadgeLabel(ref.kind);
  const target = targetAppName(ref.kind);
  const shown = label && label.trim() && label.trim() !== ref.raw ? label.trim() : ref.defaultLabel;
  return (
    `<a class="otto-ref otto-ref-${ref.kind}" href="#/${escapeHtml(ref.route)}" ` +
    `data-raw="${escapeHtml(ref.raw)}" title="Open in ${target}: ${escapeHtml(ref.raw)}">` +
    `<span class="otto-ref-kind">${kindBadge}</span>` +
    `<span class="otto-ref-label">${escapeHtml(shown)}</span>` +
    `</a>`
  );
}

/** Marked extension: renders `[text](otto://story|design|canvas/...)` links and bare `otto://...` URIs as pills. */
export function ottoRefMarkedExtension(): MarkedExtension {
  return {
    extensions: [
      {
        name: 'ottoRef',
        level: 'inline',
        start(src: string) {
          const i = src.indexOf('otto://');
          return i >= 0 ? i : undefined;
        },
        tokenizer(src: string) {
          const m = /^otto:\/\/(?:story|design|canvas)\/[A-Za-z0-9._:-]+(?:@[A-Za-z0-9._-]+)?(?:#[A-Za-z0-9._:/-]+)?/.exec(src);
          if (!m) return undefined;
          const ref = parseOttoRef(m[0]);
          if (!ref) return undefined;
          return { type: 'ottoRef', raw: m[0], ref };
        },
        renderer(token) {
          const ref = (token as { ref?: OttoRef }).ref;
          return ref ? renderOttoRefPill(ref) : '';
        },
      },
    ],
    renderer: {
      link({ href, text }) {
        const ref = href ? parseOttoRef(href) : null;
        if (!ref) return false;
        return renderOttoRefPill(ref, text);
      },
    },
  };
}

function makePillElement(doc: Document, ref: OttoRef, label?: string | null): HTMLAnchorElement {
  const a = doc.createElement('a');
  a.className = `otto-ref otto-ref-${ref.kind}`;
  a.setAttribute('href', `#/${ref.route}`);
  a.setAttribute('data-raw', ref.raw);
  a.setAttribute('title', `Open in ${targetAppName(ref.kind)}: ${ref.raw}`);
  const kind = doc.createElement('span');
  kind.className = 'otto-ref-kind';
  kind.textContent = kindBadgeLabel(ref.kind);
  const lbl = doc.createElement('span');
  lbl.className = 'otto-ref-label';
  lbl.textContent = label && label.trim() && label.trim() !== ref.raw ? label.trim() : ref.defaultLabel;
  a.append(kind, lbl);
  return a;
}

/** Build the initial live-transclusion preview card shell for `![[otto://design|canvas/...]]`. */
function makeTransclusionCard(doc: Document, ref: OttoRef, alias?: string | null): HTMLDivElement {
  const card = doc.createElement('div');
  card.className = `otto-card otto-card-${ref.kind}`;
  card.setAttribute('data-otto-card', ref.kind);
  card.setAttribute('data-otto-id', ref.id);
  card.setAttribute('data-raw', ref.raw);
  if (ref.selector) card.setAttribute('data-otto-selector', ref.selector);

  const thumb = doc.createElement('div');
  thumb.className = 'otto-card-thumb';
  thumb.setAttribute('aria-hidden', 'true');
  thumb.textContent = kindBadgeLabel(ref.kind);

  const info = doc.createElement('div');
  info.className = 'otto-card-info';

  const head = doc.createElement('div');
  head.className = 'otto-card-head';
  const badge = doc.createElement('span');
  badge.className = 'otto-card-kind';
  badge.textContent = kindBadgeLabel(ref.kind);
  const title = doc.createElement('span');
  title.className = 'otto-card-title';
  title.textContent = alias?.trim() || ref.defaultLabel;
  const status = doc.createElement('span');
  status.className = 'otto-card-status';
  status.textContent = ref.selector ? `@${ref.selector}` : 'Loading preview…';
  head.append(badge, title, status);

  const meta = doc.createElement('div');
  meta.className = 'otto-card-meta';
  meta.textContent = ref.raw;

  info.append(head, meta);

  const open = doc.createElement('a');
  open.className = 'otto-ref otto-card-open btn small';
  open.setAttribute('href', `#/${ref.route}`);
  open.setAttribute('data-raw', ref.raw);
  open.textContent = 'Open in Studio';

  card.append(thumb, info, open);
  return card;
}

/** Parse an unresolved wikilink/embed target text like `otto://design/art-1@approved#hero|Hero` into ref + alias. */
export function parseTransclusionTarget(rawAttr: string, textContent: string): { ref: OttoRef; alias: string | null } | null {
  const [targetPart, alias] = textContent.split('|');
  const ref = parseOttoRef(targetPart.trim()) ?? parseOttoRef(rawAttr.trim());
  if (!ref) return null;
  return { ref, alias: alias?.trim() || null };
}

/** Post-render DOM pass for:
 *  1) `![[otto://design|canvas/...]]` unresolved embeds → rich live preview cards;
 *  2) inline `<code>otto://...</code>` refs (outside `<pre>`) → clickable pills;
 *  3) plain text runs (e.g. worker-rendered notes) containing bare `otto://` URIs → pills. */
export function upgradeOttoRefsInDom(host: HTMLElement): void {
  const doc = host.ownerDocument;
  if (!doc) return;

  // 1. Upgrade `![[otto://...]]` / `[[otto://...]]` anchors emitted by vaultEmbed/wikilink.
  for (const a of Array.from(host.querySelectorAll<HTMLAnchorElement>('a.internal-link.unresolved[data-raw]'))) {
    const rawAttr = a.getAttribute('data-raw') ?? '';
    if (!rawAttr.startsWith('otto://')) continue;
    const parsed = parseTransclusionTarget(rawAttr, a.textContent ?? rawAttr);
    if (!parsed) continue;
    if (parsed.ref.kind === 'design' || parsed.ref.kind === 'canvas') {
      a.replaceWith(makeTransclusionCard(doc, parsed.ref, parsed.alias));
    } else {
      a.replaceWith(makePillElement(doc, parsed.ref, parsed.alias));
    }
  }

  // 2. Inline <code>otto://...</code> (outside fenced <pre> blocks) → clickable pill.
  for (const code of Array.from(host.querySelectorAll('code'))) {
    if (code.closest('pre, a, .otto-card')) continue;
    const ref = parseOttoRef(code.textContent ?? '');
    if (ref) code.replaceWith(makePillElement(doc, ref));
  }

  // 3. Plain text runs (e.g. worker-rendered notes) containing bare otto:// URIs → clickable pills.
  const walker = doc.createTreeWalker(host, 4 /* NodeFilter.SHOW_TEXT */);
  const targets: Text[] = [];
  let node: Node | null;
  while ((node = walker.nextNode())) {
    const parent = node.parentElement;
    if (!parent || parent.closest('a, pre, code, script, style, .otto-card')) continue;
    if (node.nodeValue && node.nodeValue.includes('otto://')) targets.push(node as Text);
  }
  for (const textNode of targets) {
    const text = textNode.nodeValue ?? '';
    OTTO_INLINE_RE.lastIndex = 0;
    const frag = doc.createDocumentFragment();
    let last = 0;
    let replaced = false;
    let m: RegExpExecArray | null;
    while ((m = OTTO_INLINE_RE.exec(text))) {
      const ref = parseOttoRef(m[0]);
      if (!ref) continue;
      if (m.index > last) frag.append(doc.createTextNode(text.slice(last, m.index)));
      frag.append(makePillElement(doc, ref));
      last = m.index + m[0].length;
      replaced = true;
    }
    if (replaced) {
      if (last < text.length) frag.append(doc.createTextNode(text.slice(last)));
      textNode.replaceWith(frag);
    }
  }
}

/** Parse `#/vault/<vaultId>/<encodedNotePath>` deep links (used by Product → Promote to Vault). */
export function parseVaultDeepLink(parts: readonly string[]): { vaultId: number; path: string } | null {
  if (parts[0] !== 'vault' || !parts[1] || !parts[2]) return null;
  const vaultId = Number(parts[1]);
  if (!Number.isInteger(vaultId) || vaultId <= 0) return null;
  let path: string;
  try {
    path = decodeURIComponent(parts.slice(2).join('/'));
  } catch {
    return null;
  }
  const clean = path.replace(/^\/+/, '');
  if (!clean || clean.split('/').some((seg) => seg === '..')) return null;
  return { vaultId, path: clean };
}

interface VaultDeepLinkStore {
  vaults: readonly { id: number }[];
  current: { id: number } | null;
  select(id: number): Promise<void>;
  open(path: string): Promise<void>;
}

/** Consume `#/vault/<vaultId>/<path>` once vaults are loaded and clear the deep-link suffix. */
export async function consumeVaultDeepLink(
  parts: readonly string[],
  store: VaultDeepLinkStore,
  nav: { replace(route: string): void },
): Promise<boolean> {
  const target = parseVaultDeepLink(parts);
  if (!target || !store.vaults.some((v) => v.id === target.vaultId)) return false;
  nav.replace('vault');
  if (store.current?.id !== target.vaultId) await store.select(target.vaultId);
  await store.open(target.path);
  return true;
}
