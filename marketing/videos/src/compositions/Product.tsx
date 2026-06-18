import React from 'react';
import {
  AbsoluteFill,
  Sequence,
  useCurrentFrame,
  useVideoConfig,
  interpolate,
  spring,
  staticFile,
  Img,
} from 'remotion';
import { theme } from '../theme';
import { OttoWindow } from '../components/OttoWindow';
import { Appear, Caption, Cursor, TitleCard } from '../components/ui';

// ─────────────────────────────────────────────────────────────────────────────
// Product — a ~60s walkthrough of Otto's Product story workflow:
//   raw Jira/Confluence ticket → AI analysis → questions → rewrite →
//   notes / test cases / plan / history → inject into a coding agent → learnings.
// All data below is illustrative (mocked), same approach as the other comps.
// ─────────────────────────────────────────────────────────────────────────────

const WIN = { width: 1440, height: 820 } as const;

// Provider accent colors (claude / codex / gemini).
const PROV = {
  claude: theme.accent,
  codex: '#bf7aff',
  gemini: theme.accent2,
} as const;

// ─── Shared helpers ───────────────────────────────────────────────────────────

const FadeIn: React.FC<{ children: React.ReactNode; durationFrames?: number }> = ({
  children,
  durationFrames = 16,
}) => {
  const frame = useCurrentFrame();
  const opacity = interpolate(frame, [0, durationFrames], [0, 1], {
    extrapolateRight: 'clamp',
    extrapolateLeft: 'clamp',
  });
  return <div style={{ opacity, width: '100%', height: '100%' }}>{children}</div>;
};

const SectionLabel: React.FC<{ children: React.ReactNode; style?: React.CSSProperties }> = ({
  children,
  style,
}) => (
  <div
    style={{
      fontFamily: theme.font,
      fontSize: 11,
      fontWeight: 700,
      letterSpacing: 1,
      textTransform: 'uppercase' as const,
      color: theme.textDim,
      ...style,
    }}
  >
    {children}
  </div>
);

const StageBadge: React.FC<{ stage: string }> = ({ stage }) => {
  const map: Record<string, string> = {
    draft: theme.textDim,
    review: theme.warn,
    approved: theme.working,
    done: theme.accent,
  };
  const c = map[stage] ?? theme.textDim;
  return (
    <span
      style={{
        fontFamily: theme.font,
        fontSize: 9.5,
        fontWeight: 700,
        textTransform: 'uppercase' as const,
        letterSpacing: 0.6,
        padding: '1px 7px',
        borderRadius: 999,
        background: `${c}22`,
        color: c,
      }}
    >
      {stage}
    </span>
  );
};

// ─── Stories sidebar (left rail of the Product page) ──────────────────────────

const STORIES = [
  { key: 'SIN-4821', title: 'Multi-currency wallet for VIP players', kind: 'jira', stage: 'review', tags: ['wallet', 'vip'] },
  { key: 'SIN-4790', title: 'Bonus eligibility rules engine', kind: 'jira', stage: 'approved', tags: ['bonus'] },
  { key: 'CONF-212', title: 'Q3 Payments — discovery RFC', kind: 'confluence', stage: 'draft', tags: ['payments'] },
  { key: 'DRAFT', title: 'Loyalty tier revamp', kind: 'draft', stage: 'draft', tags: [] },
];

const kindIcon = (k: string) => (k === 'jira' ? '🎫' : k === 'confluence' ? '🌐' : '📄');

const SideBtn: React.FC<{ icon: string; label: string; primary?: boolean; glow?: boolean }> = ({
  icon,
  label,
  primary,
  glow,
}) => (
  <div
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 4,
      padding: '3px 9px',
      borderRadius: 6,
      border: `1px solid ${primary ? theme.accent : theme.border}`,
      background: primary ? theme.accent : 'transparent',
      color: primary ? '#fff' : theme.text,
      fontFamily: theme.font,
      fontSize: 11.5,
      fontWeight: 600,
      whiteSpace: 'nowrap' as const,
      boxShadow: glow ? `0 0 0 3px ${theme.accent}44, 0 6px 18px ${theme.accent}55` : 'none',
    }}
  >
    <span style={{ fontSize: 11 }}>{icon}</span>
    {label}
  </div>
);

