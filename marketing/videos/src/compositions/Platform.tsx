import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, themes, Theme, brand, fonts, alpha, status as STATUS } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  track,
  Caption,
  TitleCard,
  Chip,
  Toggle,
  Segmented,
  Keys,
  Caret,
  StatusDot,
  Icon,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  PLATFORM — power & polish: command palette, mission control, search,
//  capability health, theming, CLI auto-update.
// ════════════════════════════════════════════════════════════════════════════

// ── Scene 1 — title ──────────────────────────────────────────────────────────
const Title: React.FC = () => (
  <TitleCard
    kicker="Power & Polish"
    title="Built to run all day"
    subtitle="Command palette, mission control, health & theming — keyboard-first."
  />
);

// ── Scene 2 — command palette (⌘K) + Ask Otto (⌘I) ───────────────────────────
type PaletteRow = { icon: string; label: string; hint?: string; kind: 'cmd' | 'recent' };

const PALETTE_ROWS: PaletteRow[] = [
  { icon: 'branch', label: 'Git · Open repo tab…', hint: 'Action', kind: 'cmd' },
  { icon: 'send', label: 'API · New request', hint: 'Action', kind: 'cmd' },
  { icon: 'terminal', label: 'fix auth tests', hint: 'sinatra-users-go', kind: 'recent' },
  { icon: 'grid', label: 'Swarm · payments-platform', hint: 'Project', kind: 'recent' },
  { icon: 'db', label: 'Database · prod-readonly', hint: 'Connection', kind: 'recent' },
];

const PaletteResult: React.FC<{ row: PaletteRow; active?: boolean }> = ({ row, active }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 12,
      height: 42,
      padding: '0 14px',
      borderRadius: 9,
      background: active ? alpha(brand.violet, 0.2) : 'transparent',
      boxShadow: active ? `inset 2px 0 0 ${brand.cyan}` : 'none',
    }}
  >
    <span
      style={{
        width: 26,
        height: 26,
        borderRadius: 7,
        background: alpha(active ? brand.cyan : T.textDim, 0.14),
        display: 'grid',
        placeItems: 'center',
        color: active ? brand.cyan : T.textDim,
      }}
    >
      <Icon name={row.icon} size={15} />
    </span>
    <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 16, fontWeight: active ? 650 : 550, color: active ? '#fff' : T.text }}>
      {row.label}
    </span>
    {row.hint && (
      <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>{row.hint}</span>
    )}
    {row.kind === 'recent' && <Icon name="clock" size={13} color={alpha(T.textDim, 0.8)} />}
  </div>
);

const ASK_PLAN = [
  'Find the CI run for `payments` that failed',
  'Open the failing job + jump to the error log',
  'Draft a fix branch from the offending commit',
];

