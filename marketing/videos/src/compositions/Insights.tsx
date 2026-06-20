import React from 'react';
import {
  AbsoluteFill,
  Sequence,
  useCurrentFrame,
  useVideoConfig,
  interpolate,
  spring,
} from 'remotion';
import { theme } from '../theme';
import { OttoWindow } from '../components/OttoWindow';
import { Appear, Caption, TitleCard } from '../components/ui';

// ─── Insights — scheduled multi-provider catch-up reports — ~28s ─────────────
// Scenes: report list with filter pills → open a daily report (iframe-style
// preview) → run-now picker → scheduled delivery note.
// ─────────────────────────────────────────────────────────────────────────────

const TITLE_DUR  = 75;
const S1_DUR     = 180;  // report list + filter
const S2_DUR     = 180;  // report preview
const S3_DUR     = 120;  // run-now picker
const OUTRO_DUR  = 90;

const S1_START   = TITLE_DUR;
const S2_START   = S1_START + S1_DUR;
const S3_START   = S2_START + S2_DUR;
const OUTRO_START = S3_START + S3_DUR;

// ─── Data ─────────────────────────────────────────────────────────────────────
const KINDS = ['All', 'Daily', 'Weekly', 'Monthly', 'Ad-hoc'] as const;
type Kind = typeof KINDS[number];

const REPORTS = [
  { title: 'Daily Catch-Up',          kind: 'Daily',   date: 'Today, 06:00',      providers: ['claude', 'codex'],    snippetLines: ['12 sessions completed', '3 PRs merged', '2 Jira issues resolved'] },
  { title: 'Weekly Summary',          kind: 'Weekly',  date: 'Mon, 06:00',        providers: ['claude'],              snippetLines: ['47 sessions · 8 PRs · 12 reviews', '~38h of agent work', 'Top repo: sinatra-users-go'] },
  { title: 'Daily Catch-Up',          kind: 'Daily',   date: 'Yesterday, 06:00',  providers: ['claude', 'codex'],    snippetLines: ['9 sessions', '1 PR merged', '5 commits'] },
  { title: 'Monthly Rollup',          kind: 'Monthly', date: 'Jun 1, 06:00',      providers: ['claude'],              snippetLines: ['183 sessions · 42 PRs', '~160h of work', 'Cost: $24.80'] },
  { title: 'Ad-hoc: Sprint review',   kind: 'Ad-hoc', date: 'Jun 19, 14:32',     providers: ['claude'],              snippetLines: ['Custom period: Jun 10–19', '24 features shipped'] },
];

const PROVIDER_COLOR: Record<string, string> = {
  claude: theme.accent,
  codex:  '#10b981',
};

const KindBadge: React.FC<{ kind: Kind | string }> = ({ kind }) => {
  const COLOR: Record<string, string> = { Daily: theme.accent, Weekly: theme.accent2, Monthly: '#bf7aff', 'Ad-hoc': theme.warn };
  const c = COLOR[kind] ?? theme.textDim;
  return <span style={{ fontFamily: theme.mono, fontSize: 11, fontWeight: 700, color: c, background: `${c}22`, border: `1px solid ${c}44`, borderRadius: 6, padding: '2px 8px', letterSpacing: 0.4 }}>{kind}</span>;
};

