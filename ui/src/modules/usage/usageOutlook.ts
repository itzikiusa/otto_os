// The Usage page's first screen in words (pure, no Svelte; unit/usageOutlook.test.ts):
//   "This month ≈ $854 · on track for $1,120"  (monthOutlook)
//   "Claude budget 92 % used"                  (budgetHeadsUp, only at ≥ 80 %)
// and the rows of the "Where it went" table (costRows), biggest cost first.
import type { DailySpendPoint } from './budgetForecast.ts';

export interface MonthOutlook {
  /** Spend from the 1st of this month through today. */
  monthToDate: number;
  /** monthToDate + the last 7 days' daily average × the days left in the month. */
  projected: number;
  /** Average spend per day the projection uses. */
  paceUsd: number;
}

/** "YYYY-MM-DD" of `now` in local time (the day keys the daemon groups by). */
export function localDay(now: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
}

/** This month's spend so far and where it is heading — null when the window
 *  does not reach back to the 1st (a 7-day window on the 20th cannot say what
 *  the month cost) or there is no spend at all. */
export function monthOutlook(daily: readonly DailySpendPoint[], windowDays: number, now: Date): MonthOutlook | null {
  const today = localDay(now);
  const dayOfMonth = now.getDate();
  if (windowDays < dayOfMonth || daily.length === 0) return null;
  const monthKey = today.slice(0, 7);
  const weekStart = localDay(new Date(now.getFullYear(), now.getMonth(), now.getDate() - 6));
  let monthToDate = 0;
  let lastWeek = 0;
  for (const d of daily) {
    if (d.day > today || !Number.isFinite(d.cost_usd)) continue;
    const cost = Math.max(0, d.cost_usd);
    if (d.day.startsWith(monthKey)) monthToDate += cost;
    if (d.day >= weekStart) lastWeek += cost;
  }
  // Days without a row spent nothing: the average is over the 7 calendar days.
  const pace = lastWeek / 7;
  if (monthToDate <= 0 && pace <= 0) return null;
  const daysInMonth = new Date(now.getFullYear(), now.getMonth() + 1, 0).getDate();
  return { monthToDate, projected: monthToDate + pace * (daysInMonth - dayOfMonth), paceUsd: pace };
}

export interface BudgetRowLike {
  scope: string;
  key: string;
  label: string | null;
  used_fraction: number;
  exceeded: boolean;
}

/** One line per budget at 80 % or more ("Claude budget 92 % used", "Harbor
 *  budget: over"), the fullest first — what earns a place on the first screen. */
export function budgetHeadsUp(rows: readonly BudgetRowLike[], nameOf: (r: BudgetRowLike) => string): { text: string; over: boolean }[] {
  return rows
    .filter((r) => r.exceeded || r.used_fraction >= 0.8)
    .sort((a, b) => b.used_fraction - a.used_fraction)
    .map((r) => ({
      text: r.exceeded ? `${nameOf(r)} budget is used up` : `${nameOf(r)} budget ${Math.round(r.used_fraction * 100)} % used`,
      over: r.exceeded,
    }));
}

export interface CostRowSource {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

export interface CostRow<T> {
  key: string;
  label: string;
  item: T;
  /** Share of the largest cost in the list (0–100), for the row's bar. */
  share: number;
}

/** The rows of one "Where it went" view: biggest cost first (ties by tokens). */
export function costRows<T extends CostRowSource>(items: readonly T[], keyOf: (t: T) => string, labelOf: (t: T) => string): CostRow<T>[] {
  const sorted = [...items].sort((a, b) => b.cost_usd - a.cost_usd || b.total_tokens - a.total_tokens);
  const max = Math.max(0, ...sorted.map((t) => t.cost_usd));
  return sorted.map((item) => ({ key: keyOf(item), label: labelOf(item), item, share: max > 0 ? (item.cost_usd / max) * 100 : 0 }));
}
