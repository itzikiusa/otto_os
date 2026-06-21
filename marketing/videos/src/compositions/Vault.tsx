import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  TitleCard,
  Caption,
  Chip,
  Field,
  Card,
  Icon,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  KNOWLEDGE VAULT — workspace knowledge store with hybrid recall
// ════════════════════════════════════════════════════════════════════════════

// kind → chip color (entity / decision / requirement / qa / chunk)
const KIND: Record<string, string> = {
  entity: brand.cyan,
  decision: brand.violet,
  requirement: '#febc2e',
  qa: '#28c840',
  chunk: '#98989f',
};

const KindChip: React.FC<{ kind: keyof typeof KIND }> = ({ kind }) => (
  <Chip color={KIND[kind]} style={{ height: 19, fontSize: 11, textTransform: 'uppercase', letterSpacing: 0.4 }}>
    {kind}
  </Chip>
);

const SectionLabel: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div
    style={{
      fontFamily: fonts.ui,
      fontSize: 11,
      fontWeight: 700,
      letterSpacing: 0.7,
      textTransform: 'uppercase',
      color: T.textDim,
      marginBottom: 10,
    }}
  >
    {children}
  </div>
);

// ── Scene 1 — title card ──────────────────────────────────────────────────────
const Title: React.FC = () => (
  <TitleCard
    kicker="Knowledge Vault"
    title="Your team's memory, searchable"
    subtitle="Notes, backlinks & semantic recall — fed straight to your agents"
  />
);

// ── Scene 2 — notes list + note reader ───────────────────────────────────────
const NOTES: { title: string; kind: keyof typeof KIND }[] = [
  { title: 'JWT auth flow', kind: 'decision' },
  { title: 'Player balance model', kind: 'entity' },
  { title: 'Withdrawal limits', kind: 'requirement' },
  { title: 'Why ClickHouse?', kind: 'qa' },
];

const NoteRow: React.FC<{ title: string; kind: keyof typeof KIND; active?: boolean }> = ({ title, kind, active }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      height: 44,
      padding: '0 12px',
      borderRadius: 8,
      background: active ? alpha(T.accent, 0.12) : 'transparent',
      border: `1px solid ${active ? alpha(T.accent, 0.4) : 'transparent'}`,
    }}
  >
    <Icon name="note" size={15} color={active ? T.accent : T.textDim} />
    <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 14.5, fontWeight: active ? 600 : 500, color: T.text }}>
      {title}
    </span>
    <KindChip kind={kind} />
  </div>
);

// inline backlink token rendered inside the prose
const Backlink: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <span
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 4,
      padding: '0 6px',
      margin: '0 1px',
      borderRadius: 5,
      background: alpha(brand.cyan, 0.14),
      border: `1px solid ${alpha(brand.cyan, 0.4)}`,
      color: brand.cyan,
      fontWeight: 600,
    }}
  >
    <Icon name="link" size={12} color={brand.cyan} />
    {children}
  </span>
);