// ─── Scene 1 – Report list with filter pills ──────────────────────────────────
const Scene1List: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const activeFilter: Kind = frame < 80 ? 'All' : 'Daily';

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      {/* header */}
      <Appear delay={4}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '22px 28px 0' }}>
          <div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 26, fontWeight: 800 }}>Insights</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 15, marginTop: 4 }}>Scheduled catch-up reports across all providers</div>
          </div>
          <div style={{ display: 'flex', gap: 10 }}>
            <div style={{ padding: '9px 20px', borderRadius: 10, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 14 }}>Schedule</div>
            <div style={{ padding: '9px 22px', borderRadius: 10, background: theme.accent, color: '#fff', fontFamily: theme.font, fontSize: 14, fontWeight: 700, boxShadow: `0 6px 20px ${theme.accent}44` }}>▶ Run now</div>
          </div>
        </div>
      </Appear>

      {/* filter pills */}
      <Appear delay={14}>
        <div style={{ display: 'flex', gap: 8, padding: '18px 28px 0' }}>
          {KINDS.map((k) => {
            const isActive = k === activeFilter;
            const COLOR: Record<string, string> = { Daily: theme.accent, Weekly: theme.accent2, Monthly: '#bf7aff', 'Ad-hoc': theme.warn, All: theme.textDim };
            const c = isActive ? (COLOR[k] ?? theme.textDim) : theme.textDim;
            return (
              <div key={k} style={{ padding: '7px 16px', borderRadius: 20, background: isActive ? `${c}18` : 'transparent', border: `1px solid ${isActive ? c : theme.border}`, color: c, fontFamily: theme.font, fontSize: 13, fontWeight: isActive ? 700 : 400 }}>{k}</div>
            );
          })}
        </div>
      </Appear>

      {/* report cards */}
      <div style={{ flex: 1, overflow: 'hidden', padding: '16px 28px 28px', display: 'flex', flexDirection: 'column', gap: 12 }}>
        {REPORTS.map((r, i) => {
          const s = spring({ frame: frame - (i * 12 + 28), fps, config: { damping: 200 } });
          return (
            <div key={i} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [14, 0])}px)`, background: theme.surface2, borderRadius: 14, border: `1px solid ${theme.border}`, padding: '16px 22px', display: 'flex', alignItems: 'center', gap: 20 }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 6 }}>
                  <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 16, fontWeight: 700 }}>{r.title}</span>
                  <KindBadge kind={r.kind} />
                  <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12, marginLeft: 4 }}>{r.date}</span>
                </div>
                <div style={{ display: 'flex', gap: 16 }}>
                  {r.snippetLines.map((line, j) => (
                    <span key={j} style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>{line}</span>
                  ))}
                </div>
              </div>
              <div style={{ display: 'flex', gap: 6 }}>
                {r.providers.map((p) => (
                  <div key={p} style={{ width: 8, height: 8, borderRadius: '50%', background: PROVIDER_COLOR[p] ?? theme.textDim }} />
                ))}
              </div>
              <div style={{ padding: '7px 16px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>Open</div>
            </div>
          );
        })}
      </div>

      <Caption step={1} title="Catch-up reports" sub="Daily, weekly, monthly, or on-demand — filter by kind" delay={55} />
    </div>
  );
};

// ─── Scene 2 – Report preview (iframe-style) ──────────────────────────────────
const ReportPreview: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const sections = [
    { icon: '📊', heading: 'Activity summary', body: '12 agent sessions · 3 PRs merged · 2 Jira issues resolved · 48 commits pushed' },
    { icon: '🤖', heading: 'Claude sessions', body: 'feat/rbac-multiuser: 4h 12m · sinatra-users-go: 2h 08m · otto_os: 1h 45m' },
    { icon: '🔁', heading: 'Codex sessions', body: 'go-casino-kit: 55m · go_utilities: 38m' },
    { icon: '💡', heading: 'Key learnings', body: 'SSRF guard on broker fetches. PTY daemon PATH expanded to include ~/go/bin.' },
  ];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      {/* report titlebar */}
      <div style={{ padding: '16px 28px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 12, flexShrink: 0 }}>
        <KindBadge kind="Daily" />
        <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 16, fontWeight: 700 }}>Daily Catch-Up — Today, 06:00</span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          <div style={{ padding: '6px 14px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>← Back</div>
        </div>
      </div>

      {/* scrollable body */}
      <div style={{ flex: 1, overflow: 'hidden', padding: '24px 40px', display: 'flex', flexDirection: 'column', gap: 22 }}>
        {sections.map((sec, i) => {
          const s = spring({ frame: frame - i * 14, fps, config: { damping: 200 } });
          return (
            <div key={sec.heading} style={{ opacity: s, transform: `translateY(${interpolate(s, [0, 1], [14, 0])}px)`, background: theme.surface2, borderRadius: 14, padding: '18px 22px', border: `1px solid ${theme.border}` }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 8 }}>
                <span style={{ fontSize: 20 }}>{sec.icon}</span>
                <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 16, fontWeight: 700 }}>{sec.heading}</span>
              </div>
              <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14, lineHeight: 1.6 }}>{sec.body}</div>
            </div>
          );
        })}
      </div>

      <Caption step={2} title="Rich HTML reports" sub="Session breakdown, learnings, and key events — in one place" delay={55} />
    </div>
  );
};

// ─── Scene 3 – Run-now period picker ──────────────────────────────────────────
const Scene3RunNow: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const modalS = spring({ frame, fps, config: { damping: 180 } });
  const PERIODS = [
    { id: 'day',   label: 'Last 24h',   desc: 'Activity since yesterday 06:00' },
    { id: 'week',  label: 'Last 7 days', desc: 'Current sprint summary' },
    { id: 'month', label: 'This month',  desc: 'Monthly rollup' },
  ];
  const active = frame < 60 ? 'day' : 'week';

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ opacity: modalS, transform: `scale(${interpolate(modalS, [0, 1], [0.9, 1])})`, width: 560, background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: 18, boxShadow: '0 40px 100px rgba(0,0,0,0.7)', padding: '32px 36px' }}>
        <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 20, fontWeight: 800, marginBottom: 6 }}>Run report now</div>
        <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14, marginBottom: 24 }}>Choose the time window to cover</div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {PERIODS.map((p, i) => {
            const s = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
            const isActive = p.id === active;
            return (
              <div key={p.id} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [14, 0])}px)`, padding: '14px 18px', borderRadius: 12, background: isActive ? `${theme.accent}14` : theme.surface2, border: `1px solid ${isActive ? theme.accent : theme.border}`, display: 'flex', alignItems: 'center', gap: 16 }}>
                <div style={{ width: 16, height: 16, borderRadius: '50%', border: `2px solid ${isActive ? theme.accent : theme.border}`, display: 'grid', placeItems: 'center', flexShrink: 0 }}>
                  {isActive && <div style={{ width: 8, height: 8, borderRadius: '50%', background: theme.accent }} />}
                </div>
                <div>
                  <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 15, fontWeight: isActive ? 700 : 400 }}>{p.label}</div>
                  <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 2 }}>{p.desc}</div>
                </div>
              </div>
            );
          })}
        </div>

        <div style={{ display: 'flex', gap: 12, marginTop: 28, justifyContent: 'flex-end' }}>
          <div style={{ padding: '10px 22px', border: `1px solid ${theme.border}`, borderRadius: 10, color: theme.textDim, fontFamily: theme.font, fontSize: 15, fontWeight: 600 }}>Cancel</div>
          <div style={{ padding: '10px 28px', background: theme.accent, borderRadius: 10, color: '#fff', fontFamily: theme.font, fontSize: 15, fontWeight: 700, boxShadow: `0 6px 20px ${theme.accent}44` }}>Generate</div>
        </div>
      </div>

      <Caption step={3} title="Run on demand" sub="Any window — last 24h, sprint, or full month" delay={40} />
    </div>
  );
};

