// Auto-refresh cadence for the AWS service views (ViewToolbar). Kept out of the
// component so the all-regions floor is unit-tested (unit/pollBackoff.test.ts).
import { ALL_REGIONS } from '../../lib/api/aws';

/** Single-region auto-refresh: one `aws` CLI process per tick. */
export const AUTO_REFRESH_MS = 10_000;
/** "All enabled regions" fans one refresh out to ~17 `aws` processes (one per
 *  region), so a 10 s tick kept a couple of cores busy — floor it at 30 s. */
export const ALL_REGIONS_REFRESH_FLOOR_MS = 30_000;

export function effectiveRefreshMs(intervalMs: number, region: string | undefined): number {
  return region === ALL_REGIONS ? Math.max(intervalMs, ALL_REGIONS_REFRESH_FLOOR_MS) : intervalMs;
}