const NotesScene: React.FC = () => (
  <>
    <Stage scale={0.92}>
      <OttoWindow nav={<Navigator active="vault" />} title="Otto — Knowledge Vault · sinatra-wallet">
        <div style={{ display: 'flex', height: '100%', boxSizing: 'border-box' }}>
          {/* left — notes list */}
          <div
            style={{
              width: 420,
              flexShrink: 0,
              borderRight: `1px solid ${T.border}`,
              padding: 18,
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              background: alpha('#fff', 0.012),
            }}
          >
            <Appear delay={4} y={10}>
              <Field placeholder="Search the vault…" icon="search" focused />
            </Appear>
            <Appear delay={8} y={8} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginTop: 4 }}>
              <SectionLabel>Notes · 128</SectionLabel>
              <Chip color={brand.cyan} style={{ height: 19, fontSize: 11 }}>collection · wallet</Chip>
            </Appear>
            <Stagger delay={14} step={7} y={12} style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              {NOTES.map((n, i) => (
                <NoteRow key={n.title} title={n.title} kind={n.kind} active={i === 1} />
              ))}
            </Stagger>
          </div>

          {/* right — note reader */}
          <div style={{ flex: 1, minWidth: 0, padding: 26, display: 'flex', flexDirection: 'column', gap: 16 }}>
            <Appear delay={20} y={12}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
                <span style={{ fontFamily: fonts.ui, fontSize: 26, fontWeight: 750 as never, color: T.text, letterSpacing: -0.4 }}>
                  Player balance model
                </span>
                <KindChip kind="entity" />
                <Chip color={status.working} style={{ height: 21 }}>
                  <Icon name="check" size={12} color={status.working} /> accepted
                </Chip>
              </div>
            </Appear>
            <Appear delay={26} y={12}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14, fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                  <Icon name="globe" size={13} color={T.textDim} /> collection · wallet
                </span>
                <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                  <Icon name="link" size={13} color={T.textDim} /> 6 backlinks
                </span>
                <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                  <Icon name="clock" size={13} color={T.textDim} /> updated 2d ago
                </span>
              </div>
            </Appear>
            <Appear delay={32} y={14} style={{ flex: 1 }}>
              <Card pad={22} style={{ height: '100%', boxSizing: 'border-box' }}>
                <div style={{ fontFamily: fonts.ui, fontSize: 16.5, lineHeight: 1.85, color: alpha(T.text, 0.92) }}>
                  The <Backlink>Player balance model</Backlink> unifies cash + bonus into a single ledger row
                  (<span style={{ fontFamily: fonts.mono, fontSize: 14, color: brand.cyan }}>latestBalance</span>,{' '}
                  <span style={{ fontFamily: fonts.mono, fontSize: 14, color: brand.cyan }}>bonusBalance</span>). Every
                  bet/win mutates it atomically via the <Backlink>Wallet gateway</Backlink>, and{' '}
                  <Backlink>Withdrawal limits</Backlink> read it before a payout clears.
                  <br />
                  <br />
                  Decided in <Backlink>JWT auth flow</Backlink> review — see{' '}
                  <Backlink>Why ClickHouse?</Backlink> for why aggregates live elsewhere.
                </div>
                <div
                  style={{
                    marginTop: 22,
                    paddingTop: 16,
                    borderTop: `1px solid ${T.border}`,
                    display: 'flex',
                    gap: 18,
                    fontFamily: fonts.ui,
                    fontSize: 12.5,
                    color: T.textDim,
                  }}
                >
                  <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Icon name="merge" size={14} color={T.textDim} /> Merge
                  </span>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Icon name="split" size={14} color={T.textDim} /> Split
                  </span>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <Icon name="archive" size={14} color={T.textDim} /> Forget · undo
                  </span>
                </div>
              </Card>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Notes with [[backlinks]] & a lifecycle"
      sub="entity · decision · requirement · qa — suggested → accepted → stale"
    />
  </>
);

// ── Scene 3 — hybrid search (keyword + semantic, RRF-fused) ───────────────────
const HITS: { title: string; kind: keyof typeof KIND; snippet: string; keyword: boolean; semantic: boolean; score: string }[] = [
  {
    title: 'Withdrawal limits',
    kind: 'requirement',
    snippet: 'daily cashout capped at €2,000 · KYC tier gates the ceiling',
    keyword: true,
    semantic: true,
    score: '0.94',
  },
  {
    title: 'Player balance model',
    kind: 'entity',
    snippet: 'payout reads latestBalance before a withdrawal clears',
    keyword: false,
    semantic: true,
    score: '0.81',
  },
  {
    title: 'Why ClickHouse?',
    kind: 'qa',
    snippet: 'cashout velocity checks run off the hourly aggregate',
    keyword: false,
    semantic: true,
    score: '0.67',
  },
];