// ─── Outro ─────────────────────────────────────────────────────────────────────
const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t1 = spring({ frame,              fps, config: { damping: 160 } });
  const t2 = spring({ frame: frame - 18, fps, config: { damping: 160 } });
  const t3 = spring({ frame: frame - 32, fps, config: { damping: 160 } });

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 12 }}>
      <div style={{ opacity: t1, transform: `scale(${interpolate(t1, [0, 1], [0.5, 1])})`, fontSize: 80 }}>📊</div>
      <div style={{ opacity: t2, transform: `translateY(${interpolate(t2, [0, 1], [24, 0])}px)`, color: theme.text, fontFamily: theme.font, fontSize: 64, fontWeight: 800, textAlign: 'center' }}>
        Always in the loop.
      </div>
      <div style={{ opacity: t3, transform: `translateY(${interpolate(t3, [0, 1], [16, 0])}px)`, color: theme.textDim, fontFamily: theme.font, fontSize: 24, textAlign: 'center' }}>
        Daily · Weekly · Monthly · Ad-hoc — every provider, one report
      </div>
    </div>
  );
};

// ─── Root composition ─────────────────────────────────────────────────────────
export const Insights: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>

      <Sequence durationInFrames={TITLE_DUR}>
        <TitleCard kicker="OTTO ADE" title="Insights" subtitle="Scheduled catch-up reports, automatically" />
      </Sequence>

      <Sequence from={S1_START} durationInFrames={S1_DUR + S2_DUR + S3_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Insights">
            <Sequence durationInFrames={S1_DUR}>
              <Scene1List />
            </Sequence>
            <Sequence from={S1_DUR} durationInFrames={S2_DUR}>
              <ReportPreview />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR} durationInFrames={S3_DUR}>
              <Scene3RunNow />
            </Sequence>
          </OttoWindow>
        </AbsoluteFill>
      </Sequence>

      <Sequence from={OUTRO_START} durationInFrames={OUTRO_DUR}>
        <Outro />
      </Sequence>

    </AbsoluteFill>
  );
};
