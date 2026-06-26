import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, providers, series, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  MetricStat,
  BarChart,
  Sparkline,
  Ring,
  Segmented,
  Chip,
  Card,
  Icon,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  USAGE, COST & INSIGHTS
//  Otto tails Claude & Codex transcripts into an embedded ClickHouse engine:
//  per-turn token + cost breakdown (input / output / cache-read / cache-write),
//  per-provider / day / session rollups, system metrics (CPU / RAM), opt-in
//  budgets, and scheduled multi-provider catch-up reports (daily / weekly /
//  monthly) that turn recent activity into action-first HTML summaries.
// ════════════════════════════════════════════════════════════════════════════

// ── local helpers ─────────────────────────────────────────────────────────────

const SectionLabel: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div
    style={{
      fontFamily: fonts.ui,
      fontSize: 11.5,
      fontWeight: 600,
      letterSpacing: 0.7,
      textTransform: 'uppercase',
      color: T.textDim,
      marginBottom: 10,
    }}
  >
    {children}
  </div>
);

const BreakdownBox: React.FC<{
  label: string;
  value: string;
  pct: string;
  color: string;
}> = ({ label, value, pct, color }) => (
  <div
    style={{
      flex: 1,
      background: T.surface,
      border: `1px solid ${T.border}`,
      borderRadius: 8,
      padding: '10px 13px',
      display: 'flex',
      flexDirection: 'column',
      gap: 3,
    }}
  >
    <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
      <span
        style={{
          width: 8,
          height: 8,
          borderRadius: 2,
          background: color,
          flexShrink: 0,
          boxShadow: `0 0 6px ${alpha(color, 0.55)}`,
        }}
      />
      <span style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim }}>
        {label}
      </span>
    </div>
    <div
      style={{
        fontFamily: fonts.ui,
        fontSize: 20,
        fontWeight: 700,
        color: T.text,
        letterSpacing: -0.3,
      }}
    >
      {value}
    </div>
    <div style={{ fontFamily: fonts.ui, fontSize: 11, color: T.textDim }}>
      {pct} of total
    </div>
  </div>
);

const ProviderBar: React.FC<{
  name: string;
  providerColor: string;
  cost: string;
  pct: string;
  frac: number;
  delay: number;
}> = ({ name, providerColor, cost, pct, frac, delay }) => (
  <Appear delay={delay} y={8}>
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
        <span
          style={{
            width: 10,
            height: 10,
            borderRadius: '50%',
            background: providerColor,
            flexShrink: 0,
            boxShadow: `0 0 7px ${alpha(providerColor, 0.55)}`,
          }}
        />
        <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text, flex: 1 }}>
          {name}
        </span>
        <span
          style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 700, color: T.text }}
        >
          {cost}
        </span>
        <Chip>{pct}</Chip>
      </div>
      <div
        style={{
          height: 8,
          borderRadius: 999,
          background: T.surface2,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            width: `${frac * 100}%`,
            height: '100%',
            borderRadius: 999,
            background: `linear-gradient(90deg, ${alpha(providerColor, 0.6)}, ${providerColor})`,
          }}
        />
      </div>
    </div>
  </Appear>
);

const BulletRow: React.FC<{
  icon: string;
  color: string;
  text: string;
  sub: string;
  delay: number;
}> = ({ icon, color, text, sub, delay }) => (
  <Appear delay={delay} y={12}>
    <div style={{ display: 'flex', gap: 14, alignItems: 'flex-start' }}>
      <div
        style={{
          width: 34,
          height: 34,
          borderRadius: 9,
          background: alpha(color, 0.13),
          border: `1px solid ${alpha(color, 0.35)}`,
          display: 'grid',
          placeItems: 'center',
          flexShrink: 0,
          marginTop: 1,
        }}
      >
        <Icon name={icon} size={16} color={color} />
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
        <span
          style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 600, color: T.text }}
        >
          {text}
        </span>
        <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
          {sub}
        </span>
      </div>
    </div>
  </Appear>
);

// ── data ─────────────────────────────────────────────────────────────────────

const COST_DATA = [
  18.4, 22.1, 15.7, 28.9, 31.2, 24.6, 19.8, 35.1, 27.4, 22.9, 41.2, 33.6, 28.1, 38.7,
];
const DAY_LABELS = [
  'Jun 13', 'Jun 14', 'Jun 15', 'Jun 16', 'Jun 17', 'Jun 18', 'Jun 19',
  'Jun 20', 'Jun 21', 'Jun 22', 'Jun 23', 'Jun 24', 'Jun 25', 'Jun 26',
];