const StoriesSidebar: React.FC<{ selected?: string; importGlow?: boolean }> = ({
  selected = 'SIN-4821',
  importGlow,
}) => (
  <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
    {/* header */}
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '12px 12px 6px' }}>
      <SectionLabel>Stories</SectionLabel>
      <div style={{ display: 'flex', gap: 6 }}>
        <SideBtn icon="＋" label="New draft" />
        <SideBtn icon="＋" label="Import" primary glow={importGlow} />
      </div>
    </div>

    {/* tag filter row */}
    <div style={{ display: 'flex', gap: 4, padding: '2px 12px 8px', borderBottom: `1px solid ${theme.border}`, flexWrap: 'wrap' as const }}>
      {['All', 'wallet', 'vip', 'bonus', 'payments'].map((t, i) => (
        <span
          key={t}
          style={{
            fontFamily: theme.font,
            fontSize: 9.5,
            padding: '1px 7px',
            borderRadius: 999,
            border: `1px solid ${i === 0 ? theme.accent : theme.border}`,
            color: i === 0 ? theme.accent : theme.textDim,
            background: i === 0 ? `${theme.accent}18` : 'transparent',
          }}
        >
          {t}
        </span>
      ))}
    </div>

    {/* story rows */}
    <div style={{ flex: 1, padding: '6px 8px', display: 'flex', flexDirection: 'column', gap: 2, overflow: 'hidden' as const }}>
      {STORIES.map((s) => {
        const active = s.key === selected;
        return (
          <div
            key={s.key}
            style={{
              display: 'flex',
              alignItems: 'flex-start',
              gap: 8,
              padding: '7px 8px',
              borderRadius: 7,
              background: active ? `${theme.accent}1f` : 'transparent',
            }}
          >
            <span style={{ fontSize: 13, marginTop: 1 }}>{kindIcon(s.kind)}</span>
            <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 3 }}>
              <span
                style={{
                  fontFamily: theme.font,
                  fontSize: 12.5,
                  fontWeight: 500,
                  color: active ? theme.accent : theme.text,
                  overflow: 'hidden' as const,
                  textOverflow: 'ellipsis' as const,
                  whiteSpace: 'nowrap' as const,
                }}
              >
                {s.title}
              </span>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <StageBadge stage={s.stage} />
                {s.kind === 'draft' ? (
                  <span style={{ fontFamily: theme.font, fontSize: 8.5, fontWeight: 700, letterSpacing: 0.6, padding: '1px 5px', borderRadius: 999, background: `${theme.accent}22`, color: theme.accent }}>
                    DRAFT
                  </span>
                ) : (
                  <span style={{ fontFamily: theme.mono, fontSize: 10, color: theme.textDim }}>{s.key}</span>
                )}
              </div>
              {s.tags.length > 0 && (
                <div style={{ display: 'flex', gap: 3, flexWrap: 'wrap' as const }}>
                  {s.tags.map((t) => (
                    <span key={t} style={{ fontFamily: theme.font, fontSize: 8.5, padding: '1px 5px', borderRadius: 999, background: `${theme.accent}14`, color: theme.accent }}>
                      {t}
                    </span>
                  ))}
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  </div>
);

const LearningsSidebar: React.FC<{ filter?: string }> = ({ filter = 'all' }) => (
  <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
    <div style={{ padding: '12px 12px 6px' }}>
      <SectionLabel>Learnings</SectionLabel>
    </div>
    <div style={{ padding: '6px 8px', display: 'flex', flexDirection: 'column', gap: 2 }}>
      {[
        { v: 'all', label: 'All' },
        { v: 'pattern', label: 'Patterns to follow' },
        { v: 'avoid', label: 'Cases to avoid' },
      ].map((o) => {
        const active = o.v === filter;
        return (
          <div
            key={o.v}
            style={{
              padding: '7px 10px',
              borderRadius: 7,
              fontFamily: theme.font,
              fontSize: 12.5,
              fontWeight: active ? 600 : 500,
              color: active ? theme.accent : theme.textDim,
              background: active ? `${theme.accent}18` : 'transparent',
            }}
          >
            {o.label}
          </div>
        );
      })}
    </div>
  </div>
);

// ─── Header rows: view toggle + tab strip ─────────────────────────────────────

const TABS = ['Overview', 'Analysis', 'Questions', 'Notes', 'Rewrite', 'Test Cases', 'Plan', 'History', 'Inject'];

const ViewToggle: React.FC<{ active: 'stories' | 'learnings' }> = ({ active }) => (
  <div style={{ display: 'flex', gap: 2, padding: '8px 14px 0', borderBottom: `1px solid ${theme.border}` }}>
    {(['stories', 'learnings'] as const).map((v) => (
      <div
        key={v}
        style={{
          height: 30,
          padding: '0 12px',
          display: 'flex',
          alignItems: 'center',
          fontFamily: theme.font,
          fontSize: 12.5,
          fontWeight: 500,
          color: active === v ? theme.accent : theme.textDim,
          borderBottom: `2px solid ${active === v ? theme.accent : 'transparent'}`,
          marginBottom: -1,
          textTransform: 'capitalize' as const,
        }}
      >
        {v}
      </div>
    ))}
  </div>
);

const TabStrip: React.FC<{ active: string }> = ({ active }) => (
  <div style={{ display: 'flex', gap: 1, padding: '0 14px', borderBottom: `1px solid ${theme.border}`, overflow: 'hidden' as const }}>
    {TABS.map((t) => {
      const on = t === active;
      return (
        <div
          key={t}
          style={{
            height: 30,
            padding: '0 11px',
            display: 'flex',
            alignItems: 'center',
            fontFamily: theme.font,
            fontSize: 12,
            fontWeight: 500,
            color: on ? theme.accent : theme.textDim,
            borderBottom: `2px solid ${on ? theme.accent : 'transparent'}`,
            marginBottom: -1,
            whiteSpace: 'nowrap' as const,
            flexShrink: 0,
          }}
        >
          {t}
        </div>
      );
    })}
  </div>
);

/** The full Product page chrome (window + sidebar + view toggle + tab strip). */
const ProductFrame: React.FC<{
  title?: string;
  view?: 'stories' | 'learnings';
  activeTab?: string;
  importGlow?: boolean;
  learnFilter?: string;
  children: React.ReactNode;
  cursor?: React.ReactNode;
}> = ({ title = 'Otto — Product', view = 'stories', activeTab, importGlow, learnFilter, children, cursor }) => (
  <OttoWindow
    title={title}
    sidebar={view === 'stories' ? <StoriesSidebar importGlow={importGlow} /> : <LearningsSidebar filter={learnFilter} />}
    style={{ width: WIN.width, height: WIN.height }}
  >
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <ViewToggle active={view} />
      {view === 'stories' && activeTab ? <TabStrip active={activeTab} /> : null}
      <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' as const, padding: 16 }}>{children}</div>
      {cursor}
    </div>
  </OttoWindow>
);

// Generic card
const Card: React.FC<{ children: React.ReactNode; style?: React.CSSProperties }> = ({ children, style }) => (
  <div
    style={{
      border: `1px solid ${theme.border}`,
      borderRadius: 8,
      padding: '12px 14px',
      background: theme.surface2,
      ...style,
    }}
  >
    {children}
  </div>
);

// ════════════════════════════════════════════════════════════════════════════
// Scene 1 — Title
// ════════════════════════════════════════════════════════════════════════════

const SceneTitle: React.FC = () => (
  <AbsoluteFill>
    <TitleCard kicker="OTTO ADE" title="Product" subtitle="From a raw ticket to a build-ready spec" />
  </AbsoluteFill>
);

// ════════════════════════════════════════════════════════════════════════════
// Scene 2 — Bring stories in (Import / Draft)
// ════════════════════════════════════════════════════════════════════════════

const SceneImport: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // import dialog slides in after the cursor reaches the Import button (~50f)
  const dlgS = spring({ frame: frame - 52, fps, config: { damping: 200 } });
  const dlgOp = interpolate(dlgS, [0, 1], [0, 1]);
  const dlgY = interpolate(dlgS, [0, 1], [18, 0]);

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <ProductFrame
        importGlow={frame > 30}
        cursor={<Cursor from={[700, 480]} to={[202, 24]} startAt={6} duration={34} click />}
      >
        <div style={{ position: 'relative', height: '100%' }}>
          {/* empty-state prompt */}
          <div style={{ height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 14, color: theme.textDim }}>
            <div style={{ fontSize: 40 }}>📄</div>
            <div style={{ fontFamily: theme.font, fontSize: 15, maxWidth: 360, textAlign: 'center' as const, lineHeight: 1.5 }}>
              Select a story from the sidebar, or import one to get started.
            </div>
          </div>

          {/* Import dialog */}
          {frame > 48 && (
            <div
              style={{
                position: 'absolute',
                inset: 0,
                background: 'rgba(5,7,11,0.55)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              <div
                style={{
                  opacity: dlgOp,
                  transform: `translateY(${dlgY}px)`,
                  width: 560,
                  background: theme.surface,
                  border: `1px solid ${theme.border}`,
                  borderRadius: 14,
                  boxShadow: '0 30px 90px rgba(0,0,0,0.6)',
                  overflow: 'hidden' as const,
                }}
              >
                <div style={{ padding: '14px 18px', borderBottom: `1px solid ${theme.border}`, fontFamily: theme.font, fontSize: 15, fontWeight: 700, color: theme.text }}>
                  Import a story
                </div>
                <div style={{ padding: 18, display: 'flex', flexDirection: 'column', gap: 12 }}>
                  <div style={{ display: 'flex', gap: 10 }}>
                    {[
                      { icon: '🎫', label: 'Jira issue', sub: 'Paste a key or URL', on: true },
                      { icon: '🌐', label: 'Confluence page', sub: 'Spec / RFC / notes', on: false },
                    ].map((o) => (
                      <div
                        key={o.label}
                        style={{
                          flex: 1,
                          padding: '12px 14px',
                          borderRadius: 10,
                          border: `1px solid ${o.on ? theme.accent : theme.border}`,
                          background: o.on ? `${theme.accent}14` : theme.surface2,
                        }}
                      >
                        <div style={{ fontSize: 20, marginBottom: 6 }}>{o.icon}</div>
                        <div style={{ fontFamily: theme.font, fontSize: 13.5, fontWeight: 700, color: theme.text }}>{o.label}</div>
                        <div style={{ fontFamily: theme.font, fontSize: 11, color: theme.textDim }}>{o.sub}</div>
                      </div>
                    ))}
                  </div>
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 10,
                      padding: '10px 12px',
                      borderRadius: 8,
                      border: `1px solid ${theme.accent}66`,
                      background: theme.surface2,
                      fontFamily: theme.mono,
                      fontSize: 13,
                      color: theme.text,
                    }}
                  >
                    <span style={{ color: theme.textDim }}>🔎</span> SIN-4821
                    <span style={{ flex: 1 }} />
                    <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.accent2 }}>found · Multi-currency wallet…</span>
                  </div>
                  <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
                    <div style={{ padding: '7px 16px', borderRadius: 8, background: theme.accent, color: '#fff', fontFamily: theme.font, fontSize: 13, fontWeight: 700 }}>
                      Import →
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>
      </ProductFrame>
      <Caption step={1} title="Bring your stories in" sub="Import from Jira & Confluence — or start a blank Discovery draft" delay={14} />
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Scene 3 — Overview
// ════════════════════════════════════════════════════════════════════════════

const SceneOverview: React.FC = () => {
  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <ProductFrame activeTab="Overview">
        <div style={{ maxWidth: 760, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <Appear delay={6} y={14}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <span style={{ fontFamily: theme.mono, fontSize: 13, color: theme.accent }}>SIN-4821</span>
              <StageBadge stage="review" />
              <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.textDim }}>· synced from Jira</span>
            </div>
          </Appear>
          <Appear delay={12} y={14}>
            <div style={{ fontFamily: theme.font, fontSize: 26, fontWeight: 800, color: theme.text, lineHeight: 1.15 }}>
              Multi-currency wallet for VIP players
            </div>
          </Appear>
          <Appear delay={20} y={12}>
            <Card>
              <SectionLabel style={{ marginBottom: 8 }}>Description</SectionLabel>
              <div style={{ fontFamily: theme.font, fontSize: 14, color: theme.text, lineHeight: 1.65 }}>
                VIP players should be able to hold balances in multiple currencies and switch the active
                wallet without a support ticket. Conversions use the live FX rate at time of bet.
              </div>
            </Card>
          </Appear>
          <Appear delay={28} y={12}>
            <Card>
              <SectionLabel style={{ marginBottom: 8 }}>Acceptance criteria</SectionLabel>
              {[
                'Player can add up to 3 currency wallets',
                'Active wallet switch is instant and audited',
                'Bets convert at the FX rate captured at placement',
              ].map((c, i) => (
                <div key={i} style={{ display: 'flex', gap: 8, alignItems: 'flex-start', padding: '3px 0', fontFamily: theme.font, fontSize: 13.5, color: theme.text }}>
                  <span style={{ color: theme.accent2 }}>✓</span> {c}
                </div>
              ))}
            </Card>
          </Appear>
        </div>
      </ProductFrame>
      <Caption step={2} title="Overview — the story, in one place" sub="Description, acceptance criteria & stage, synced live from the source" delay={14} />
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Scene 4 — Analysis (HERO)
// ════════════════════════════════════════════════════════════════════════════

const LENSES = [
  { name: 'PO Overview', provs: ['claude'] as const, icon: '🧭' },
  { name: 'Architecture', provs: ['claude', 'codex'] as const, icon: '🏗' },
  { name: 'Clarifying Questions', provs: ['gemini'] as const, icon: '❓' },
] as const;

type AgentStatus = 'idle' | 'running' | 'done';

const AgentRow: React.FC<{
  name: string;
  prov: keyof typeof PROV;
  status: AgentStatus;
  findings?: number;
  delay?: number;
}> = ({ name, prov, status, findings, delay = 0 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 200 } });
  const color = PROV[prov];
  const label = status === 'running' ? 'running…' : status === 'done' ? 'done' : 'queued';
  const sc = status === 'running' ? theme.warn : status === 'done' ? theme.accent2 : theme.textDim;
  const pulse = status === 'running' ? 0.5 + Math.sin(frame / 5) * 0.5 : 1;
  return (
    <div
      style={{
        opacity: interpolate(s, [0, 1], [0, 1]),
        transform: `translateY(${interpolate(s, [0, 1], [14, 0])}px)`,
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        padding: '9px 12px',
        borderRadius: 8,
        background: theme.surface2,
        border: `1px solid ${color}44`,
        boxShadow: status === 'done' ? `0 0 16px ${color}22` : 'none',
      }}
    >
      <span style={{ fontFamily: theme.font, fontSize: 13, fontWeight: 600, color: theme.text, flex: 1 }}>{name}</span>
      <span style={{ fontFamily: theme.mono, fontSize: 11, color, background: `${color}18`, padding: '2px 8px', borderRadius: 5 }}>{prov}</span>
      <div style={{ display: 'flex', alignItems: 'center', gap: 5, width: 78, justifyContent: 'flex-end' }}>
        <span style={{ width: 7, height: 7, borderRadius: '50%', background: sc, opacity: pulse }} />
        <span style={{ fontFamily: theme.font, fontSize: 11.5, color: sc, fontWeight: 600 }}>{label}</span>
      </div>
      {status === 'done' && findings != null && (
        <span style={{ fontFamily: theme.font, fontSize: 10.5, fontWeight: 700, color, background: `${color}22`, padding: '2px 8px', borderRadius: 8 }}>
          {findings} found
        </span>
      )}
    </div>
  );
};

const SceneAnalysis: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // timeline: 0 configure → 44 click Run → 55 running → 130 done → 140 summary/findings
  const a1: AgentStatus = frame < 55 ? 'idle' : frame < 120 ? 'running' : 'done';
  const a2: AgentStatus = frame < 58 ? 'idle' : frame < 130 ? 'running' : 'done';
  const a3: AgentStatus = frame < 60 ? 'idle' : frame < 138 ? 'running' : 'done';
  const allDone = frame >= 140;

  const sumS = spring({ frame: frame - 142, fps, config: { damping: 200 } });
  const sumOp = interpolate(sumS, [0, 1], [0, 1]);
  const sumY = interpolate(sumS, [0, 1], [16, 0]);

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <ProductFrame
        title="Otto — Product · Analysis"
        activeTab="Analysis"
        cursor={frame > 30 && frame < 64 ? <Cursor from={[470, 470]} to={[470, 372]} startAt={0} duration={16} click /> : undefined}
      >
        <div style={{ display: 'flex', gap: 14, height: '100%' }}>
          {/* Left: configure panel */}
          <div style={{ width: 470, flexShrink: 0, display: 'flex', flexDirection: 'column', gap: 10 }}>
            <Card>
              <SectionLabel style={{ marginBottom: 10 }}>Configure</SectionLabel>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                {LENSES.map((l) => (
                  <div key={l.name} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                    <span
                      style={{
                        width: 16,
                        height: 16,
                        borderRadius: 4,
                        background: theme.accent,
                        color: '#fff',
                        display: 'grid',
                        placeItems: 'center',
                        fontSize: 11,
                        flexShrink: 0,
                      }}
                    >
                      ✓
                    </span>
                    <span style={{ fontFamily: theme.font, fontSize: 13, color: theme.text, width: 168 }}>
                      {l.icon} {l.name}
                    </span>
                    <div style={{ display: 'flex', gap: 5 }}>
                      {(['claude', 'codex', 'gemini'] as const).map((p) => {
                        const on = (l.provs as readonly string[]).includes(p);
                        return (
                          <span
                            key={p}
                            style={{
                              height: 22,
                              padding: '0 9px',
                              display: 'inline-flex',
                              alignItems: 'center',
                              borderRadius: 999,
                              border: `1px solid ${on ? PROV[p] : theme.border}`,
                              background: on ? `${PROV[p]}1f` : 'transparent',
                              color: on ? PROV[p] : theme.textDim,
                              fontFamily: theme.font,
                              fontSize: 10.5,
                              fontWeight: 600,
                            }}
                          >
                            {p}
                          </span>
                        );
                      })}
                    </div>
                  </div>
                ))}
              </div>

              {/* focus + summarizer + run */}
              <div style={{ marginTop: 12, paddingTop: 10, borderTop: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 10 }}>
                <SectionLabel>Summarizer</SectionLabel>
                <span style={{ fontFamily: theme.font, fontSize: 12, color: theme.text, border: `1px solid ${theme.border}`, borderRadius: 6, padding: '3px 10px' }}>
                  claude ▾
                </span>
                <span style={{ flex: 1 }} />
                <div
                  style={{
                    padding: '7px 18px',
                    borderRadius: 8,
                    border: `1px solid ${theme.accent}`,
                    background: frame >= 48 ? theme.accent : `${theme.accent}1f`,
                    color: frame >= 48 ? '#fff' : theme.accent,
                    fontFamily: theme.font,
                    fontSize: 13,
                    fontWeight: 700,
                    boxShadow: frame >= 48 ? `0 6px 20px ${theme.accent}55` : 'none',
                  }}
                >
                  {frame >= 48 ? 'Starting…' : 'Run analysis'}
                </div>
              </div>
            </Card>

            {/* Agents */}
            {frame > 52 && (
              <Appear delay={0} y={10}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <SectionLabel>Agents</SectionLabel>
                    <span style={{ fontFamily: theme.font, fontSize: 10.5, fontWeight: 700, color: allDone ? theme.accent2 : theme.warn, background: `${allDone ? theme.accent2 : theme.warn}22`, padding: '2px 8px', borderRadius: 6 }}>
                      {allDone ? 'done' : 'running'}
                    </span>
                  </div>
                  <AgentRow name="PO Overview" prov="claude" status={a1} findings={3} delay={2} />
                  <AgentRow name="Architecture" prov="codex" status={a2} findings={5} delay={6} />
                  <AgentRow name="Clarifying Questions" prov="gemini" status={a3} findings={4} delay={10} />
                </div>
              </Appear>
            )}
          </div>

          {/* Right: synthesized summary + findings */}
          <div style={{ flex: 1, minWidth: 0 }}>
            {allDone ? (
              <div style={{ opacity: sumOp, transform: `translateY(${sumY}px)`, display: 'flex', flexDirection: 'column', gap: 10 }}>
                <Card style={{ borderColor: `${theme.accent}55` }}>
                  <SectionLabel style={{ marginBottom: 8 }}>Synthesized summary</SectionLabel>
                  <div style={{ fontFamily: theme.font, fontSize: 13.5, color: theme.text, lineHeight: 1.6 }}>
                    Spans <b>wallet-gateway</b>, <b>currency</b> and <b>ledger</b> services. Main risks: FX-rate
                    capture timing and audit of wallet switches. 4 open questions need PO input before build.
                  </div>
                </Card>
                <Card>
                  <SectionLabel style={{ marginBottom: 8, color: theme.danger }}>⚠ Risks</SectionLabel>
                  {['FX rate must be locked at bet placement, not settlement', 'Wallet switch needs an audit trail for compliance'].map((r, i) => (
                    <div key={i} style={{ fontFamily: theme.font, fontSize: 12.5, color: theme.text, padding: '2px 0' }}>• {r}</div>
                  ))}
                </Card>
                <Card>
                  <SectionLabel style={{ marginBottom: 8 }}>Open questions <span style={{ color: theme.textDim }}>(4)</span></SectionLabel>
                  {[
                    ['Which currencies are in scope for launch?', 'Scope'],
                    ['Do we cap the number of active wallets?', 'Edge case'],
                  ].map(([q, cat], i) => (
                    <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '3px 0' }}>
                      <span style={{ fontFamily: theme.font, fontSize: 12.5, color: theme.text, flex: 1 }}>{q}</span>
                      <span style={{ fontFamily: theme.font, fontSize: 10, color: theme.textDim, fontStyle: 'italic' as const }}>{cat}</span>
                    </div>
                  ))}
                </Card>
              </div>
            ) : (
              <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>
                {frame > 52 ? 'Agents working — results stream in live…' : 'Pick lenses & providers, then Run.'}
              </div>
            )}
          </div>
        </div>
      </ProductFrame>
      {frame < 138 ? (
        <Caption step={3} title="Analysis — many lenses, many models" sub="PO · Architecture · Clarifying Questions × claude / codex / gemini" delay={12} />
      ) : (
        <Caption step={3} title="One synthesized summary + findings" sub="Risks, open questions & suggested learnings — distilled by the summarizer" delay={0} />
      )}
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Scene 5 — Questions
// ════════════════════════════════════════════════════════════════════════════