const PaletteScene: React.FC = () => {
  const frame = useCurrentFrame();
  const dim = track(frame, [8, 22], [0, 0.55]);
  return (
    <>
      <Stage scale={0.9}>
        <div style={{ position: 'relative' }}>
          <OttoWindow nav={<Navigator active="agents" workingCount={2} />} title="Otto — sinatra-users-go">
            <div style={{ height: '100%' }} />
          </OttoWindow>

          {/* scrim over the window */}
          <div style={{ position: 'absolute', inset: 0, background: alpha('#05050a', dim), borderRadius: 14 }} />

          {/* ⌘K command palette modal */}
          <Appear delay={14} y={-18} scale={0.97} style={{ position: 'absolute', top: 86, left: '50%', transform: 'translateX(-50%)' }}>
            <div
              style={{
                width: 660,
                borderRadius: 16,
                background: alpha(T.surface, 0.98),
                border: `1px solid ${alpha('#fff', 0.14)}`,
                boxShadow: `0 40px 120px rgba(0,0,0,0.7), 0 0 0 1px ${alpha(brand.violet, 0.3)}`,
                overflow: 'hidden',
              }}
            >
              {/* search field with typed query */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '16px 18px', borderBottom: `1px solid ${T.border}` }}>
                <Icon name="search" size={19} color={T.textDim} />
                <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 19, color: '#fff' }}>
                  open repo
                  <Caret color={brand.cyan} h={20} />
                </span>
                <Chip color={brand.violet}>⌘K</Chip>
              </div>

              {/* result list */}
              <div style={{ padding: '8px 8px 10px' }}>
                <div style={{ fontFamily: fonts.ui, fontSize: 11.5, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase', color: T.textDim, padding: '6px 14px 4px' }}>
                  Commands
                </div>
                <Stagger delay={22} step={5} y={8}>
                  {PALETTE_ROWS.slice(0, 2).map((r, i) => (
                    <PaletteResult key={i} row={r} active={i === 0} />
                  ))}
                </Stagger>
                <div style={{ fontFamily: fonts.ui, fontSize: 11.5, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase', color: T.textDim, padding: '8px 14px 4px' }}>
                  Recent
                </div>
                <Stagger delay={34} step={5} y={8}>
                  {PALETTE_ROWS.slice(2).map((r, i) => (
                    <PaletteResult key={i} row={r} />
                  ))}
                </Stagger>
              </div>
            </div>
          </Appear>

          {/* ⌘I Ask Otto variant — natural language → deterministic plan */}
          <Appear delay={50} y={20} style={{ position: 'absolute', bottom: 96, left: '50%', transform: 'translateX(-50%)' }}>
            <div
              style={{
                width: 660,
                borderRadius: 14,
                background: alpha(T.surface, 0.98),
                border: `1px solid ${alpha(brand.cyan, 0.4)}`,
                boxShadow: `0 30px 90px rgba(0,0,0,0.6)`,
                overflow: 'hidden',
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '12px 16px', borderBottom: `1px solid ${T.border}` }}>
                <Icon name="zap" size={16} color={brand.cyan} />
                <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 15, fontWeight: 600, color: '#fff' }}>
                  “open the failing CI for payments”
                </span>
                <Chip color={brand.cyan}>⌘I · Ask Otto</Chip>
              </div>
              <div style={{ padding: '10px 14px 12px', display: 'flex', flexDirection: 'column', gap: 7 }}>
                <Stagger delay={62} step={6} y={8}>
                  {ASK_PLAN.map((step, i) => (
                    <div
                      key={i}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 11,
                        padding: '8px 12px',
                        borderRadius: 9,
                        background: alpha(brand.violet, 0.1),
                        border: `1px solid ${alpha(brand.violet, 0.28)}`,
                      }}
                    >
                      <span
                        style={{
                          width: 20,
                          height: 20,
                          borderRadius: '50%',
                          background: brand.grad,
                          color: '#fff',
                          fontFamily: fonts.ui,
                          fontSize: 11,
                          fontWeight: 800,
                          display: 'grid',
                          placeItems: 'center',
                          flexShrink: 0,
                        }}
                      >
                        {i + 1}
                      </span>
                      <span style={{ fontFamily: fonts.ui, fontSize: 13.5, color: T.text }}>{step}</span>
                    </div>
                  ))}
                </Stagger>
              </div>
            </div>
          </Appear>
        </div>
      </Stage>
      <Caption step={1} title="⌘K to launch anything · ⌘I to ask" sub="Natural language → a deterministic plan." />
    </>
  );
};

// ── Scene 3 — Mission Control: 6 bucket board ────────────────────────────────
type Bucket = {
  id: string;
  label: string;
  count: number;
  color: string;
  rows: { title: string; provider: string; status: keyof typeof STATUS }[];
};