const MatchBadge: React.FC<{ kind: 'keyword' | 'semantic'; on: boolean }> = ({ kind, on }) => {
  const c = kind === 'keyword' ? '#0a84ff' : brand.violet;
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 5,
        height: 22,
        padding: '0 9px',
        borderRadius: 999,
        fontFamily: fonts.ui,
        fontSize: 11.5,
        fontWeight: 600,
        color: on ? c : T.textDim,
        background: on ? alpha(c, 0.14) : alpha(T.textDim, 0.08),
        border: `1px solid ${on ? alpha(c, 0.4) : T.border}`,
        opacity: on ? 1 : 0.5,
      }}
    >
      <Icon name={kind === 'keyword' ? 'search' : 'zap'} size={12} color={on ? c : T.textDim} />
      {kind}
    </span>
  );
};

const HitRow: React.FC<{ hit: (typeof HITS)[number]; rank: number }> = ({ hit, rank }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 16,
      padding: '14px 18px',
      borderRadius: 10,
      background: T.surface,
      border: `1px solid ${T.border}`,
    }}
  >
    <span
      style={{
        width: 26,
        height: 26,
        borderRadius: 8,
        flexShrink: 0,
        display: 'grid',
        placeItems: 'center',
        background: alpha(brand.cyan, 0.16),
        color: brand.cyan,
        fontFamily: fonts.ui,
        fontSize: 13,
        fontWeight: 700,
      }}
    >
      {rank}
    </span>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span style={{ fontFamily: fonts.ui, fontSize: 16, fontWeight: 650 as never, color: T.text }}>{hit.title}</span>
        <KindChip kind={hit.kind} />
      </div>
      <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginTop: 4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {hit.snippet}
      </div>
    </div>
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
      <MatchBadge kind="keyword" on={hit.keyword} />
      <MatchBadge kind="semantic" on={hit.semantic} />
    </div>
    <div style={{ width: 70, textAlign: 'right', flexShrink: 0 }}>
      <div style={{ fontFamily: fonts.mono, fontSize: 15, fontWeight: 700, color: brand.cyan }}>{hit.score}</div>
      <div style={{ fontFamily: fonts.ui, fontSize: 10.5, color: T.textDim, letterSpacing: 0.4 }}>RRF</div>
    </div>
  </div>
);

const SearchScene: React.FC = () => (
  <>
    <Stage scale={0.92}>
      <OttoWindow nav={<Navigator active="vault" />} title="Otto — Knowledge Vault · search">
        <div style={{ padding: 30, height: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', gap: 20 }}>
          <Appear delay={4} y={12}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                height: 54,
                padding: '0 18px',
                borderRadius: 12,
                background: T.surface2,
                border: `1px solid ${T.accent}`,
                boxShadow: `0 0 0 3px ${alpha(T.accent, 0.18)}`,
              }}
            >
              <Icon name="search" size={20} color={T.accent} />
              <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 18, color: T.text }}>
                how are withdrawals limited?
              </span>
              <Chip color={brand.violet} style={{ height: 24 }}>hybrid</Chip>
            </div>
          </Appear>
          <Appear delay={10} y={8} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <SectionLabel>3 results · keyword + semantic, fused</SectionLabel>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
              <Icon name="zap" size={14} color={brand.violet} /> reciprocal-rank fusion · 12&nbsp;ms
            </div>
          </Appear>
          <Stagger delay={16} step={10} y={16} style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            {HITS.map((h, i) => (
              <HitRow key={h.title} hit={h} rank={i + 1} />
            ))}
          </Stagger>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Keyword + semantic search, fused"
      sub="RRF hybrid recall — finds it even when wording differs"
    />
  </>
);

// ── Scene 4 — knowledge graph + recall brief ─────────────────────────────────
type GNode = { id: string; x: number; y: number; label: string; color: string; r: number };