const QUESTIONS = [
  { q: 'Which currencies ship at launch (EUR, USD, GBP)?', cat: 'Scope' },
  { q: 'Do we cap the number of active wallets per player?', cat: 'Edge case' },
  { q: 'Is FX margin configurable per brand or global?', cat: 'Compliance' },
  { q: 'Where does the wallet switcher live in the UI?', cat: 'UX' },
];

const SceneQuestions: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const posted = frame > 120;
  const toastS = spring({ frame: frame - 124, fps, config: { damping: 180 } });

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <ProductFrame
        title="Otto — Product · Questions"
        activeTab="Questions"
        cursor={frame > 80 && frame < 122 ? <Cursor from={[680, 470]} to={[600, 150]} startAt={0} duration={26} click /> : undefined}
      >
        <div style={{ maxWidth: 740, display: 'flex', flexDirection: 'column', gap: 10, position: 'relative' }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <SectionLabel>Clarifying questions <span style={{ color: theme.textDim }}>· 4 for the PO</span></SectionLabel>
            <div
              style={{
                padding: '6px 14px',
                borderRadius: 8,
                border: `1px solid ${theme.accent}`,
                background: posted ? theme.accent : `${theme.accent}1f`,
                color: posted ? '#fff' : theme.accent,
                fontFamily: theme.font,
                fontSize: 12.5,
                fontWeight: 700,
              }}
            >
              {posted ? '✓ Posted to Jira' : 'Post 4 to Jira →'}
            </div>
          </div>

          {QUESTIONS.map((item, i) => {
            const s = spring({ frame: frame - 10 - i * 12, fps, config: { damping: 200 } });
            return (
              <div
                key={i}
                style={{
                  opacity: interpolate(s, [0, 1], [0, 1]),
                  transform: `translateX(${interpolate(s, [0, 1], [-12, 0])}px)`,
                }}
              >
                <Card style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                  <span style={{ fontFamily: theme.font, fontSize: 13.5, color: theme.text, flex: 1, lineHeight: 1.45 }}>{item.q}</span>
                  <span style={{ fontFamily: theme.font, fontSize: 10.5, fontWeight: 700, color: theme.accent, background: `${theme.accent}18`, padding: '2px 9px', borderRadius: 999 }}>{item.cat}</span>
                  {posted && <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.accent2 }}>↗ commented</span>}
                </Card>
              </div>
            );
          })}

          {/* toast */}
          {frame > 122 && (
            <div
              style={{
                position: 'absolute',
                right: 0,
                bottom: -70,
                opacity: interpolate(toastS, [0, 1], [0, 1]),
                transform: `translateY(${interpolate(toastS, [0, 1], [16, 0])}px)`,
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '10px 16px',
                borderRadius: 10,
                background: theme.surface2,
                border: `1px solid ${theme.accent2}55`,
                boxShadow: '0 14px 40px rgba(0,0,0,0.5)',
              }}
            >
              <span style={{ fontSize: 16 }}>🎫</span>
              <span style={{ fontFamily: theme.font, fontSize: 13, color: theme.text }}>
                4 questions posted as comments on <b>SIN-4821</b>
              </span>
            </div>
          )}
        </div>
      </ProductFrame>
      <Caption step={4} title="Questions — straight back to Jira" sub="Triage AI clarifications, then post them as comments on the ticket" delay={12} />
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Scene 6 — Rewrite
// ════════════════════════════════════════════════════════════════════════════