const BUCKETS: Bucket[] = [
  {
    id: 'needs_you',
    label: 'Needs you',
    count: 3,
    color: STATUS.needsYou,
    rows: [
      { title: 'approve plan · api/v2', provider: 'claude', status: 'needsYou' },
      { title: 'resolve conflict · web', provider: 'codex', status: 'needsYou' },
    ],
  },
  {
    id: 'working',
    label: 'Working',
    count: 5,
    color: STATUS.working,
    rows: [
      { title: 'fix auth tests', provider: 'claude', status: 'working' },
      { title: 'refactor billing', provider: 'codex', status: 'working' },
    ],
  },
  {
    id: 'review_ready',
    label: 'Review-ready',
    count: 4,
    color: brand.cyan,
    rows: [
      { title: 'PR #482 · rate-limit', provider: 'claude', status: 'idle' },
      { title: 'PR #479 · cache layer', provider: 'codex', status: 'idle' },
    ],
  },
  {
    id: 'waiting',
    label: 'Waiting',
    count: 2,
    color: STATUS.idle,
    rows: [{ title: 'blocked on review', provider: 'claude', status: 'idle' }],
  },
  {
    id: 'failed',
    label: 'Failed',
    count: 1,
    color: STATUS.exited,
    rows: [{ title: 'deploy · payments-svc', provider: 'codex', status: 'exited' }],
  },
  {
    id: 'budget_warn',
    label: 'Budget warn',
    count: 2,
    color: STATUS.needsYou,
    rows: [{ title: 'swarm · over 80% cap', provider: 'claude', status: 'needsYou' }],
  },
];

const SAVED_VIEWS = ['All workspaces', 'My turn', 'Failing now', 'Over budget'];

const BucketCard: React.FC<{ b: Bucket }> = ({ b }) => (
  <div
    style={{
      background: T.surface,
      border: `1px solid ${alpha(b.color, 0.45)}`,
      borderRadius: 12,
      padding: 14,
      display: 'flex',
      flexDirection: 'column',
      gap: 10,
      boxShadow: `inset 0 2px 0 ${alpha(b.color, 0.5)}`,
    }}
  >
    <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
      <span style={{ width: 9, height: 9, borderRadius: '50%', background: b.color, boxShadow: `0 0 8px ${alpha(b.color, 0.8)}` }} />
      <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>{b.label}</span>
      <span
        style={{
          minWidth: 28,
          height: 26,
          padding: '0 8px',
          borderRadius: 8,
          background: alpha(b.color, 0.18),
          color: b.color,
          fontFamily: fonts.ui,
          fontSize: 16,
          fontWeight: 800,
          display: 'grid',
          placeItems: 'center',
        }}
      >
        {b.count}
      </span>
    </div>
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      {b.rows.map((r, i) => (
        <div
          key={i}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '7px 10px',
            borderRadius: 8,
            background: T.surface2,
            border: `1px solid ${T.border}`,
          }}
        >
          <StatusDot kind={r.status} size={8} />
          <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 12.5, color: T.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {r.title}
          </span>
          <span style={{ fontFamily: fonts.ui, fontSize: 10.5, color: T.textDim }}>{r.provider}</span>
        </div>
      ))}
    </div>
  </div>
);

const MissionControlScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={<Navigator active="agents" workingCount={5} />} title="Otto — Mission Control">
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 18, gap: 14, boxSizing: 'border-box' }}>
          {/* header + saved views */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Icon name="gauge" size={18} color={brand.cyan} />
            <span style={{ fontFamily: fonts.ui, fontSize: 19, fontWeight: 750 as never, color: '#fff' }}>Mission Control</span>
            <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>· every workspace</span>
            <div style={{ flex: 1 }} />
            <Appear delay={40} y={0} style={{ display: 'flex', gap: 7 }}>
              <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim, alignSelf: 'center', marginRight: 2 }}>Saved views</span>
              {SAVED_VIEWS.map((v, i) => (
                <Chip key={v} tone={i === 1 ? 'accent' : 'default'} color={i === 1 ? brand.cyan : undefined}>
                  {v}
                </Chip>
              ))}
            </Appear>
          </div>

          {/* 6-bucket grid */}
          <Stagger
            delay={12}
            step={6}
            y={18}
            style={{ flex: 1, display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gridTemplateRows: '1fr 1fr', gap: 14 }}
            childStyle={{ minHeight: 0 }}
          >
            {BUCKETS.map((b) => (
              <BucketCard key={b.id} b={b} />
            ))}
          </Stagger>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Mission Control — every workspace, six buckets"
      sub="needs-you · working · review-ready · waiting · failed · budget · saved views."
    />
  </>
);