const NODES: GNode[] = [
  { id: 'balance', x: 168, y: 150, label: 'Balance model', color: brand.cyan, r: 30 },
  { id: 'wallet', x: 60, y: 70, label: 'Wallet gateway', color: brand.violet, r: 22 },
  { id: 'limits', x: 290, y: 64, label: 'Withdrawal limits', color: '#febc2e', r: 22 },
  { id: 'jwt', x: 56, y: 250, label: 'JWT auth', color: brand.violet, r: 21 },
  { id: 'ch', x: 300, y: 248, label: 'ClickHouse', color: '#28c840', r: 21 },
];
const EDGES: [string, string][] = [
  ['balance', 'wallet'],
  ['balance', 'limits'],
  ['balance', 'jwt'],
  ['balance', 'ch'],
  ['wallet', 'limits'],
];
const NODE_MAP = Object.fromEntries(NODES.map((n) => [n.id, n]));

const Graph: React.FC = () => {
  const frame = useCurrentFrame();
  const W = 360;
  const H = 320;
  return (
    <svg width={W} height={H} style={{ overflow: 'visible' }}>
      {EDGES.map(([a, b], i) => {
        const na = NODE_MAP[a];
        const nb = NODE_MAP[b];
        const draw = track(frame, [10 + i * 4, 28 + i * 4], [0, 1]);
        return (
          <line
            key={i}
            x1={na.x}
            y1={na.y}
            x2={na.x + (nb.x - na.x) * draw}
            y2={na.y + (nb.y - na.y) * draw}
            stroke={alpha(brand.cyan, 0.32)}
            strokeWidth={1.6}
          />
        );
      })}
      {NODES.map((n, i) => {
        const pop = track(frame, [22 + i * 6, 38 + i * 6], [0, 1]);
        const pulse = n.id === 'balance' ? Math.sin(frame / 14) * 1.5 + 1 : 0;
        return (
          <g key={n.id} transform={`translate(${n.x},${n.y}) scale(${pop})`} opacity={pop}>
            <circle r={n.r + 6 + pulse} fill={alpha(n.color, 0.12)} />
            <circle r={n.r} fill={alpha(n.color, 0.22)} stroke={n.color} strokeWidth={1.6} />
            <circle r={n.r * 0.34} fill={n.color} />
            <text
              x={0}
              y={n.r + 17}
              textAnchor="middle"
              fontFamily={fonts.ui}
              fontSize={12}
              fontWeight={600}
              fill={T.text}
            >
              {n.label}
            </text>
          </g>
        );
      })}
    </svg>
  );
};

const BriefLine: React.FC<{ icon: string; color: string; head: string; body: string }> = ({ icon, color, head, body }) => (
  <div style={{ display: 'flex', gap: 11, alignItems: 'flex-start' }}>
    <span style={{ width: 24, height: 24, borderRadius: 7, flexShrink: 0, marginTop: 1, display: 'grid', placeItems: 'center', background: alpha(color, 0.16), color }}>
      <Icon name={icon} size={13} color={color} />
    </span>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 600, color: T.text }}>{head}</div>
      <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim, lineHeight: 1.5, marginTop: 1 }}>{body}</div>
    </div>
  </div>
);