const CPU_DATA = [
  42, 38, 55, 71, 63, 48, 52, 68, 74, 61, 57, 66, 73, 58, 62, 55, 68, 72, 64, 59,
];
const RAM_DATA = [
  6.2, 6.4, 6.8, 7.1, 7.0, 6.9, 7.3, 7.5, 7.4, 7.2, 7.6, 7.8, 7.9, 7.7, 7.6, 7.8, 8.0,
  7.9, 7.8, 7.7,
];

const BULLETS: { icon: string; color: string; text: string; sub: string }[] = [
  {
    icon: 'pr',
    color: series[0],
    text: 'Review PR #312 — 3 tests added by agent in sinatra-go need your sign-off',
    sub: 'feat/auth-refactor · opened 2 h ago by claude · go test ./internal/auth/... passing',
  },
  {
    icon: 'gauge',
    color: series[2],
    text: 'Budget at 64% ($319 / $500) — claude accounts for 79% of spend this week',
    sub: 'Pace: $26/day · projected to close at $428 · 12 days remaining',
  },
  {
    icon: 'bell',
    color: series[4],
    text: '12 sessions completed overnight · "refactor api/v2" closed with PR #147 opened',
    sub: '247 sessions total · 3 agents still running · 1 needs your input',
  },
];

// ── Scene 1 — Title (~75f) ────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Usage, Cost & Insights"
    title="Know Your Agents"
    subtitle="Real token cost tailed from transcripts · budgets · catch-up reports"
  />
);

// ── Scene 2 — Usage Dashboard (~190f) ─────────────────────────────────────────

const UsageDashboardScene: React.FC = () => {
  const frame = useCurrentFrame();
  const chartGrow = track(frame, [44, 110], [0, 1]);

  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="usage" />}
          title="Otto — Usage"
        >
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 14,
              padding: 18,
              height: '100%',
              boxSizing: 'border-box',
              overflow: 'hidden',
            }}
          >
            {/* ── metric strip ── */}
            <div style={{ display: 'flex', gap: 12, flexShrink: 0 }}>
              <Appear delay={10} y={14} style={{ flex: 1 }}>
                <MetricStat
                  label="Total Tokens"
                  value="48.2M"
                  delta="↑ 12% vs last week"
                  deltaTone="ok"
                  accent={series[0]}
                  style={{ width: '100%' }}
                />
              </Appear>
              <Appear delay={16} y={14} style={{ flex: 1 }}>
                <MetricStat
                  label="Total Cost"
                  value="$312.40"
                  delta="↑ 8% vs last week"
                  deltaTone="ok"
                  style={{ width: '100%' }}
                />
              </Appear>
              <Appear delay={22} y={14} style={{ flex: 1 }}>
                <MetricStat
                  label="Sessions"
                  value="247"
                  delta="↑ 34 this week"
                  deltaTone="ok"
                  accent={series[2]}
                  style={{ width: '100%' }}
                />
              </Appear>
              <Appear delay={28} y={14} style={{ flex: 1 }}>
                <MetricStat
                  label="Cache Savings"
                  value="$48.70"
                  delta="15.6% of total spend"
                  deltaTone="ok"
                  accent={series[4]}
                  style={{ width: '100%' }}
                />
              </Appear>
            </div>

            {/* ── cost-by-day bar chart ── */}
            <Appear
              delay={30}
              y={10}
              style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}
            >
              <Card
                style={{
                  flex: 1,
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 10,
                  minHeight: 0,
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    flexShrink: 0,
                  }}
                >
                  <span
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 13,
                      fontWeight: 600,
                      color: T.text,
                    }}
                  >
                    Cost by Day (USD)
                  </span>
                  <div style={{ display: 'flex', gap: 8 }}>
                    <Chip>Jun 13 – Jun 26</Chip>
                    <Chip tone="ok">claude + codex</Chip>
                  </div>
                </div>
                <div style={{ flex: 1, minHeight: 0, display: 'flex', alignItems: 'flex-end' }}>
                  <BarChart
                    data={COST_DATA}
                    labels={DAY_LABELS}
                    color={series[0]}
                    grow={chartGrow}
                    height={190}
                  />
                </div>
              </Card>
            </Appear>

            {/* ── token breakdown row ── */}
            <div style={{ display: 'flex', gap: 10, flexShrink: 0 }}>
              <Appear delay={78} y={10} style={{ flex: 1 }}>
                <BreakdownBox label="Input" value="31.4M" pct="65%" color={series[0]} />
              </Appear>
              <Appear delay={86} y={10} style={{ flex: 1 }}>
                <BreakdownBox label="Output" value="9.2M" pct="19%" color={series[1]} />
              </Appear>
              <Appear delay={94} y={10} style={{ flex: 1 }}>
                <BreakdownBox label="Cache Read" value="6.8M" pct="14%" color={series[4]} />
              </Appear>
              <Appear delay={102} y={10} style={{ flex: 1 }}>
                <BreakdownBox label="Cache Write" value="0.8M" pct="2%" color={series[5]} />
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>

      <Caption
        step={1}
        title="Real per-turn tokens & cost"
        sub="Tailed from transcripts, zero instrumentation · input / output / cache-read / cache-write breakdown"
        delay={16}
      />
    </>
  );
};