// ── Scene 4 — Theming + capability health ────────────────────────────────────
const THEME_SWATCHES: { label: string; t: Theme }[] = [
  { label: 'Native', t: themes.nativeDark },
  { label: 'Pro Dark', t: themes.proDark },
  { label: 'Warm', t: themes.warmDark },
];

const ThemeSwatch: React.FC<{ label: string; t: Theme; active?: boolean }> = ({ label, t, active }) => (
  <div
    style={{
      borderRadius: 12,
      padding: 9,
      background: alpha('#000', 0.2),
      border: `1.5px solid ${active ? brand.cyan : T.border}`,
      boxShadow: active ? `0 0 0 3px ${alpha(brand.cyan, 0.22)}` : 'none',
      flex: 1,
    }}
  >
    {/* mini mock window in this theme */}
    <div style={{ borderRadius: 8, overflow: 'hidden', border: `1px solid ${t.border}`, background: t.bg }}>
      <div style={{ height: 16, background: t.bgSidebar, display: 'flex', alignItems: 'center', gap: 3, padding: '0 6px' }}>
        <span style={{ width: 5, height: 5, borderRadius: '50%', background: '#ff5f57' }} />
        <span style={{ width: 5, height: 5, borderRadius: '50%', background: '#febc2e' }} />
        <span style={{ width: 5, height: 5, borderRadius: '50%', background: '#28c840' }} />
      </div>
      <div style={{ display: 'flex', height: 56 }}>
        <div style={{ width: 26, background: t.bgSidebar, borderRight: `1px solid ${t.border}`, padding: 5, display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span style={{ height: 5, borderRadius: 2, background: t.accent }} />
          <span style={{ height: 5, borderRadius: 2, background: alpha(t.textDim, 0.5) }} />
          <span style={{ height: 5, borderRadius: 2, background: alpha(t.textDim, 0.5) }} />
        </div>
        <div style={{ flex: 1, padding: 7, display: 'flex', flexDirection: 'column', gap: 5 }}>
          <span style={{ height: 6, width: '70%', borderRadius: 3, background: t.surface2 }} />
          <span style={{ height: 6, width: '55%', borderRadius: 3, background: t.surface2 }} />
          <span style={{ height: 6, width: '40%', borderRadius: 3, background: alpha(t.accent, 0.7) }} />
        </div>
      </div>
    </div>
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 8 }}>
      <span style={{ width: 11, height: 11, borderRadius: '50%', background: t.accent }} />
      <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: active ? 700 : 600, color: active ? '#fff' : T.text }}>{label}</span>
      {active && <Icon name="check" size={13} color={brand.cyan} style={{ marginLeft: 'auto' }} />}
    </div>
  </div>
);

type Health = { label: string; icon: string; state: 'ready' | 'degraded' | 'missing'; note: string };
const HEALTH: Health[] = [
  { label: 'Agents', icon: 'terminal', state: 'ready', note: '5 sessions' },
  { label: 'Message Brokers', icon: 'box', state: 'ready', note: '2 clusters' },
  { label: 'Database', icon: 'db', state: 'ready', note: '4 connections' },
  { label: 'Insights', icon: 'gauge', state: 'missing', note: 'ClickHouse not configured' },
];

const stateColor = (s: Health['state']) =>
  s === 'ready' ? STATUS.working : s === 'degraded' ? STATUS.needsYou : STATUS.needsYou;
