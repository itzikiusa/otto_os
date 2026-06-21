import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, providers, status as STATUS } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Chip,
  Button,
  Card,
  MetricStat,
  BarChart,
  Sparkline,
  Ring,
  Segmented,
  Toggle,
  StatusDot,
  Icon,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  USAGE · BUDGETS · INSIGHTS — Otto tails Claude & Codex transcripts into an
//  embedded ClickHouse store: token + cost rollups, budget guardrails, daemon
//  health metrics, and scheduled multi-provider catch-up reports.
// ════════════════════════════════════════════════════════════════════════════

// ── Scene 1 — title card ─────────────────────────────────────────────────────
const Title: React.FC = () => (
  <TitleCard
    kicker="Usage · Budgets · Insights"
    title="Know exactly what your agents cost"
    subtitle="Tokens & spend, tracked automatically — with caps that hold"
  />
);

// ── small building blocks ────────────────────────────────────────────────────

// A per-provider spend row: chip + token count + dollars + a proportional bar.
const ProviderRow: React.FC<{
  name: string;
  color: string;
  tokens: string;
  cost: string;
  frac: number; // 0–1 share of the bar track
  grow: number; // 0–1 animation
  delay: number;
}> = ({ name, color, tokens, cost, frac, grow, delay }) => (
  <Appear delay={delay} y={10}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
      <Chip color={color} style={{ height: 22, minWidth: 78, justifyContent: 'center' }}>
        {name}
      </Chip>
      <div style={{ flex: 1, height: 9, borderRadius: 999, background: T.surface2, overflow: 'hidden' }}>
        <div
          style={{
            width: `${frac * grow * 100}%`,
            height: '100%',
            borderRadius: 999,
            background: `linear-gradient(90deg, ${alpha(color, 0.7)}, ${color})`,
          }}
        />
      </div>
      <span style={{ fontFamily: fonts.mono, fontSize: 13, color: T.textDim, minWidth: 78, textAlign: 'right' }}>
        {tokens}
      </span>
      <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text, minWidth: 78, textAlign: 'right' }}>
        {cost}
      </span>
    </div>
  </Appear>
);

// Section header used inside content cards/panels.
const PanelHead: React.FC<{ icon: string; title: string; color?: string; right?: React.ReactNode }> = ({
  icon,
  title,
  color = brand.cyan,
  right,
}) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 14 }}>
    <Icon name={icon} size={15} color={color} />
    <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>{title}</span>
    <div style={{ flex: 1 }} />
    {right}
  </div>
);

