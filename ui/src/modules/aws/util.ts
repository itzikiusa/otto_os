// Small formatting helpers shared by the AWS console views.

import type { AwsIdentity } from '../../lib/api/types';

/** 1234567 → "1.2 MB" (binary-ish, 1 decimal above KB). */
export function fmtBytes(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return '—';
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB', 'PB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

/** ISO timestamp → "3m ago" / "2d ago" (falls back to the date for old items). */
export function fmtAgo(iso: string | null | undefined): string {
  if (!iso) return '—';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const s = Math.max(0, Math.round((Date.now() - t) / 1000));
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 30) return `${d}d ago`;
  return new Date(t).toLocaleDateString();
}

/** ISO → local "YYYY-MM-DD HH:MM". */
export function fmtDate(iso: string | null | undefined): string {
  if (!iso) return '—';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const d = new Date(t);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** ms → "1.2 s" / "340 ms". */
export function fmtMs(ms: number | null | undefined): string {
  if (ms == null) return '—';
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms)} ms`;
}

/** Athena pricing: $5 per TB scanned (10 MB minimum per query). */
export function athenaCostUsd(bytes: number | null | undefined): string {
  if (bytes == null) return '—';
  const billed = Math.max(bytes, 10 * 1024 * 1024);
  const usd = (billed / 1024 ** 4) * 5;
  return usd < 0.01 ? `<$0.01` : `$${usd.toFixed(usd < 1 ? 3 : 2)}`;
}

/** The role / user name out of a caller ARN
 *  (`arn:aws:sts::123:assumed-role/Admin/session` → `Admin`). */
export function roleFromArn(identity: AwsIdentity | null | undefined): string {
  const arn = identity?.arn ?? '';
  const tail = arn.split(':').pop() ?? '';
  const parts = tail.split('/');
  if (parts[0] === 'assumed-role' || parts[0] === 'role' || parts[0] === 'user') {
    return parts[1] ?? tail;
  }
  return tail || '—';
}

/** Pretty-print JSON text when it parses; otherwise return it unchanged. */
export function prettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

/** Parse the `?prefix=` deep-link tail out of the router's (already decoded)
 *  bucket segment: `"my-bucket?prefix=logs/2024/"` → `["my-bucket", "logs/2024/"]`. */
export function splitBucketSegment(seg: string | undefined): [string, string] {
  if (!seg) return ['', ''];
  const q = seg.indexOf('?');
  if (q < 0) return [seg, ''];
  const bucket = seg.slice(0, q);
  const m = /(?:^|&)prefix=([^&]*)/.exec(seg.slice(q + 1));
  return [bucket, m ? m[1] : ''];
}