const stateLabel = (s: Health['state']) => (s === 'ready' ? 'Ready' : s === 'degraded' ? 'Degraded' : 'Missing setup');

const HealthRow: React.FC<{ h: Health }> = ({ h }) => {
  const c = stateColor(h.state);
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 11,
        padding: '11px 13px',
        borderRadius: 10,
        background: T.surface2,
        border: `1px solid ${h.state === 'ready' ? T.border : alpha(c, 0.4)}`,
      }}
    >
      <span style={{ width: 28, height: 28, borderRadius: 8, background: alpha(c, 0.16), display: 'grid', placeItems: 'center', color: c }}>
        <Icon name={h.icon} size={15} />
      </span>
      <div style={{ flex: 1 }}>
        <div style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 650 as never, color: T.text }}>{h.label}</div>
        <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim }}>{h.note}</div>
      </div>
      <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 700, color: c }}>
        {h.state === 'ready' ? '✓ ' : ''}
        {stateLabel(h.state)}
      </span>
      {h.state !== 'ready' && (
        <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 700, color: brand.cyan, padding: '4px 10px', borderRadius: 7, background: alpha(brand.cyan, 0.14) }}>
          Fix →
        </span>
      )}
    </div>
  );
};

const ThemeHealthScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={<Navigator active="settings" />} title="Otto — Settings · Appearance & Health">
        <div style={{ display: 'flex', height: '100%', gap: 16, padding: 18, boxSizing: 'border-box' }}>
          {/* LEFT — theming */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <Appear delay={6} y={10}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
                <Icon name="gear" size={16} color={brand.violet} />
                <span style={{ fontFamily: fonts.ui, fontSize: 16, fontWeight: 700, color: '#fff' }}>Theme</span>
              </div>
            </Appear>
            <Stagger delay={12} step={6} y={16} style={{ display: 'flex', gap: 11 }} childStyle={{ flex: 1, display: 'flex' }}>
              {THEME_SWATCHES.map((s, i) => (
                <ThemeSwatch key={s.label} label={s.label} t={s.t} active={i === 1} />
              ))}
            </Stagger>
            <Appear delay={30} y={12}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginTop: 4 }}>
                <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>Appearance</span>
                <Segmented options={['Light', 'Dark', 'Auto']} active={1} />
                <div style={{ flex: 1 }} />
                <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>RTL</span>
                <Toggle on />
              </div>
            </Appear>
            <Appear delay={38} y={12}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '11px 13px', borderRadius: 10, background: T.surface2, border: `1px solid ${T.border}`, marginTop: 2 }}>
                <Icon name="archive" size={15} color={T.textDim} />
                <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13, color: T.text }}>Settings backup &amp; restore</span>
                <Chip color={brand.cyan}>Export</Chip>
                <Chip>Restore</Chip>
              </div>
            </Appear>
          </div>

          {/* RIGHT — capability health */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 12 }}>
            <Appear delay={10} y={10}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
                <Icon name="gauge" size={16} color={STATUS.working} />
                <span style={{ fontFamily: fonts.ui, fontSize: 16, fontWeight: 700, color: '#fff' }}>Capability health</span>
              </div>
            </Appear>
            <Stagger delay={16} step={6} y={14} style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
              {HEALTH.map((h) => (
                <HealthRow key={h.label} h={h} />
              ))}
            </Stagger>
            <Appear delay={44} y={12}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '11px 13px', borderRadius: 10, background: alpha(brand.violet, 0.1), border: `1px solid ${alpha(brand.violet, 0.3)}`, marginTop: 2 }}>
                <Icon name="refresh" size={15} color={brand.cyan} />
                <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13, color: T.text }}>CLI auto-update · daily 03:00</span>
                <Chip tone="ok">On</Chip>
              </div>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="Make it yours — and keep it healthy"
      sub="3 themes × light/dark + RTL · capability health · daily CLI auto-update."
    />
  </>
);