// ── Scene 3 — Budgets + System (~150f) ────────────────────────────────────────

const BudgetsSystemScene: React.FC = () => {
  const frame = useCurrentFrame();
  const sparkProgress = track(frame, [34, 100], [0, 1]);

  return (
    <>
      <Stage scale={0.88}>
        <OttoWindow
          nav={<Navigator active="usage" />}
          title="Otto — Usage · Budgets & System"
        >
          <div
            style={{
              display: 'flex',
              gap: 14,
              padding: 18,
              height: '100%',
              boxSizing: 'border-box',
              overflow: 'hidden',
            }}
          >
            {/* ── left: budget ring + provider breakdown ── */}
            <div
              style={{
                width: 340,
                flexShrink: 0,
                display: 'flex',
                flexDirection: 'column',
                gap: 14,
              }}
            >
              <Appear delay={8} y={16}>
                <Card
                  style={{
                    display: 'flex',
                    flexDirection: 'column',
                    alignItems: 'center',
                    padding: 20,
                    gap: 12,
                  }}
                >
                  <SectionLabel>Monthly Budget</SectionLabel>
                  <Ring value={0.64} size={144} color={series[2]} label="64%" />
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 13,
                      color: T.textDim,
                      textAlign: 'center',
                    }}
                  >
                    <span style={{ color: T.text, fontWeight: 700 }}>$319.40</span>
                    {' '}of $500.00 used
                  </div>
                  <div style={{ display: 'flex', gap: 6 }}>
                    <Chip tone="warn">12 days left</Chip>
                    <Chip tone="ok">On track</Chip>
                  </div>
                </Card>
              </Appear>

              <Appear delay={18} y={14}>
                <Card style={{ padding: 14 }}>
                  <SectionLabel>Provider Breakdown</SectionLabel>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
                    <ProviderBar
                      name="claude"
                      providerColor={providers.claude}
                      cost="$248.60"
                      pct="79%"
                      frac={1}
                      delay={28}
                    />
                    <ProviderBar
                      name="codex"
                      providerColor={providers.codex}
                      cost="$63.80"
                      pct="21%"
                      frac={0.27}
                      delay={36}
                    />
                  </div>
                </Card>
              </Appear>
            </div>

            {/* ── right: today rollup + cpu/ram sparklines ── */}
            <div
              style={{
                flex: 1,
                display: 'flex',
                flexDirection: 'column',
                gap: 14,
                minWidth: 0,
              }}
            >
              <Appear delay={14} y={12}>
                <Card>
                  <SectionLabel>Today · Jun 26</SectionLabel>
                  <div style={{ display: 'flex', gap: 10 }}>
                    <MetricStat label="Sessions"  value="18"     style={{ flex: 1 }} />
                    <MetricStat
                      label="Tokens"
                      value="4.1M"
                      accent={series[0]}
                      style={{ flex: 1 }}
                    />
                    <MetricStat label="Cost"      value="$26.80" style={{ flex: 1 }} />
                    <MetricStat
                      label="Cache Hit"
                      value="78%"
                      accent={series[4]}
                      style={{ flex: 1 }}
                    />
                  </div>
                </Card>
              </Appear>

              <Appear delay={26} y={12} style={{ flex: 1 }}>
                <Card
                  style={{
                    height: '100%',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 8,
                  }}
                >
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      flexShrink: 0,
                    }}
                  >
                    <SectionLabel>CPU · last hour</SectionLabel>
                    <Chip>64% avg</Chip>
                  </div>
                  <div style={{ flex: 1 }}>
                    <Sparkline
                      data={CPU_DATA}
                      color={series[4]}
                      width={860}
                      height={72}
                      progress={sparkProgress}
                    />
                  </div>
                </Card>
              </Appear>

              <Appear delay={36} y={12} style={{ flex: 1 }}>
                <Card
                  style={{
                    height: '100%',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 8,
                  }}
                >
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      flexShrink: 0,
                    }}
                  >
                    <SectionLabel>RAM · last hour</SectionLabel>
                    <Chip>7.8 GB</Chip>
                  </div>
                  <div style={{ flex: 1 }}>
                    <Sparkline
                      data={RAM_DATA}
                      color={series[1]}
                      width={860}
                      height={72}
                      progress={sparkProgress}
                    />
                  </div>
                </Card>
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>

      <Caption
        step={2}
        title="Provider / day / session rollups · budgets · system metrics"
        sub="Per-provider spend · monthly budget ring · live CPU & RAM sparklines · retention TTL"
        delay={20}
      />
    </>
  );
};

