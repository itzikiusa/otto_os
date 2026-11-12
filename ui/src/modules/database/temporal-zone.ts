// Which time zone a date-time column's values are SHOWN in. Otto never converts
// them; the drivers render what the session hands back, and that depends on the
// type and on the connection's `timezone` parameter (default UTC):
//   - MySQL TIMESTAMP: converted by the server to the session zone (`SET
//     time_zone`, the parameter, default +00:00).
//   - ClickHouse DateTime / DateTime64: the column's own zone when the type
//     names one (`DateTime('Europe/Berlin')`), else the session_timezone
//     setting (the parameter, default UTC).
//   - Postgres TIMESTAMPTZ: an instant, rendered as UTC with its offset.
//   - MySQL DATETIME, Postgres TIMESTAMP, ClickHouse Date: wall-clock values
//     with no zone at all — saying "UTC" there would be wrong.
// The grid header shows the zone next to the type and says how it was chosen.
// Pure, so node tests import it directly.

export interface ColumnZone {
  /** Short marker for the header ("UTC", "Europe/Berlin", "no zone"). */
  label: string;
  /** One sentence for the tooltip: what the values are and why. */
  note: string;
}

/** The connection parameter as the drivers apply it ('' / 'UTC' / '+00:00' → UTC). */
function sessionZone(param: unknown): string {
  const tz = typeof param === 'string' ? param.trim() : '';
  return !tz || /^utc$/i.test(tz) || tz === '+00:00' ? 'UTC' : tz;
}

const shownIn = (zone: string, why: string): ColumnZone => ({
  label: zone,
  note: `Shown in ${zone} (${why}); Otto does not convert it.`,
});
const NO_ZONE: ColumnZone = { label: 'no zone', note: 'Stored without a time zone; shown exactly as stored.' };

/** The zone a column of `typeHint` is shown in on `engine`, or null when the
 *  type is not a date-time (or the engine renders its own marker). */
export function columnZone(
  engine: string | null | undefined,
  typeHint: string | null | undefined,
  tzParam?: unknown,
): ColumnZone | null {
  if (!typeHint) return null;
  const t = typeHint.trim();
  const upper = t.toUpperCase();
  const session = sessionZone(tzParam);
  const why = tzParam ? "the connection’s Time zone setting" : 'the session default; set Time zone on the connection to change it';
  if (engine === 'mysql') {
    if (upper === 'TIMESTAMP') return shownIn(session, why);
    if (upper === 'DATETIME') return NO_ZONE;
    return null;
  }
  if (engine === 'postgres') {
    if (upper === 'TIMESTAMPTZ') return { label: 'UTC', note: 'An instant, shown in UTC with its offset; Otto does not convert it.' };
    if (upper === 'TIMESTAMP') return NO_ZONE;
    return null;
  }
  if (engine === 'clickhouse') {
    // Unwrap Nullable(...) / LowCardinality(...) to the DateTime inside.
    const m = /\bDateTime(?:64)?\s*(?:\(([^)]*)\))?/.exec(t);
    if (!m) return null;
    const named = /'([^']+)'/.exec(m[1] ?? '');
    return named ? shownIn(named[1], 'the zone named in the column type') : shownIn(session, why);
  }
  return null;
}