// ── Scene 2 — usage dashboard ────────────────────────────────────────────────
const UsageDashScene: React.FC = () => {
  const frame = useCurrentFrame();
  const grow = track(frame, [40, 78], [0, 1]); // bars + provider bars draw in
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="usage" />}
          tabs={[{ label: 'Usage · last 30 days', icon: 'chart', active: true }]}
          title="Otto — Usage · embedded ClickHouse"
        >
          <div style={{ height: '100%', padding: 20, display: 'flex', flexDirection: 'column', gap: 16, minHeight: 0 }}>
            {/* header strip */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <Icon name="chart" size={16} color={brand.cyan} />
              <span style={{ fontFamily: fonts.ui, fontSize: 17, fontWeight: 700, color: T.text }}>
                Usage — last 30 days
              </span>
              <Chip color="#febc2e" style={{ height: 22 }}>
                ClickHouse
              </Chip>
              <div style={{ flex: 1 }} />
              <Chip tone="ok" style={{ height: 22 }}>
                <StatusDot kind="working" size={7} />
                tailing transcripts
              </Chip>
              <Segmented options={['Day', 'Session', 'Feature']} active={0} />
            </div>

            {/* top metric row */}
            <div style={{ display: 'flex', gap: 14 }}>
              <Appear delay={6} y={14} style={{ flex: 1 }}>
                <MetricStat label="Spend · 30d" value="$284.10" delta="▲ 11% vs prev 30d" deltaTone="bad" style={{ minWidth: 0 }} accent={T.text} />
              </Appear>
              <Appear delay={11} y={14} style={{ flex: 1 }}>
                <MetricStat label="Tokens" value="412M" delta="in · out · cache" deltaTone="ok" style={{ minWidth: 0 }} accent={T.text} />
              </Appear>
              <Appear delay={16} y={14} style={{ flex: 1 }}>
                <MetricStat label="Sessions" value="1,203" delta="▲ 142 this week" deltaTone="ok" style={{ minWidth: 0 }} accent={T.text} />
              </Appear>
              <Appear delay={21} y={14} style={{ flex: 1 }}>
                <MetricStat label="Cache hit" value="71%" delta="cache-read saves $$" deltaTone="ok" style={{ minWidth: 0 }} accent={brand.cyan} />
              </Appear>
            </div>

            {/* charts row: daily spend bars + per-provider breakdown */}
            <div style={{ flex: 1, minHeight: 0, display: 'flex', gap: 14 }}>
              {/* daily spend */}
              <Appear delay={28} y={16} scale={0.97} style={{ flex: 1.55 }}>
                <Card pad={0} style={{ height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
                  <div style={{ padding: '12px 16px 0' }}>
                    <PanelHead
                      icon="chart"
                      title="Daily spend"
                      right={<span style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim }}>$9.47 / day avg</span>}
                    />
                  </div>
                  <div style={{ flex: 1, minHeight: 0, padding: '0 16px 14px', display: 'flex' }}>
                    <BarChart
                      data={[6, 9, 7, 11, 8, 13, 10, 12, 9, 15, 11, 14, 8, 12, 16]}
                      labels={['', '', '5', '', '', '10', '', '', '15', '', '', '20', '', '', '30']}
                      color={brand.cyan}
                      grow={grow}
                      height={172}
                    />
                  </div>
                </Card>
              </Appear>

              {/* per-provider breakdown */}
              <Appear delay={34} y={16} scale={0.97} style={{ flex: 1 }}>
                <Card style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
                  <PanelHead icon="grid" title="By provider" color={brand.violet} />
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 16, marginTop: 4 }}>
                    <ProviderRow name="claude" color={providers.claude} tokens="268M tok" cost="$196.40" frac={1} grow={grow} delay={42} />
                    <ProviderRow name="codex" color={providers.codex} tokens="144M tok" cost="$87.70" frac={0.46} grow={grow} delay={48} />
                  </div>
                  <div style={{ flex: 1 }} />
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 8,
                      paddingTop: 14,
                      borderTop: `1px solid ${T.border}`,
                      fontFamily: fonts.ui,
                      fontSize: 12,
                      color: T.textDim,
                    }}
                  >
                    <Icon name="info" size={12} color={T.textDim} />
                    input · output · cache-read · cache-write tracked per call
                  </div>
                </Card>
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={1}
        title="Every token & dollar, by provider, day & session"
        sub="Tailed straight from transcripts — zero instrumentation"
      />
    </>
  );
};

// ── Scene 3 — budgets ────────────────────────────────────────────────────────
const BudgetBar: React.FC<{
  label: string;
  spend: string;
  cap: string;
  frac: number; // 0–1 of cap
  color: string;
  grow: number;
  delay: number;
  chip?: React.ReactNode;
}> = ({ label, spend, cap, frac, color, grow, delay, chip }) => (
  <Appear delay={delay} y={12}>
    <div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
        <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 600, color: T.text }}>{label}</span>
        {chip}
        <div style={{ flex: 1 }} />
        <span style={{ fontFamily: fonts.mono, fontSize: 13.5, color: T.text }}>
          <span style={{ color, fontWeight: 700 }}>{spend}</span>
          <span style={{ color: T.textDim }}> / {cap}</span>
        </span>
      </div>
      <div style={{ position: 'relative', height: 14, borderRadius: 999, background: T.surface2, overflow: 'hidden' }}>
        {/* 80% warn tick */}
        <div style={{ position: 'absolute', left: '80%', top: 0, bottom: 0, width: 2, background: alpha(STATUS.needsYou, 0.7), zIndex: 2 }} />
        <div
          style={{
            width: `${Math.min(1, frac) * grow * 100}%`,
            height: '100%',
            borderRadius: 999,
            background: `linear-gradient(90deg, ${alpha(color, 0.65)}, ${color})`,
          }}
        />
      </div>
    </div>
  </Appear>
);