const GraphRecallScene: React.FC = () => (
  <>
    <Stage scale={0.92}>
      <OttoWindow nav={<Navigator active="vault" />} title="Otto — Knowledge Vault · recall">
        <div style={{ display: 'flex', height: '100%', boxSizing: 'border-box' }}>
          {/* left — graph */}
          <div style={{ flex: 1, minWidth: 0, padding: 30, display: 'flex', flexDirection: 'column', borderRight: `1px solid ${T.border}` }}>
            <Appear delay={4} y={8}>
              <SectionLabel>Graph view · entity relationships</SectionLabel>
            </Appear>
            <Appear delay={6} y={10} style={{ flex: 1, display: 'grid', placeItems: 'center' }}>
              <Graph />
            </Appear>
          </div>
          {/* right — recall brief card */}
          <div style={{ width: 540, flexShrink: 0, padding: 28, display: 'flex', flexDirection: 'column' }}>
            <Appear delay={14} y={14} style={{ flex: 1 }}>
              <Card pad={0} style={{ height: '100%', boxSizing: 'border-box', overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
                <div
                  style={{
                    padding: '16px 20px',
                    borderBottom: `1px solid ${T.border}`,
                    background: brand.gradSoft,
                    display: 'flex',
                    alignItems: 'center',
                    gap: 11,
                  }}
                >
                  <Icon name="zap" size={18} color="#fff" />
                  <div style={{ flex: 1 }}>
                    <div style={{ fontFamily: fonts.ui, fontSize: 16, fontWeight: 750 as never, color: '#fff' }}>Recall brief</div>
                    <div style={{ fontFamily: fonts.ui, fontSize: 12, color: alpha('#fff', 0.85) }}>
                      Context for SIN-4821 · 1,840 tokens
                    </div>
                  </div>
                  <Chip color="#fff" style={{ height: 22, background: alpha('#fff', 0.2), borderColor: alpha('#fff', 0.4), color: '#fff' }}>
                    token-budgeted
                  </Chip>
                </div>
                <div style={{ padding: 20, display: 'flex', flexDirection: 'column', gap: 15 }}>
                  <Stagger delay={22} step={7} y={12} style={{ display: 'flex', flexDirection: 'column', gap: 15 }}>
                    <BriefLine
                      icon="note"
                      color={brand.violet}
                      head="Decision · JWT auth flow"
                      body="Withdrawals require a re-validated token; payout path checks balance atomically."
                    />
                    <BriefLine
                      icon="db"
                      color={brand.cyan}
                      head="Entity · Player balance model"
                      body="Unified ledger: latestBalance + bonusBalance, read before any cashout clears."
                    />
                    <BriefLine
                      icon="tag"
                      color="#febc2e"
                      head="Requirement · Withdrawal limits"
                      body="Daily cap €2,000, gated by KYC tier — pulled in at 0.94 relevance."
                    />
                  </Stagger>
                  <div style={{ paddingTop: 14, borderTop: `1px solid ${T.border}`, display: 'flex', alignItems: 'center', gap: 16, flexWrap: 'wrap' }}>
                    <Chip color={status.working} style={{ height: 22 }}>
                      <Icon name="check" size={12} color={status.working} /> no re-fetching
                    </Chip>
                    <Chip color="#0a84ff" style={{ height: 22 }}>
                      <Icon name="file" size={12} color="#0a84ff" /> import AGENTS.md
                    </Chip>
                    <Chip style={{ height: 22 }}>
                      <Icon name="file" size={12} color={T.textDim} /> CLAUDE.md · .cursorrules
                    </Chip>
                  </div>
                </div>
              </Card>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="A recall brief, assembled for each task"
      sub="Token-budgeted context — no re-fetching · import AGENTS.md/CLAUDE.md"
    />
  </>
);

// ── Scene assembly ────────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <Title />, name: 'Title' },
  { dur: 220, node: <NotesScene />, name: 'Notes' },
  { dur: 200, node: <SearchScene />, name: 'Hybrid search' },
  { dur: 150, node: <GraphRecallScene />, name: 'Graph + recall' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Knowledge Vault"
        tagline="Context that compounds."
        pills={[
          { label: 'Backlinks', color: '#47bfff', icon: 'link' },
          { label: 'Hybrid search', color: brand.cyan, icon: 'search' },
          { label: 'Graph view', color: brand.violet, icon: 'globe' },
          { label: 'Governance', color: '#28c840', icon: 'check' },
          { label: 'Recall brief', color: '#0a84ff', icon: 'note' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const vaultDuration = scenesDuration(SCENES);
export const Vault: React.FC = () => <Scenes scenes={SCENES} />;
