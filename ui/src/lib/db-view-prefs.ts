// DB Explorer "auto-Vertical" preference — pure (no Svelte, no storage) so the
// resolution + one-time migration are unit-testable. The ui store owns the
// localStorage side (`ui.dbAutoVertical`).
//
// A result with MORE than N columns opens in the Vertical (record-per-block)
// view unless the tab has an explicit pick. That reads well for MongoDB's
// ragged, nested documents — but a SQL table with 12 flat columns is exactly
// what a grid is for, so the threshold is PER ENGINE: MongoDB on (10) by
// default, the SQL engines and Redis off (0 = never).

export type AutoVerticalEngine = 'mongodb' | 'mysql' | 'postgres' | 'clickhouse' | 'redis';

export const AUTO_VERTICAL_ENGINES: readonly { id: AutoVerticalEngine; label: string }[] = [
  { id: 'mongodb', label: 'MongoDB' },
  { id: 'mysql', label: 'MySQL' },
  { id: 'postgres', label: 'PostgreSQL' },
  { id: 'clickhouse', label: 'ClickHouse' },
  { id: 'redis', label: 'Redis' },
];

export type AutoVerticalPrefs = Record<AutoVerticalEngine, number>;

export const AUTO_VERTICAL_DEFAULTS: Readonly<AutoVerticalPrefs> = {
  mongodb: 10,
  mysql: 0,
  postgres: 0,
  clickhouse: 0,
  redis: 0,
};

/** The pre-per-engine setting's default (one threshold for every engine). */
const LEGACY_DEFAULT = 10;

export function clampAutoVertical(n: unknown): number {
  const v = typeof n === 'number' ? n : Number(n);
  return Number.isFinite(v) ? Math.max(0, Math.min(500, Math.round(v))) : 0;
}

/**
 * Resolve the stored preference. `stored` is the per-engine JSON (new key),
 * `legacy` the old single-threshold value. Precedence:
 *  1. a valid per-engine record — merged over the defaults (an engine added
 *     later picks up its default);
 *  2. a legacy value the user set — `0` ("never") stays never everywhere, a
 *     CUSTOM threshold (≠ the old default 10) is someone who deliberately
 *     wanted wide results vertical, so it carries to every engine; the old
 *     default itself is treated as "never chose" and takes the new defaults;
 *  3. the defaults.
 * `migrated` tells the caller to persist `value` under the new key and drop
 * the legacy one (a one-shot migration).
 */
export function resolveAutoVertical(
  stored: string | null,
  legacy: string | null,
): { value: AutoVerticalPrefs; migrated: boolean } {
  if (stored) {
    try {
      const raw = JSON.parse(stored) as Partial<Record<string, unknown>>;
      if (raw && typeof raw === 'object' && !Array.isArray(raw)) {
        const value = { ...AUTO_VERTICAL_DEFAULTS };
        for (const { id } of AUTO_VERTICAL_ENGINES) {
          if (id in raw) value[id] = clampAutoVertical(raw[id]);
        }
        return { value, migrated: false };
      }
    } catch {
      /* corrupt JSON — fall through to the legacy / default path */
    }
  }
  if (legacy !== null && legacy.trim() !== '' && Number.isFinite(Number(legacy))) {
    const n = clampAutoVertical(legacy);
    const value = { ...AUTO_VERTICAL_DEFAULTS };
    if (n !== LEGACY_DEFAULT) for (const { id } of AUTO_VERTICAL_ENGINES) value[id] = n;
    return { value, migrated: true };
  }
  return { value: { ...AUTO_VERTICAL_DEFAULTS }, migrated: false };
}

/** The threshold that applies to one engine (0 = never; unknown engine = 0). */
export function autoVerticalFor(prefs: AutoVerticalPrefs, engine: string | null | undefined): number {
  if (!engine) return 0;
  return (prefs as Record<string, number>)[engine] ?? 0;
}