const BudgetsScene: React.FC = () => {
  const frame = useCurrentFrame();
  const grow = track(frame, [34, 70], [0, 1]);
  const ringV = track(frame, [40, 78], [0, 0.57]); // workspace ring fills to 57%
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="usage" />}
          tabs={[{ label: 'Budgets', icon: 'gauge', active: true }]}
          title="Otto — Usage · Budgets"
        >
          <div style={{ height: '100%', padding: 20, display: 'flex', gap: 16, minHeight: 0 }}>
            {/* left — workspace cap ring */}
            <Appear delay={6} y={16} scale={0.97} style={{ width: 360, flexShrink: 0 }}>
              <Card style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
                <PanelHead icon="gauge" title="Workspace budget" />
                <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 18 }}>
                  <Ring value={ringV} size={188} color={brand.cyan} label="57%" />
                  <div style={{ textAlign: 'center' }}>
                    <div style={{ fontFamily: fonts.ui, fontSize: 22, fontWeight: 800, color: T.text }}>
                      $284.10 <span style={{ color: T.textDim, fontWeight: 600, fontSize: 18 }}>/ $500</span>
                    </div>
                    <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginTop: 4 }}>
                      this month · resets in 9 days
                    </div>
                  </div>
                </div>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    paddingTop: 14,
                    borderTop: `1px solid ${T.border}`,
                  }}
                >
                  <Toggle on />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 600, color: T.text }}>Block on exceed</span>
                  <div style={{ flex: 1 }} />
                  <Chip style={{ height: 22 }}>opt-in</Chip>
                </div>
              </Card>
            </Appear>

            {/* right — per-provider caps with warn / block states */}
            <Appear delay={12} y={16} scale={0.97} style={{ flex: 1 }}>
              <Card style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
                <PanelHead
                  icon="grid"
                  title="Provider caps"
                  color={brand.violet}
                  right={
                    <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>
                      warn at 80% · block at 100%
                    </span>
                  }
                />
                <div style={{ display: 'flex', flexDirection: 'column', gap: 26, marginTop: 8 }}>
                  <BudgetBar
                    label="claude"
                    spend="$196.40"
                    cap="$250"
                    frac={0.79}
                    color={STATUS.needsYou}
                    grow={grow}
                    delay={20}
                    chip={
                      <Chip tone="warn" style={{ height: 20 }}>
                        <Icon name="info" size={11} color={STATUS.needsYou} />
                        warning · 79%
                      </Chip>
                    }
                  />
                  <BudgetBar
                    label="codex"
                    spend="$87.70"
                    cap="$150"
                    frac={0.58}
                    color={STATUS.working}
                    grow={grow}
                    delay={28}
                    chip={
                      <Chip tone="ok" style={{ height: 20 }}>
                        healthy · 58%
                      </Chip>
                    }
                  />
                  <BudgetBar
                    label="gemini · sandbox"
                    spend="$60"
                    cap="$60"
                    frac={1}
                    color={STATUS.exited}
                    grow={grow}
                    delay={36}
                    chip={
                      <Chip tone="bad" style={{ height: 20 }}>
                        <Icon name="x" size={11} color={STATUS.exited} />
                        blocked · 100%
                      </Chip>
                    }
                  />
                </div>
                <div style={{ flex: 1 }} />
                <Appear delay={48} y={10}>
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 11,
                      padding: '12px 14px',
                      borderRadius: 8,
                      background: alpha(STATUS.exited, 0.1),
                      border: `1px solid ${alpha(STATUS.exited, 0.4)}`,
                    }}
                  >
                    <Icon name="info" size={15} color={STATUS.exited} />
                    <span style={{ fontFamily: fonts.ui, fontSize: 13.5, color: T.text }}>
                      <b>gemini · sandbox</b> hit its $60 cap — new runs are blocked until next month.
                    </span>
                    <div style={{ flex: 1 }} />
                    <Button size="s">Raise cap</Button>
                  </div>
                </Appear>
              </Card>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={2}
        title="Set caps — warn at 80%, block at 100%"
        sub="Per workspace & per provider · enforcement is opt-in"
      />
    </>
  );
};

// ── Scene 4 — metrics + scheduled insights ───────────────────────────────────
const InsightRow: React.FC<{ icon: string; color: string; text: string; delay: number }> = ({
  icon,
  color,
  text,
  delay,
}) => (
  <Appear delay={delay} y={10}>
    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 11 }}>
      <span
        style={{
          width: 26,
          height: 26,
          borderRadius: 8,
          flexShrink: 0,
          marginTop: 1,
          background: alpha(color, 0.16),
          border: `1px solid ${alpha(color, 0.4)}`,
          display: 'grid',
          placeItems: 'center',
        }}
      >
        <Icon name={icon} size={14} color={color} />
      </span>
      <span style={{ fontFamily: fonts.ui, fontSize: 14, lineHeight: 1.45, color: T.text }}>{text}</span>
    </div>
  </Appear>
);

