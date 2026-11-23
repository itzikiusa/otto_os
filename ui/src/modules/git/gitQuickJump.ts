// Pure Git → IDE & Git → Browser deep-link builders and PR preview URL
// detection (zero cross-module imports; routes via Otto's hash router).
import type { PrCheck, PrChecksResp } from '../../lib/api/types';

export interface PreviewUrl {
  name: string;
  url: string;
  state: PrCheck['state'];
}

/** Build an `#/ide?...` router path (without leading `#/`) to open `relPath`
 *  inside `repoRoot`, optionally jumping to 1-based `line`. Rejects traversal. */
export function buildIdeRoute(
  repoRoot: string | null | undefined,
  filePath: string | null | undefined,
  line?: number | null,
): string | null {
  const root = (repoRoot ?? '').trim().replace(/\/+$/, '');
  const raw = (filePath ?? '').trim();
  if (!root || !raw) return null;
  const rel = raw.replace(/^\.\/+/, '');
  const segments = rel.split('/');
  if (segments.some((s) => s === '..')) return null;
  const abs = raw.startsWith('/') ? raw : `${root}/${rel.replace(/^\/+/, '')}`;
  const params = new URLSearchParams({ root, file: abs });
  if (typeof line === 'number' && Number.isFinite(line) && line >= 1) {
    params.set('line', String(Math.floor(line)));
  }
  return `ide?${params.toString()}`;
}

/** Build a `browser?...` router path to open an http(s) URL in Otto's in-app
 *  Browser (defaulting to live mode for interactive preview environments). */
export function buildBrowserRoute(url: string | null | undefined, mode: 'live' | 'reader' = 'live'): string | null {
  const raw = (url ?? '').trim();
  if (!raw) return null;
  try {
    const parsed = new URL(raw);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
    const params = new URLSearchParams({ url: parsed.href, mode });
    return `browser?${params.toString()}`;
  } catch {
    return null;
  }
}

const PREVIEW_NAME_RE = /\b(preview|deploy(?:ment|ed)?|vercel|netlify|cloudflare\s+pages|pages|staging|review[\s-]?app|storybook|chromatic|demo|sandbox|surge)\b/i;
const PREVIEW_HOST_RE = /(?:^|\.)(?:vercel\.app|netlify\.app|pages\.dev|fly\.dev|onrender\.com|herokuapp\.com|surge\.sh|chromatic\.com|ngrok(?:-free)?\.(?:app|io)|web\.app|firebaseapp\.com)$/i;
const PREVIEW_SUBDOMAIN_RE = /(?:^|\.)(?:preview|staging|review)[.-]/i;
const CI_LOG_HOST_RE = /(?:^|\.)(?:github\.com|gitlab\.com|bitbucket\.org|circleci\.com|travis-ci\.org|buildkite\.com|app\.harness\.io|dev\.azure\.com)$/i;

function isPreviewCheck(check: PrCheck, parsed: URL): boolean {
  const host = parsed.hostname.toLowerCase();
  // Never treat forge CI job/log pages as deployable web previews.
  if (CI_LOG_HOST_RE.test(host)) return false;
  if (PREVIEW_HOST_RE.test(host) || PREVIEW_SUBDOMAIN_RE.test(host)) return true;
  if (PREVIEW_NAME_RE.test(check.name)) return true;
  return false;
}

const STATE_RANK: Record<PrCheck['state'], number> = {
  success: 0,
  pending: 1,
  neutral: 2,
  skipped: 3,
  failure: 4,
};

/** Extract unique deployment/preview URLs from `PrChecksResp`, preferring
 *  successful deployments first and ignoring raw CI log links. */
export function detectPreviewUrls(checks: PrChecksResp | null | undefined): PreviewUrl[] {
  const list = checks?.checks ?? [];
  if (list.length === 0) return [];
  const ranked = list
    .map((check) => {
      const raw = (check.url ?? '').trim();
      if (!raw) return null;
      let parsed: URL;
      try {
        parsed = new URL(raw);
      } catch {
        return null;
      }
      if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return null;
      if (!isPreviewCheck(check, parsed)) return null;
      return { name: check.name.trim() || parsed.hostname, url: parsed.href, state: check.state };
    })
    .filter((x): x is PreviewUrl => x !== null)
    .sort((a, b) => (STATE_RANK[a.state] ?? 9) - (STATE_RANK[b.state] ?? 9));

  const seen = new Set<string>();
  const out: PreviewUrl[] = [];
  for (const item of ranked) {
    const key = item.url.replace(/\/+$/, '');
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(item);
  }
  return out;
}
