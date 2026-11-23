// Outgoing push safety gate (E-09): asks the daemon to scan outgoing commits
// for secrets, sensitive control-path edits (.github/workflows/, .env, .ssh/,
// *.pem, *.key, …), and oversized blobs (> 5 MiB) before `git push` runs.
//
// Older daemons that do not expose POST /repos/{id}/push/preflight return
// 404/405/501 and are skipped gracefully so pushing never breaks across
// daemon versions. Pure formatting + gating logic stays free of top-level
// Svelte/DOM imports so `node --test` can import this module directly.

export interface PreflightFinding {
  kind?: string;
  path?: string;
  file_path?: string;
  detail?: string;
  message?: string;
  commit?: string;
  line?: number;
  bytes?: number;
  size_bytes?: number;
}

export interface PushPreflightReq {
  remote?: string;
  branch?: string;
  force?: boolean;
}

export interface PushPreflightResp {
  ok: boolean;
  commits_ahead?: number;
  secrets?: PreflightFinding[];
  control_paths?: PreflightFinding[];
  large_blobs?: PreflightFinding[];
}

export type PushPreflightResult =
  | { ok: true; skipped?: boolean; bypassed?: boolean; resp?: PushPreflightResp }
  | { ok: false; canceled: true; resp: PushPreflightResp };

export interface PushPreflightConfirmOpts {
  title: string;
  confirmLabel: string;
  danger: boolean;
}

export interface PushPreflightDeps {
  post?: (path: string, body: PushPreflightReq) => Promise<PushPreflightResp>;
  confirm?: (message: string, opts: PushPreflightConfirmOpts) => Promise<boolean>;
}

const MAX_ITEMS_PER_BUCKET = 6;
const MIB = 1024 * 1024;
const KIB = 1024;

function countLabel(n: number, singular: string, pluralWord: string): string {
  return n === 1 ? `1 ${singular}` : `${n} ${pluralWord}`;
}

/** Human-readable byte size for large-blob preflight warnings. */
export function formatPreflightBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '0 B';
  if (bytes >= MIB) return `${(bytes / MIB).toFixed(1)} MiB`;
  if (bytes >= KIB) return `${Math.round(bytes / KIB)} KiB`;
  return `${Math.round(bytes)} B`;
}

/** True when the response reports any preflight issue. */
export function hasPreflightIssues(resp: PushPreflightResp): boolean {
  return (
    !resp.ok ||
    (resp.secrets?.length ?? 0) > 0 ||
    (resp.control_paths?.length ?? 0) > 0 ||
    (resp.large_blobs?.length ?? 0) > 0
  );
}

/** True for HTTP statuses that mean the daemon does not implement push preflight. */
export function isPreflightUnsupported(err: unknown): boolean {
  const status = typeof err === 'object' && err !== null && 'status' in err
    ? Number((err as { status?: unknown }).status)
    : 0;
  return status === 404 || status === 405 || status === 501;
}

function formatFindingLine(item: PreflightFinding): string {
  const rawPath = (item.path ?? item.file_path ?? '').trim() || '(unknown path)';
  const loc = typeof item.line === 'number' && item.line > 0 ? `${rawPath}:${item.line}` : rawPath;
  const size = typeof item.bytes === 'number' ? item.bytes : item.size_bytes;
  const sizePart = typeof size === 'number' && size > 0 ? ` (${formatPreflightBytes(size)})` : '';
  const detail = (item.detail ?? item.message ?? '').trim();
  const sha = typeof item.commit === 'string' && item.commit.trim() !== '' ? item.commit.trim().slice(0, 7) : '';
  const shaPart = sha && !detail.includes(sha) ? ` [${sha}]` : '';
  return detail ? `• ${loc}${sizePart} — ${detail}${shaPart}` : `• ${loc}${sizePart}${shaPart}`;
}

function formatBucket(heading: string, items: readonly PreflightFinding[] | undefined): string | null {
  if (!items || items.length === 0) return null;
  const shown = items.slice(0, MAX_ITEMS_PER_BUCKET).map(formatFindingLine);
  const rest = items.length - shown.length;
  if (rest > 0) shown.push(`• …and ${rest} more`);
  return `${heading} (${items.length}):\n${shown.join('\n')}`;
}

/** Build a concise, human-readable warning body for the push preflight modal. */
export function formatPreflightSummary(
  resp: PushPreflightResp,
  opts?: { remote?: string; branch?: string },
): string {
  const target = opts?.remote && opts?.branch
    ? `${opts.remote}/${opts.branch}`
    : opts?.branch ?? opts?.remote ?? 'the remote';
  const ahead = typeof resp.commits_ahead === 'number' && resp.commits_ahead > 0
    ? ` across ${countLabel(resp.commits_ahead, 'outgoing commit', 'outgoing commits')}`
    : '';
  const sections = [
    formatBucket('Potential secrets', resp.secrets),
    formatBucket('Sensitive control paths', resp.control_paths),
    formatBucket('Large files (> 5 MiB)', resp.large_blobs),
  ].filter((s): s is string => s !== null);

  const lead = `Review these findings before pushing to ${target}${ahead}:`;
  if (sections.length === 0) {
    return `${lead}\n\n• The safety check flagged an issue in the outgoing push.`;
  }
  return `${lead}\n\n${sections.join('\n\n')}`;
}

async function defaultPost(path: string, body: PushPreflightReq): Promise<PushPreflightResp> {
  const { api } = await import('../../lib/api/client');
  return api.post<PushPreflightResp>(path, body);
}

async function defaultConfirm(message: string, opts: PushPreflightConfirmOpts): Promise<boolean> {
  const { confirmer } = await import('../../lib/confirm.svelte');
  return confirmer.ask(message, opts);
}

/** Run the outgoing push preflight gate and prompt on findings.
 *  Returns `{ ok: true }` when safe, skipped (404/405/501), or explicitly bypassed;
 *  returns `{ ok: false, canceled: true, resp }` when the user declines. */
export async function runPushPreflight(
  repoId: string,
  req: PushPreflightReq = {},
  deps: PushPreflightDeps = {},
): Promise<PushPreflightResult> {
  const post = deps.post ?? defaultPost;
  const askConfirm = deps.confirm ?? defaultConfirm;
  let resp: PushPreflightResp;
  try {
    resp = await post(`/repos/${repoId}/push/preflight`, req);
  } catch (err) {
    if (isPreflightUnsupported(err)) return { ok: true, skipped: true };
    throw err;
  }
  if (!hasPreflightIssues(resp)) return { ok: true, resp };

  const proceed = await askConfirm(formatPreflightSummary(resp, { remote: req.remote, branch: req.branch }), {
    title: 'Push safety check found issues',
    confirmLabel: 'Push anyway',
    danger: true,
  });
  return proceed ? { ok: true, bypassed: true, resp } : { ok: false, canceled: true, resp };
}