const MetricsInsightsScene: React.FC = () => {
  const frame = useCurrentFrame();
  const cpu = track(frame, [34, 84], [0, 1]); // cpu sparkline draws in
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="insights" />}
          tabs={[{ label: 'Health & Insights', icon: 'gauge', active: true, dot: 'working' }]}
          title="Otto — Insights · daemon metrics & reports"
        >
          <div style={{ height: '100%', padding: 20, display: 'flex', gap: 16, minHeight: 0 }}>
            {/* left — daemon health */}
            <Appear delay={6} y={16} scale={0.97} style={{ width: 460, flexShrink: 0 }}>
              <Card style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
                <PanelHead
                  icon="gauge"
                  title="Daemon health"
                  right={
                    <Chip tone="ok" style={{ height: 20 }}>
                      <StatusDot kind="working" size={7} />
                      ottod up · 6d
                    </Chip>
                  }
                />
                <div style={{ display: 'flex', gap: 12, marginBottom: 16 }}>
                  <MetricStat label="RAM" value="412 MB" delta="steady" deltaTone="ok" style={{ flex: 1, minWidth: 0 }} accent={T.text} />
                  <MetricStat label="Load avg" value="1.24" delta="8 cores" deltaTone="ok" style={{ flex: 1, minWidth: 0 }} accent={T.text} />
                </div>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    marginBottom: 8,
                    fontFamily: fonts.ui,
                    fontSize: 12.5,
                    color: T.textDim,
                  }}
                >
                  <span>CPU · last 60 min</span>
                  <span style={{ fontFamily: fonts.mono, color: STATUS.working }}>peak 38%</span>
                </div>
                <div style={{ flex: 1, minHeight: 0, display: 'flex', alignItems: 'flex-end' }}>
                  <Sparkline
                    data={[8, 11, 9, 14, 12, 22, 18, 16, 27, 21, 19, 31, 24, 20, 38, 26, 17, 22, 15, 12]}
                    color={STATUS.working}
                    width={412}
                    height={150}
                    progress={cpu}
                  />
                </div>
              </Card>
            </Appear>

            {/* right — scheduled insights report */}
            <Appear delay={12} y={16} scale={0.97} style={{ flex: 1 }}>
              <Card pad={0} style={{ height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
                {/* report header */}
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 11,
                    padding: '14px 18px',
                    borderBottom: `1px solid ${T.border}`,
                    background: alpha(brand.purple, 0.08),
                  }}
                >
                  <Icon name="note" size={16} color={brand.cyan} />
                  <div style={{ flex: 1 }}>
                    <div style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>
                      Weekly catch-up
                    </div>
                    <div style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>
                      Jun 15 – Jun 21 · claude + codex · written for you
                    </div>
                  </div>
                  <Segmented options={['Daily', 'Weekly', 'Monthly']} active={1} />
                  <Button variant="primary" icon="play" size="s">
                    Run now
                  </Button>
                </div>

                {/* action-first bullets */}
                <div style={{ flex: 1, padding: '18px 20px', display: 'flex', flexDirection: 'column', gap: 18, minHeight: 0 }}>
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 700, letterSpacing: 0.5, textTransform: 'uppercase', color: T.textDim }}>
                    Do this next
                  </span>
                  <InsightRow
                    color={STATUS.needsYou}
                    icon="zap"
                    text="claude spend is up 23% — the auth-refactor sessions retry the test suite 4×. Cache the fixtures to cut ~$40/wk."
                    delay={24}
                  />
                  <InsightRow
                    color={brand.cyan}
                    icon="branch"
                    text="3 sessions on sinatra-users-go stalled needing review — they are 81% of idle token burn this week."
                    delay={32}
                  />
                  <InsightRow
                    color={STATUS.working}
                    icon="check"
                    text="codex stayed 42% under cap; cache-hit climbed to 71%. Consider shifting refactors to codex."
                    delay={40}
                  />
                </div>

                {/* footer schedule */}
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 9,
                    padding: '12px 18px',
                    borderTop: `1px solid ${T.border}`,
                    fontFamily: fonts.ui,
                    fontSize: 12.5,
                    color: T.textDim,
                  }}
                >
                  <Icon name="clock" size={13} color={T.textDim} />
                  Scheduled — every Monday 09:00 · delivered to Slack
                  <div style={{ flex: 1 }} />
                  <Toggle on />
                </div>
              </Card>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={3}
        title="Health metrics + scheduled insight reports"
        sub="Daily / weekly / monthly catch-ups, written for you"
      />
    </>
  );
};

// ── Scenes ────────────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <Title />, name: 'Title' },
  { dur: 230, node: <UsageDashScene />, name: 'Usage dashboard' },
  { dur: 210, node: <BudgetsScene />, name: 'Budgets' },
  { dur: 190, node: <MetricsInsightsScene />, name: 'Metrics + Insights' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Usage & Insights"
        tagline="Cost you can see — and control."
        pills={[
          { label: 'Token tracking', color: '#0a84ff', icon: 'chart' },
          { label: 'Per-feature spend', color: brand.cyan, icon: 'gauge' },
          { label: 'Budget caps', color: '#febc2e', icon: 'gauge' },
          { label: 'CPU/RAM', color: '#28c840', icon: 'gauge' },
          { label: 'Scheduled reports', color: brand.violet, icon: 'clock' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const usageInsightsDuration = scenesDuration(SCENES);
export const UsageInsights: React.FC = () => <Scenes scenes={SCENES} />;