// ── Scene 4 — Insights Report (~170f) ────────────────────────────────────────

const InsightsReportScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={<Navigator active="insights" />}
        title="Otto — Insights"
      >
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 14,
            padding: 18,
            height: '100%',
            boxSizing: 'border-box',
            overflow: 'hidden',
          }}
        >
          {/* ── page header ── */}
          <Appear delay={8} y={12}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
              }}
            >
              <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 22,
                    fontWeight: 700,
                    color: T.text,
                    letterSpacing: -0.4,
                  }}
                >
                  Catch-up Reports
                </span>
                <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                  Generated by Otto · multi-provider synthesis · action-first
                </span>
              </div>
              <Segmented options={['Daily', 'Weekly', 'Monthly']} active={0} />
            </div>
          </Appear>

          {/* ── report card ── */}
          <Appear delay={18} y={16} style={{ flex: 1 }}>
            <Card
              pad={20}
              style={{
                flex: 1,
                display: 'flex',
                flexDirection: 'column',
                gap: 16,
                background: `linear-gradient(160deg, ${alpha(brand.cyan, 0.04)} 0%, ${T.surface} 40%)`,
              }}
            >
              {/* report header */}
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  flexShrink: 0,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                  <div
                    style={{
                      width: 36,
                      height: 36,
                      borderRadius: 9,
                      background: alpha(brand.cyan, 0.13),
                      border: `1px solid ${alpha(brand.cyan, 0.3)}`,
                      display: 'grid',
                      placeItems: 'center',
                    }}
                  >
                    <Icon name="bell" size={17} color={brand.cyan} />
                  </div>
                  <div>
                    <div
                      style={{
                        fontFamily: fonts.ui,
                        fontSize: 17,
                        fontWeight: 700,
                        color: T.text,
                        letterSpacing: -0.2,
                      }}
                    >
                      Daily Catch-up
                    </div>
                    <div style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>
                      Thursday, Jun 26
                    </div>
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 8 }}>
                  <Chip tone="ok">Generated 6:00 AM</Chip>
                  <Chip color={providers.claude}>claude + codex</Chip>
                </div>
              </div>

              {/* summary line */}
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 13.5,
                  color: T.textDim,
                  paddingBottom: 14,
                  borderBottom: `1px solid ${T.border}`,
                  flexShrink: 0,
                }}
              >
                3 actions recommended · 12 sessions completed overnight · 1 agent needs your input
              </div>

              {/* action bullets */}
              <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
                {BULLETS.map((b, i) => (
                  <BulletRow
                    key={i}
                    icon={b.icon}
                    color={b.color}
                    text={b.text}
                    sub={b.sub}
                    delay={32 + i * 16}
                  />
                ))}
              </div>
            </Card>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>

    <Caption
      step={3}
      title="Scheduled catch-up reports"
      sub="Daily, weekly, monthly — or on demand · action-first HTML summaries · generated & cached"
      delay={20}
    />
  </>
);

// ── Composition ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  {
    dur: 75,
    node: <TitleScene />,
    name: 'Title',
  },
  {
    dur: 190,
    node: <UsageDashboardScene />,
    name: 'UsageDashboard',
  },
  {
    dur: 150,
    node: <BudgetsSystemScene />,
    name: 'BudgetsSystem',
  },
  {
    dur: 170,
    node: <InsightsReportScene />,
    name: 'InsightsReport',
  },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Usage, Cost & Insights"
        tagline="Know exactly what your agents cost — and what they did"
        pills={[
          { label: 'Token cost',       icon: 'chart', color: series[0] },
          { label: 'Cache breakdown',  icon: 'db',    color: series[4] },
          { label: 'Budgets',          icon: 'gauge', color: series[2] },
          { label: 'Catch-up reports', icon: 'bell',  color: brand.cyan },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const usageInsightsDuration = scenesDuration(SCENES);
export const UsageInsights: React.FC = () => <Scenes scenes={SCENES} />;