// ── Scene 5 — cross-module search ────────────────────────────────────────────
type SearchGroup = { group: string; icon: string; color: string; items: string[] };
const SEARCH_GROUPS: SearchGroup[] = [
  { group: 'Stories', icon: 'note', color: '#2684ff', items: ['PAY-318 · Refund flow rounding', 'PAY-204 · Payout webhook retries'] },
  { group: 'Repos', icon: 'branch', color: '#28c840', items: ['payments-svc · main', 'payments-web · feat/checkout'] },
  { group: 'Workflows', icon: 'split', color: '#9ee039', items: ['payments · nightly reconcile'] },
  { group: 'Clusters', icon: 'box', color: '#febc2e', items: ['msk-prod · payments.events'] },
];

const SearchScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={<Navigator active="agents" workingCount={5} />} title="Otto — Search">
        <div style={{ display: 'flex', flexDirection: 'column', height: '100%', padding: 20, gap: 16, boxSizing: 'border-box', alignItems: 'center' }}>
          {/* search bar */}
          <Appear delay={4} y={-12} style={{ width: '74%', maxWidth: 820 }}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 13,
                padding: '16px 20px',
                borderRadius: 14,
                background: T.surface,
                border: `1px solid ${alpha(brand.cyan, 0.45)}`,
                boxShadow: `0 0 0 4px ${alpha(brand.cyan, 0.14)}`,
              }}
            >
              <Icon name="search" size={21} color={brand.cyan} />
              <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 21, fontWeight: 600, color: '#fff' }}>
                payments
                <Caret color={brand.cyan} h={22} />
              </span>
              <Chip color={brand.violet}>Everywhere</Chip>
            </div>
          </Appear>

          {/* grouped results */}
          <Stagger
            delay={14}
            step={6}
            y={16}
            style={{ width: '74%', maxWidth: 820, display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 13 }}
          >
            {SEARCH_GROUPS.map((g) => (
              <div key={g.group} style={{ background: T.surface, border: `1px solid ${T.border}`, borderRadius: 12, padding: 14 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 9 }}>
                  <span style={{ width: 22, height: 22, borderRadius: 7, background: alpha(g.color, 0.16), display: 'grid', placeItems: 'center', color: g.color }}>
                    <Icon name={g.icon} size={13} />
                  </span>
                  <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 700, color: T.text }}>{g.group}</span>
                  <span style={{ fontFamily: fonts.ui, fontSize: 11, fontWeight: 700, color: g.color }}>{g.items.length}</span>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  {g.items.map((it, i) => (
                    <div key={i} style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim, padding: '5px 9px', borderRadius: 7, background: T.surface2 }}>
                      {it}
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </Stagger>

          <Appear delay={40} y={10}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>
              <span>stories · workflows · API · swarm · memories · repos · brokers</span>
              <Keys keys={['⌘', 'F']} />
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption step={4} title="Search across everything" sub="Stories, repos, workflows, clusters, memories — one query." />
  </>
);

// ── Scene 6 — outro ──────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <Title />, name: 'Title' },
  { dur: 210, node: <PaletteScene />, name: 'Command palette' },
  { dur: 220, node: <MissionControlScene />, name: 'Mission Control' },
  { dur: 180, node: <ThemeHealthScene />, name: 'Theming + health' },
  { dur: 80, node: <SearchScene />, name: 'Search' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Power & Polish"
        tagline="The little things, done right."
        pills={[
          { label: '⌘K palette', color: brand.cyan, icon: 'command' },
          { label: 'Mission Control', color: '#0a84ff', icon: 'grid' },
          { label: 'Cross-search', color: brand.violet, icon: 'search' },
          { label: 'Themes + RTL', color: '#febc2e', icon: 'gear' },
          { label: 'Auto-update', color: '#28c840', icon: 'refresh' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const platformDuration = scenesDuration(SCENES);
export const Platform: React.FC = () => <Scenes scenes={SCENES} />;
