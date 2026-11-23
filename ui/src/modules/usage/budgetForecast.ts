// Pure budget burn-rate forecast math for Usage Budgets.
// Projects cap-exhaustion cadence from the scope's recent 7-day daily pace
// (when matching daily telemetry is available) or the budget window pace.
import { formatUsd } from '../../lib/metric-format.ts';

export interface BudgetRowLike {
  scope: string;
  key: string;
  spent_usd: number;
  limit_usd: number;
  exceeded: boolean;
}

export interface DailySpendPoint {
  day: string;
  cost_usd: number;
}

export interface ModelDailySpendPoint {
  day: string;
  provider: string;
  cost_usd: number;
}

export interface BudgetForecast {
  label: string;
  tone: 'ok' | 'warn' | 'bad' | 'dim';
  dailyPaceUsd: number;
  daysUntilCap: number | null;
}

/** Average daily USD over the most recent up to `paceDays` distinct calendar
 *  days in `points` (divided by `paceDays` when `points` spans at least that
 *  window, else by the observed day count so sparse early days don't under-report). */
export function recentDailyPace(
  points: readonly DailySpendPoint[],
  paceDays = 7,
  windowDays = 30,
): { paceUsd: number; spanDays: number } | null {
  if (points.length === 0) return null;
  const byDay = new Map<string, number>();
  for (const p of points) {
    if (!p.day || !Number.isFinite(p.cost_usd)) continue;
    byDay.set(p.day, (byDay.get(p.day) ?? 0) + Math.max(0, p.cost_usd));
  }
  if (byDay.size === 0) return null;
  const sorted = [...byDay.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  const spanDays = Math.min(paceDays, Math.max(1, windowDays));
  const slice = sorted.slice(-spanDays);
  const sum = slice.reduce((acc, [, v]) => acc + v, 0);
  // Divide by the full pace window when we have a multi-week history, else by the observed days.
  const denom = sorted.length >= spanDays ? spanDays : slice.length;
  return { paceUsd: sum / Math.max(1, denom), spanDays: denom };
}

export function computeBudgetForecast(
  row: BudgetRowLike,
  opts: {
    windowDays: number;
    summaryWorkspaceId?: string | null;
    daily?: readonly DailySpendPoint[];
    dailyModels?: readonly ModelDailySpendPoint[];
  },
): BudgetForecast | null {
  if (!Number.isFinite(row.limit_usd) || row.limit_usd <= 0) return null;
  const windowDays = Math.max(1, Math.round(opts.windowDays || 30));

  let dailyPaceUsd = 0;
  let paceLabel = `${windowDays}d`;

  if (row.scope === 'provider' && opts.dailyModels?.length) {
    const filtered = opts.dailyModels
      .filter((m) => m.provider === row.key)
      .map((m) => ({ day: m.day, cost_usd: m.cost_usd }));
    const recent = recentDailyPace(filtered, 7, windowDays);
    if (recent) {
      dailyPaceUsd = recent.paceUsd;
      paceLabel = `${recent.spanDays}d`;
    } else {
      dailyPaceUsd = Math.max(0, row.spent_usd) / windowDays;
    }
  } else if (row.scope === 'workspace' && opts.summaryWorkspaceId && opts.summaryWorkspaceId === row.key && opts.daily?.length) {
    const recent = recentDailyPace(opts.daily, 7, windowDays);
    if (recent) {
      dailyPaceUsd = recent.paceUsd;
      paceLabel = `${recent.spanDays}d`;
    } else {
      dailyPaceUsd = Math.max(0, row.spent_usd) / windowDays;
    }
  } else {
    dailyPaceUsd = Math.max(0, row.spent_usd) / windowDays;
  }

  if (row.exceeded || row.spent_usd >= row.limit_usd) {
    return {
      label: dailyPaceUsd > 0.0001 ? `Cap reached · ${formatUsd(dailyPaceUsd)}/day (${paceLabel} pace)` : 'Cap reached',
      tone: 'bad',
      dailyPaceUsd,
      daysUntilCap: 0,
    };
  }

  if (dailyPaceUsd <= 0.0001) {
    return {
      label: 'On track · no recent spend',
      tone: 'dim',
      dailyPaceUsd: 0,
      daysUntilCap: null,
    };
  }

  const remaining = row.limit_usd - Math.max(0, row.spent_usd);
  const daysUntilCap = remaining / dailyPaceUsd;
  const paceStr = `${formatUsd(dailyPaceUsd)}/day`;

  if (daysUntilCap < 1) {
    return {
      label: `At ${paceLabel} pace (${paceStr}), reaches cap in <1 day`,
      tone: 'bad',
      dailyPaceUsd,
      daysUntilCap,
    };
  }
  if (daysUntilCap <= 7) {
    const whole = Math.ceil(daysUntilCap);
    return {
      label: `At ${paceLabel} pace (${paceStr}), reaches cap in ~${whole} ${whole === 1 ? 'day' : 'days'}`,
      tone: 'warn',
      dailyPaceUsd,
      daysUntilCap,
    };
  }
  if (daysUntilCap <= windowDays) {
    const whole = Math.round(daysUntilCap);
    return {
      label: `At ${paceLabel} pace (${paceStr}), reaches cap in ~${whole} days`,
      tone: 'ok',
      dailyPaceUsd,
      daysUntilCap,
    };
  }
  return {
    label: `On track at ${paceLabel} pace (${paceStr})`,
    tone: 'ok',
    dailyPaceUsd,
    daysUntilCap,
  };
}