const SceneRewrite: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const newS = spring({ frame: frame - 60, fps, config: { damping: 200 } });
  const newOp = interpolate(newS, [0, 1], [0, 1]);
  const newX = interpolate(newS, [0, 1], [40, 0]);

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <ProductFrame
        title="Otto — Product · Rewrite"
        activeTab="Rewrite"
        cursor={frame > 28 && frame < 64 ? <Cursor from={[700, 470]} to={[300, 150]} startAt={0} duration={22} click /> : undefined}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12, height: '100%' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <SectionLabel>Rewrite with Otto</SectionLabel>
            <span style={{ flex: 1 }} />
            <div
              style={{
                padding: '6px 14px',
                borderRadius: 8,
                border: `1px solid ${theme.accent2}`,
                background: frame >= 50 ? `${theme.accent2}22` : 'transparent',
                color: theme.accent2,
                fontFamily: theme.font,
                fontSize: 12.5,
                fontWeight: 700,
              }}
            >
              {frame >= 50 ? 'Rewriting…' : 'Rewrite story →'}
            </div>
          </div>

          <div style={{ display: 'flex', gap: 14, flex: 1, minHeight: 0 }}>
            {/* original */}
            <Card style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 8 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <SectionLabel>Original</SectionLabel>
                <span style={{ fontFamily: theme.mono, fontSize: 10, color: theme.textDim, background: `${theme.textDim}18`, padding: '1px 7px', borderRadius: 999 }}>v1</span>
              </div>
              <div style={{ fontFamily: theme.font, fontSize: 12.5, color: theme.textDim, lineHeight: 1.6 }}>
                VIP players want multiple currencies. Let them switch the wallet. Convert using FX.
              </div>
            </Card>

            {/* rewritten */}
            <div style={{ flex: 1, opacity: frame >= 60 ? newOp : 0.15, transform: `translateX(${frame >= 60 ? newX : 40}px)` }}>
              <Card style={{ height: '100%', borderColor: `${theme.accent}66`, display: 'flex', flexDirection: 'column', gap: 8 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <SectionLabel style={{ color: theme.accent }}>Rewritten by Otto</SectionLabel>
                  <span style={{ fontFamily: theme.mono, fontSize: 10, color: theme.accent, background: `${theme.accent}22`, padding: '1px 7px', borderRadius: 999 }}>v2 · new version</span>
                </div>
                <div style={{ fontFamily: theme.font, fontSize: 12.5, color: theme.text, lineHeight: 1.55 }}>
                  <b>As a</b> VIP player, <b>I want</b> to hold and switch between up to 3 currency wallets,
                  <b> so that</b> I can bet in my preferred currency.
                </div>
                <div style={{ marginTop: 2 }}>
                  <div style={{ fontFamily: theme.font, fontSize: 11, fontWeight: 700, color: theme.textDim, marginBottom: 4 }}>Acceptance criteria</div>
                  {['Add / remove wallets (max 3)', 'Switch is instant + written to audit log', 'FX rate locked at bet placement'].map((c, i) => (
                    <div key={i} style={{ display: 'flex', gap: 6, fontFamily: theme.font, fontSize: 12, color: theme.text, padding: '1px 0' }}>
                      <span style={{ color: theme.accent2 }}>✓</span> {c}
                    </div>
                  ))}
                </div>
              </Card>
            </div>
          </div>
        </div>
      </ProductFrame>
      <Caption step={5} title="Rewrite — sharper, every time" sub="Otto restructures the story into a clean spec & saves it as a new version" delay={12} />
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Scene 7 — The rest, fast (Notes · Test Cases · Plan · History)
// ════════════════════════════════════════════════════════════════════════════

const PhaseFade: React.FC<{ start: number; children: React.ReactNode }> = ({ start, children }) => {
  const frame = useCurrentFrame();
  const op = interpolate(frame - start, [0, 10], [0, 1], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
  return <div style={{ opacity: op, transform: `translateY(${interpolate(op, [0, 1], [10, 0])}px)` }}>{children}</div>;
};

const SceneMontage: React.FC = () => {
  const frame = useCurrentFrame();
  const phase = frame < 52 ? 0 : frame < 106 ? 1 : frame < 160 ? 2 : 3;
  const meta = [
    { tab: 'Notes', step: 6, title: 'Notes — capture decisions', sub: 'Pin context, links & PO answers right on the story' },
    { tab: 'Test Cases', step: 7, title: 'Test Cases — generated for you', sub: 'Given / When / Then drafts you can edit, approve & publish' },
    { tab: 'Plan', step: 8, title: 'Plan — a build checklist', sub: 'An implementation plan with PO-checkable tasks' },
    { tab: 'History', step: 9, title: 'History — full audit trail', sub: 'Every analysis, rewrite & post, timestamped' },
  ][phase];

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <ProductFrame title={`Otto — Product · ${meta.tab}`} activeTab={meta.tab}>
        <div style={{ maxWidth: 760 }}>
          {phase === 0 && (
            <PhaseFade start={0}>
              <SectionLabel style={{ marginBottom: 10 }}>Notes</SectionLabel>
              {[
                ['PO call · 14 Jun', 'Launch currencies confirmed: EUR, USD, GBP. Cap at 3 wallets.'],
                ['Eng note', 'Reuse currency-service FX cache; no new dependency.'],
              ].map(([h, b], i) => (
                <Card key={i} style={{ marginBottom: 8, borderLeft: `3px solid ${theme.accent}` }}>
                  <div style={{ fontFamily: theme.font, fontSize: 11, color: theme.textDim, marginBottom: 4 }}>{h}</div>
                  <div style={{ fontFamily: theme.font, fontSize: 13.5, color: theme.text, lineHeight: 1.5 }}>{b}</div>
                </Card>
              ))}
            </PhaseFade>
          )}
          {phase === 1 && (
            <PhaseFade start={52}>
              <SectionLabel style={{ marginBottom: 10 }}>Test cases <span style={{ color: theme.textDim }}>· generated</span></SectionLabel>
              {[
                ['TC-1', 'Add second wallet', 'Given a VIP with 1 wallet, when they add EUR, then 2 wallets exist'],
                ['TC-2', 'Switch is audited', 'Given 2 wallets, when active switches, then an audit row is written'],
                ['TC-3', 'FX locked at bet', 'Given a placed bet, when settled, then the placement FX rate is used'],
              ].map(([id, t, gwt], i) => (
                <Card key={i} style={{ marginBottom: 8, display: 'flex', gap: 10, alignItems: 'flex-start' }}>
                  <span style={{ fontFamily: theme.mono, fontSize: 11, color: theme.accent2, background: `${theme.accent2}18`, padding: '2px 7px', borderRadius: 5 }}>{id}</span>
                  <div>
                    <div style={{ fontFamily: theme.font, fontSize: 13, fontWeight: 600, color: theme.text }}>{t}</div>
                    <div style={{ fontFamily: theme.font, fontSize: 11.5, color: theme.textDim, marginTop: 2 }}>{gwt}</div>
                  </div>
                </Card>
              ))}
            </PhaseFade>
          )}
          {phase === 2 && (
            <PhaseFade start={106}>
              <SectionLabel style={{ marginBottom: 10 }}>Implementation plan</SectionLabel>
              <Card>
                {[
                  ['✓', 'Schema: player_wallets (player_id, currency, is_active)'],
                  ['✓', 'wallet-gateway: add/switch/list endpoints'],
                  ['☐', 'Capture FX rate at bet placement in ledger'],
                  ['☐', 'Audit log on active-wallet switch'],
                  ['☐', 'UI: wallet switcher in player header'],
                ].map(([box, t], i) => (
                  <div key={i} style={{ display: 'flex', gap: 8, padding: '4px 0', fontFamily: theme.font, fontSize: 13, color: theme.text }}>
                    <span style={{ color: box === '✓' ? theme.accent2 : theme.textDim }}>{box}</span> {t}
                  </div>
                ))}
              </Card>
            </PhaseFade>
          )}
          {phase === 3 && (
            <PhaseFade start={160}>
              <SectionLabel style={{ marginBottom: 10 }}>History</SectionLabel>
              <Card>
                {[
                  ['🧠', 'Analysis run — 3 lenses, 12 findings', '2m ago'],
                  ['✍', 'Story rewritten → v2', '5m ago'],
                  ['🎫', '4 questions posted to SIN-4821', '8m ago'],
                  ['🧪', 'Test cases generated (3)', '11m ago'],
                ].map(([ic, t, when], i) => (
                  <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '6px 0', borderBottom: i < 3 ? `1px solid ${theme.border}` : 'none' }}>
                    <span style={{ fontSize: 14 }}>{ic}</span>
                    <span style={{ fontFamily: theme.font, fontSize: 13, color: theme.text, flex: 1 }}>{t}</span>
                    <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.textDim }}>{when}</span>
                  </div>
                ))}
              </Card>
            </PhaseFade>
          )}
        </div>
      </ProductFrame>
      <Caption step={meta.step} title={meta.title} sub={meta.sub} delay={4} />
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Scene 8 — Inject into a coding agent
// ════════════════════════════════════════════════════════════════════════════

