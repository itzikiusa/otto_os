/** OKF 0.2 advisory metadata. These are declarations in a file, never proof of
 * identity or permission to execute an attester. Unknown fields stay untouched. */
const mapping = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const text = (value: unknown): string => typeof value === 'string' ? value.trim() : '';
const instant = (value: unknown): number | null => {
  const raw = text(value);
  const parts = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:Z|[+-](\d{2}):(\d{2}))$/i.exec(raw);
  if (!parts) return null;
  const [year, month, day, hour, minute, second, offsetHour, offsetMinute] = parts.slice(1).map((v) => Number(v ?? 0));
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if (month < 1 || month > 12 || day < 1 || day > days[month - 1] || hour > 23 || minute > 59 || second > 59 || offsetHour > 23 || offsetMinute > 59) return null;
  const n = Date.parse(raw); return Number.isFinite(n) ? n : null;
};
export function okfMetadata(value: unknown, now = Date.now()) {
  const fm = mapping(value);
  const generated = mapping(fm.generated);
  const generatedAt = Object.hasOwn(fm, 'generated') ? text(generated.at) : text(fm.timestamp);
  const generation = instant(generatedAt);
  const rawVerified = Array.isArray(fm.verified) ? fm.verified : fm.verified == null ? [] : [fm.verified];
  const verified = rawVerified.flatMap((raw) => {
    const event = mapping(raw), by = text(event.by), at = text(event.at), time = instant(at);
    return by && time !== null ? [{ by, at, time }] : [];
  });
  const tier = verified.some((v) => v.by.startsWith('human:') && v.by.length > 6) ? 'Declared human review'
    : verified.length ? 'Declared machine verification' : 'Unverified';
  const latest = verified.reduce<number | null>((n, v) => n === null || v.time > n ? v.time : n, null);
  const staleAt = text(fm.stale_after), deadline = instant(staleAt);
  const sources = (Array.isArray(fm.sources) ? fm.sources : []).flatMap((raw) => {
    const source = mapping(raw), resource = text(source.resource);
    return resource ? [{ resource, title: text(source.title), id: text(source.id), author: text(source.author), modified: text(source.last_modified) }] : [];
  });
  return {
    tier, verified, generatedAt, generatedBy: text(generated.by), sources,
    status: text(fm.status) || 'stable', staleAt, stale: deadline !== null && now >= deadline,
    invalidDeadline: fm.stale_after != null && deadline === null,
    changedSinceVerification: generation !== null && latest !== null && generation > latest,
    attestedComputation: text(fm.type).toLowerCase() === 'attested computation',
  };
}