const SceneInject: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const spawnS = spring({ frame: frame - 70, fps, config: { damping: 180 } });
  const spawnOp = interpolate(spawnS, [0, 1], [0, 1]);
  const spawnX = interpolate(spawnS, [0, 1], [40, 0]);

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <ProductFrame
        title="Otto — Product · Inject"
        activeTab="Inject"
        cursor={frame > 28 && frame < 64 ? <Cursor from={[360, 470]} to={[300, 300]} startAt={0} duration={22} click /> : undefined}
      >
        <div style={{ display: 'flex', gap: 16, height: '100%' }}>
          {/* bundle */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 10 }}>
            <SectionLabel>Inject bundle</SectionLabel>
            <Card>
              <div style={{ fontFamily: theme.font, fontSize: 12.5, color: theme.text, marginBottom: 8 }}>
                Otto bundles the refined context for a coding session:
              </div>
              {['Rewritten story (v2)', 'Analysis summary + risks', 'PO answers & notes', 'Approved test cases', 'Relevant learnings'].map((c, i) => (
                <div key={i} style={{ display: 'flex', gap: 8, padding: '3px 0', fontFamily: theme.font, fontSize: 13, color: theme.text }}>
                  <span style={{ color: theme.accent2 }}>✓</span> {c}
                </div>
              ))}
              <div
                style={{
                  marginTop: 12,
                  padding: '9px 16px',
                  borderRadius: 8,
                  background: frame >= 50 ? theme.accent2 : `${theme.accent2}22`,
                  color: frame >= 50 ? theme.bg : theme.accent2,
                  fontFamily: theme.font,
                  fontSize: 13,
                  fontWeight: 700,
                  textAlign: 'center' as const,
                  border: `1px solid ${theme.accent2}`,
                }}
              >
                {frame >= 50 ? 'Injecting…' : '▸_ Inject into new session →'}
              </div>
            </Card>
          </div>

          {/* spawned agent */}
          <div style={{ width: 420, flexShrink: 0 }}>
            {frame > 68 && (
              <div
                style={{
                  opacity: spawnOp,
                  transform: `translateX(${spawnX}px)`,
                  background: `${theme.accent}0e`,
                  border: `1px solid ${theme.accent}44`,
                  borderRadius: 12,
                  padding: 16,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
                  <div style={{ width: 32, height: 32, borderRadius: 8, background: `${theme.accent}22`, display: 'grid', placeItems: 'center', fontSize: 15 }}>▸_</div>
                  <div>
                    <div style={{ fontFamily: theme.font, fontSize: 13, fontWeight: 700, color: theme.text }}>claude · SIN-4821-wallet</div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 5, marginTop: 2 }}>
                      <span style={{ width: 7, height: 7, borderRadius: '50%', background: theme.accent2, opacity: 0.7 + Math.sin(frame / 6) * 0.3 }} />
                      <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.accent2 }}>working</span>
                    </div>
                  </div>
                </div>
                {[
                  { t: 0, text: 'Loaded story v2 + analysis + test cases', c: theme.textDim },
                  { t: 12, text: '→ Reading wallet-gateway service…', c: theme.text },
                  { t: 26, text: '→ Drafting player_wallets migration…', c: theme.text },
                  { t: 40, text: '→ Scaffolding add/switch endpoints…', c: theme.text },
                ].map((line, i) => {
                  const op = interpolate(frame - 76, [line.t, line.t + 10], [0, 1], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });
                  return (
                    <div key={i} style={{ opacity: op, fontFamily: theme.mono, fontSize: 12, color: line.c, lineHeight: 1.7 }}>{line.text}</div>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      </ProductFrame>
      <Caption step={10} title="Inject — straight into a coding agent" sub="The whole refined bundle becomes the context for a fresh build session" delay={12} />
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Scene 9 — Learnings
// ════════════════════════════════════════════════════════════════════════════

const SceneLearnings: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const accepted = frame > 110;
  const accS = spring({ frame: frame - 112, fps, config: { damping: 180 } });

  const CARDS = [
    { kind: 'pattern', title: 'Lock FX at placement', body: 'Always capture the FX rate when a bet is placed, never at settlement.', active: true },
    { kind: 'avoid', title: 'Don’t mutate active wallet silently', body: 'Wallet switches must be audited — a past incident traced to a silent switch.', active: true },
  ];

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <ProductFrame
        title="Otto — Product · Learnings"
        view="learnings"
        learnFilter="all"
        cursor={frame > 70 && frame < 112 ? <Cursor from={[680, 470]} to={[620, 410]} startAt={0} duration={26} click /> : undefined}
      >
        <div style={{ maxWidth: 760, display: 'flex', flexDirection: 'column', gap: 10 }}>
          <SectionLabel>Learnings — your team’s memory</SectionLabel>
          {CARDS.map((c, i) => {
            const s = spring({ frame: frame - 8 - i * 12, fps, config: { damping: 200 } });
            const col = c.kind === 'pattern' ? theme.accent2 : theme.danger;
            return (
              <div key={i} style={{ opacity: interpolate(s, [0, 1], [0, 1]), transform: `translateY(${interpolate(s, [0, 1], [12, 0])}px)` }}>
                <Card style={{ borderLeft: `3px solid ${col}` }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
                    <span style={{ fontFamily: theme.font, fontSize: 10, fontWeight: 700, textTransform: 'uppercase' as const, letterSpacing: 0.5, color: col, background: `${col}22`, padding: '1px 7px', borderRadius: 999 }}>
                      {c.kind === 'pattern' ? 'Pattern to follow' : 'Case to avoid'}
                    </span>
                    <span style={{ fontFamily: theme.font, fontSize: 13.5, fontWeight: 700, color: theme.text }}>{c.title}</span>
                  </div>
                  <div style={{ fontFamily: theme.font, fontSize: 12.5, color: theme.textDim, lineHeight: 1.5 }}>{c.body}</div>
                </Card>
              </div>
            );
          })}

          {/* suggested learning from analysis → Accept */}
          <Card style={{ borderColor: `${theme.accent}66`, background: `${theme.accent}0c` }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
              <span style={{ fontFamily: theme.font, fontSize: 10, fontWeight: 700, color: theme.accent, background: `${theme.accent}22`, padding: '1px 7px', borderRadius: 999 }}>
                SUGGESTED · from analysis
              </span>
              <span style={{ fontFamily: theme.font, fontSize: 13.5, fontWeight: 700, color: theme.text }}>Reuse the currency-service FX cache</span>
            </div>
            <div style={{ fontFamily: theme.font, fontSize: 12.5, color: theme.textDim, lineHeight: 1.5, marginBottom: 10 }}>
              The architecture lens found an existing FX cache — prefer it over a new dependency.
            </div>
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <div
                style={{
                  padding: '6px 16px',
                  borderRadius: 7,
                  border: `1px solid ${theme.accent2}`,
                  background: accepted ? theme.accent2 : `${theme.accent2}22`,
                  color: accepted ? theme.bg : theme.accent2,
                  fontFamily: theme.font,
                  fontSize: 12.5,
                  fontWeight: 700,
                }}
              >
                {accepted ? '✓ Accepted' : 'Accept'}
              </div>
              {!accepted && (
                <div style={{ padding: '6px 16px', borderRadius: 7, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 12.5 }}>
                  Dismiss
                </div>
              )}
              {accepted && (
                <span style={{ opacity: interpolate(accS, [0, 1], [0, 1]), fontFamily: theme.font, fontSize: 11.5, color: theme.accent2 }}>
                  → added to your active learnings
                </span>
              )}
            </div>
          </Card>
        </div>
      </ProductFrame>
      <Caption step={11} title="Learnings — patterns the team keeps" sub="Accept AI-suggested learnings so every future story starts smarter" delay={12} />
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Scene 10 — Outro
// ════════════════════════════════════════════════════════════════════════════

const SceneOutro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const pulse = Math.sin(frame / 20) * 0.5 + 0.5;
  const glow = interpolate(pulse, [0, 1], [60, 110]);

  const logoS = spring({ frame, fps, config: { damping: 18, stiffness: 100 } });
  const logoScale = interpolate(logoS, [0, 1], [0.6, 1]);
  const logoOp = interpolate(frame, [0, 18], [0, 1], { extrapolateRight: 'clamp' });

  const textS = spring({ frame: frame - 16, fps, config: { damping: 200 } });
  const textOp = interpolate(textS, [0, 1], [0, 1]);
  const textY = interpolate(textS, [0, 1], [24, 0]);

  const subS = spring({ frame: frame - 30, fps, config: { damping: 200 } });
  const subOp = interpolate(subS, [0, 1], [0, 1]);

  const pillsS = spring({ frame: frame - 54, fps, config: { damping: 200 } });
  const pillsOp = interpolate(pillsS, [0, 1], [0, 1]);
  const pillsY = interpolate(pillsS, [0, 1], [16, 0]);

  const STEPS = [
    { label: 'Import', color: theme.accent },
    { label: 'Analysis', color: theme.accent2 },
    { label: 'Questions', color: '#bf7aff' },
    { label: 'Rewrite', color: theme.warn },
    { label: 'Test Cases', color: theme.accent },
    { label: 'Plan', color: theme.accent2 },
    { label: 'Inject', color: '#bf7aff' },
    { label: 'Learnings', color: theme.warn },
  ];

  return (
    <AbsoluteFill style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ opacity: logoOp, transform: `scale(${logoScale})`, marginBottom: 30 }}>
        <Img
          src={staticFile('otto-mark.png')}
          style={{ width: 140, height: 140, borderRadius: 34, boxShadow: `0 0 0 1.5px ${theme.accent}55, 0 20px ${glow}px ${theme.accent}55` }}
        />
      </div>
      <div
        style={{
          opacity: textOp,
          transform: `translateY(${textY}px)`,
          fontFamily: theme.font,
          fontSize: 92,
          fontWeight: 900,
          color: theme.text,
          letterSpacing: -2.5,
          lineHeight: 1,
          textAlign: 'center' as const,
        }}
      >
        From ticket to build-ready.
      </div>
      <div
        style={{
          opacity: subOp,
          fontFamily: theme.font,
          fontSize: 28,
          fontWeight: 500,
          color: theme.textDim,
          marginTop: 20,
          textAlign: 'center' as const,
        }}
      >
        Product — refine the story, then ship it to a coding agent
      </div>
      <div
        style={{
          opacity: pillsOp,
          transform: `translateY(${pillsY}px)`,
          display: 'flex',
          gap: 12,
          flexWrap: 'wrap' as const,
          justifyContent: 'center',
          marginTop: 38,
          maxWidth: 980,
        }}
      >
        {STEPS.map((f) => (
          <div
            key={f.label}
            style={{
              padding: '7px 18px',
              borderRadius: 30,
              background: `${f.color}18`,
              border: `1px solid ${f.color}44`,
              fontFamily: theme.font,
              fontSize: 17,
              fontWeight: 700,
              color: theme.text,
              display: 'flex',
              alignItems: 'center',
              gap: 8,
            }}
          >
            <span style={{ width: 7, height: 7, borderRadius: '50%', background: f.color, display: 'inline-block' }} />
            {f.label}
          </div>
        ))}
      </div>
      <div
        style={{
          position: 'absolute',
          bottom: '30%',
          left: '50%',
          transform: 'translateX(-50%)',
          width: interpolate(frame, [46, 96], [0, 520], { extrapolateRight: 'clamp', extrapolateLeft: 'clamp' }),
          height: 1.5,
          background: `linear-gradient(90deg, transparent, ${theme.accent}88, ${theme.accent2}88, transparent)`,
        }}
      />
    </AbsoluteFill>
  );
};

// ════════════════════════════════════════════════════════════════════════════
// Root — 1800 frames (60s @ 30fps)
//
// S1  Title       0    – 90    (3.0s)
// S2  Import      78   – 248   (5.7s)  overlap
// S3  Overview    240  – 370   (4.3s)  overlap
// S4  Analysis    360  – 700   (11.3s) overlap  ⭐
// S5  Questions   690  – 890   (6.7s)  overlap  ⭐
// S6  Rewrite     880  – 1090  (7.0s)  overlap  ⭐
// S7  Montage     1080 – 1295  (7.2s)  overlap
// S8  Inject      1285 – 1460  (5.8s)  overlap
// S9  Learnings   1450 – 1625  (5.8s)  overlap
// S10 Outro       1615 – 1800  (6.2s)  overlap
// ════════════════════════════════════════════════════════════════════════════

export const Product: React.FC = () => (
  <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>
    <Sequence from={0} durationInFrames={90}>
      <FadeIn durationFrames={14}><SceneTitle /></FadeIn>
    </Sequence>
    <Sequence from={78} durationInFrames={170}>
      <FadeIn><SceneImport /></FadeIn>
    </Sequence>
    <Sequence from={240} durationInFrames={130}>
      <FadeIn><SceneOverview /></FadeIn>
    </Sequence>
    <Sequence from={360} durationInFrames={340}>
      <FadeIn><SceneAnalysis /></FadeIn>
    </Sequence>
    <Sequence from={690} durationInFrames={200}>
      <FadeIn><SceneQuestions /></FadeIn>
    </Sequence>
    <Sequence from={880} durationInFrames={210}>
      <FadeIn><SceneRewrite /></FadeIn>
    </Sequence>
    <Sequence from={1080} durationInFrames={215}>
      <FadeIn><SceneMontage /></FadeIn>
    </Sequence>
    <Sequence from={1285} durationInFrames={175}>
      <FadeIn><SceneInject /></FadeIn>
    </Sequence>
    <Sequence from={1450} durationInFrames={175}>
      <FadeIn><SceneLearnings /></FadeIn>
    </Sequence>
    <Sequence from={1615} durationInFrames={185}>
      <FadeIn durationFrames={20}><SceneOutro /></FadeIn>
    </Sequence>
  </AbsoluteFill>
);
