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

// ─── Vault — workspace knowledge store walkthrough — ~30s ────────────────────
// Obsidian-like memory vault: notes with [[backlinks]], keyword/semantic search,
// SVG knowledge graph.
// ─────────────────────────────────────────────────────────────────────────────

const TITLE_DUR  = 75;
const S1_DUR     = 180;  // note list + search
const S2_DUR     = 210;  // note reader with backlinks
const S3_DUR     = 180;  // knowledge graph
const OUTRO_DUR  = 90;

const S1_START   = TITLE_DUR;
const S2_START   = S1_START + S1_DUR;
const S3_START   = S2_START + S2_DUR;
const OUTRO_START = S3_START + S3_DUR;

// ─── Helpers ─────────────────────────────────────────────────────────────────

function typewriter(text: string, frame: number, cps = 20): string {
  const chars = Math.floor((frame / 30) * cps);
  return text.slice(0, Math.min(chars, text.length));
}

// ─── Memory item kinds + colors ───────────────────────────────────────────────
const KIND_COLOR: Record<string, string> = {
  entity:      '#6ea8fe',
  decision:    '#63e6be',
  requirement: '#ffa94d',
  qa:          '#da77f2',
  chunk:       '#8b97a8',
};

const KindChip: React.FC<{ kind: string }> = ({ kind }) => (
  <span style={{ fontFamily: theme.mono, fontSize: 11, fontWeight: 700, color: KIND_COLOR[kind] ?? theme.textDim, background: `${KIND_COLOR[kind] ?? theme.textDim}22`, border: `1px solid ${KIND_COLOR[kind] ?? theme.textDim}44`, borderRadius: 6, padding: '2px 8px', letterSpacing: 0.4, textTransform: 'uppercase' }}>
    {kind}
  </span>
);

// ─── Vault sidebar ────────────────────────────────────────────────────────────
const NOTES = [
  { title: 'Player onboarding flow',     kind: 'entity',      preview: 'Steps from registration to first deposit…' },
  { title: 'Bonus engine decision',      kind: 'decision',    preview: 'Chose tier-based granting over flat rate…' },
  { title: 'KYC requirements 2024',      kind: 'requirement', preview: 'Documents required per jurisdiction…' },
  { title: 'Q: How does re-activation work?', kind: 'qa',     preview: 'A: Sends email + unlocks wallet on…' },
  { title: 'Wallet architecture notes',  kind: 'entity',      preview: 'Unified wallet — no legacy brandId+500k…' },
];

const VaultSidebar: React.FC<{ active?: number; searchVal?: string }> = ({ active = 0, searchVal = '' }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <div style={{ width: 300, background: theme.surface, borderRight: `1px solid ${theme.border}`, display: 'flex', flexDirection: 'column', height: '100%', flexShrink: 0 }}>
      <div style={{ padding: '14px 16px 12px', borderBottom: `1px solid ${theme.border}` }}>
        <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 1.2, textTransform: 'uppercase', marginBottom: 8 }}>Vault</div>
        <div style={{ background: theme.surface2, borderRadius: 8, padding: '8px 12px', display: 'flex', alignItems: 'center', gap: 8, border: `1px solid ${searchVal ? theme.accent : theme.border}`, boxShadow: searchVal ? `0 0 0 2px ${theme.accent}22` : 'none' }}>
          <span style={{ color: theme.textDim, fontSize: 14 }}>⌕</span>
          <span style={{ color: searchVal ? theme.text : theme.textDim, fontFamily: theme.mono, fontSize: 13 }}>{searchVal || 'Search memory…'}</span>
        </div>
        <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
          {(['Index', 'Graph'] as const).map((m) => (
            <div key={m} style={{ flex: 1, textAlign: 'center', padding: '6px 0', borderRadius: 8, background: m === 'Index' ? `${theme.accent}18` : 'transparent', border: `1px solid ${m === 'Index' ? theme.accent : theme.border}`, color: m === 'Index' ? theme.accent : theme.textDim, fontFamily: theme.font, fontSize: 12, fontWeight: m === 'Index' ? 700 : 400 }}>{m}</div>
          ))}
        </div>
      </div>
      <div style={{ flex: 1, overflow: 'hidden', padding: '8px 0' }}>
        {NOTES.map((note, i) => {
          const s = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
          const isActive = i === active;
          return (
            <div key={note.title} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [-10, 0])}px)`, padding: '10px 16px', background: isActive ? `${theme.accent}14` : 'transparent', borderLeft: isActive ? `2px solid ${theme.accent}` : '2px solid transparent', cursor: 'pointer' }}>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8, marginBottom: 4 }}>
                <span style={{ fontFamily: theme.font, fontSize: 13, color: isActive ? theme.text : theme.textDim, fontWeight: isActive ? 700 : 400, flex: 1, lineHeight: 1.3 }}>{note.title}</span>
                <KindChip kind={note.kind} />
              </div>
              <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 12, lineHeight: 1.4, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>{note.preview}</div>
            </div>
          );
        })}
      </div>
    </div>
  );
};

// ─── Scene 1 – Note list + semantic search ────────────────────────────────────
const Scene1List: React.FC = () => {
  const frame = useCurrentFrame();
  const SEARCH_START = 70;
  const searchVal = frame >= SEARCH_START ? typewriter('bonus engine', frame - SEARCH_START, 14) : '';

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <VaultSidebar active={0} searchVal={searchVal} />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', padding: 40 }}>
        <Appear delay={10}>
          <div style={{ textAlign: 'center', maxWidth: 500 }}>
            <div style={{ fontSize: 56, marginBottom: 20 }}>🧠</div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 28, fontWeight: 800, marginBottom: 12 }}>Your workspace memory</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 17, lineHeight: 1.6 }}>
              Capture decisions, notes, and context. Otto recalls them automatically — keyword or semantic search across everything.
            </div>
          </div>
        </Appear>
        <Appear delay={40}>
          <div style={{ marginTop: 36, display: 'flex', gap: 12 }}>
            {[
              { icon: '📝', label: 'Notes & decisions' },
              { icon: '🔗', label: '[[Backlinks]]' },
              { icon: '🔍', label: 'Semantic recall' },
            ].map(({ icon, label }) => (
              <div key={label} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '10px 18px', borderRadius: 10, background: theme.surface2, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 14 }}>
                <span style={{ fontSize: 16 }}>{icon}</span> {label}
              </div>
            ))}
          </div>
        </Appear>
      </div>

      <Caption step={1} title="Workspace knowledge store" sub="Notes, decisions, and context — all searchable" delay={50} />
    </div>
  );
};

// ─── Scene 2 – Note reader with [[backlinks]] ─────────────────────────────────
const NOTE_BODY = `# Bonus engine decision

After evaluating flat-rate vs tier-based granting we chose
**tier-based granting** to allow dynamic adjustment per brand.

## Related
- [[Player onboarding flow]] — trigger point for first bonus
- [[Wallet architecture notes]] — ledger that records grants
- [[KYC requirements 2024]] — blocks bonus until verified

## Rationale
Flat-rate was simpler but couldn't accommodate per-brand
caps required by compliance in EU markets.`;

const Scene2Note: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // Highlight [[backlinks]] as they appear in the rendered note
  const LINK_PULSE = Math.abs(Math.sin((frame / 30) * Math.PI * 0.8)) * 0.4 + 0.6;

  const renderNote = (body: string) => {
    return body.split('\n').map((line, i) => {
      // Heading
      if (line.startsWith('## ')) return <div key={i} style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 15, fontWeight: 700, letterSpacing: 0.5, textTransform: 'uppercase', margin: '18px 0 6px' }}>{line.slice(3)}</div>;
      if (line.startsWith('# '))  return <div key={i} style={{ color: theme.text, fontFamily: theme.font, fontSize: 22, fontWeight: 800, marginBottom: 12 }}>{line.slice(2)}</div>;
      if (line === '') return <div key={i} style={{ height: 8 }} />;

      // Parse backlinks
      const parts = line.split(/(\[\[[^\]]+\]\])/g);
      return (
        <div key={i} style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 15, lineHeight: 1.7, marginBottom: 2 }}>
          {parts.map((part, j) => {
            if (part.startsWith('[[') && part.endsWith(']]')) {
              const label = part.slice(2, -2);
              return (
                <span key={j} style={{ color: theme.accent, fontWeight: 700, background: `${theme.accent}18`, borderRadius: 4, padding: '1px 6px', border: `1px solid ${theme.accent}44`, opacity: LINK_PULSE }}>
                  {label}
                </span>
              );
            }
            // Bold text
            if (part.includes('**')) {
              return <span key={j} dangerouslySetInnerHTML={{ __html: part.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>') }} style={{ color: theme.text }} />;
            }
            return <span key={j}>{part}</span>;
          })}
        </div>
      );
    });
  };

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <VaultSidebar active={1} />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* note toolbar */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '12px 24px', borderBottom: `1px solid ${theme.border}`, flexShrink: 0 }}>
          <KindChip kind="decision" />
          <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>Bonus engine decision</span>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
            <div style={{ padding: '6px 14px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>Edit</div>
          </div>
        </div>

        {/* note body */}
        <div style={{ flex: 1, overflow: 'hidden', padding: '28px 40px' }}>
          <Appear delay={4}>
            {renderNote(NOTE_BODY)}
          </Appear>
        </div>

        {/* backlinks panel */}
        <Appear delay={60}>
          <div style={{ borderTop: `1px solid ${theme.border}`, padding: '14px 24px', background: theme.surface2, flexShrink: 0 }}>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase', marginBottom: 10 }}>Backlinks</div>
            <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
              {['Player onboarding flow', 'Wallet architecture notes', 'KYC requirements 2024'].map((bl) => (
                <div key={bl} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 12px', borderRadius: 8, background: `${theme.accent}11`, border: `1px solid ${theme.accent}33`, color: theme.accent, fontFamily: theme.font, fontSize: 13, fontWeight: 600 }}>
                  🔗 {bl}
                </div>
              ))}
            </div>
          </div>
        </Appear>
      </div>

      <Caption step={2} title="Notes with [[backlinks]]" sub="Link related decisions — Otto traverses the graph automatically" delay={50} />
    </div>
  );
};

// ─── Scene 3 – Knowledge graph (SVG, circular layout) ────────────────────────
const GRAPH_NODES = [
  { id: 'n0', label: 'Bonus engine decision',     kind: 'decision',    cx: 540, cy: 260 },
  { id: 'n1', label: 'Player onboarding flow',    kind: 'entity',      cx: 280, cy: 160 },
  { id: 'n2', label: 'Wallet architecture',       kind: 'entity',      cx: 800, cy: 180 },
  { id: 'n3', label: 'KYC requirements 2024',     kind: 'requirement', cx: 280, cy: 380 },
  { id: 'n4', label: 'Deposit flow notes',        kind: 'chunk',       cx: 800, cy: 380 },
  { id: 'n5', label: 'Q: Re-activation flow',     kind: 'qa',          cx: 540, cy: 450 },
];
const GRAPH_EDGES = [
  ['n0', 'n1'], ['n0', 'n2'], ['n0', 'n3'], ['n1', 'n3'], ['n2', 'n4'], ['n0', 'n5'], ['n3', 'n5'],
];

const Scene3Graph: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const nodeS = (i: number) => spring({ frame: frame - i * 10, fps, config: { damping: 180 } });
  const edgeS = (i: number) => spring({ frame: frame - (i * 8 + 20), fps, config: { damping: 200 } });

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <VaultSidebar active={0} />
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {/* toggle to graph */}
        <div style={{ display: 'flex', gap: 8, padding: '14px 24px', borderBottom: `1px solid ${theme.border}`, flexShrink: 0 }}>
          {(['Index', 'Graph'] as const).map((m) => (
            <div key={m} style={{ padding: '7px 18px', borderRadius: 8, background: m === 'Graph' ? `${theme.accent}18` : 'transparent', border: `1px solid ${m === 'Graph' ? theme.accent : theme.border}`, color: m === 'Graph' ? theme.accent : theme.textDim, fontFamily: theme.font, fontSize: 13, fontWeight: m === 'Graph' ? 700 : 400 }}>{m}</div>
          ))}
        </div>
        <div style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
          <svg width="100%" height="100%" viewBox="0 0 1080 580" style={{ position: 'absolute', inset: 0 }}>
            {/* edges */}
            {GRAPH_EDGES.map((edge, i) => {
              const from = GRAPH_NODES.find((n) => n.id === edge[0])!;
              const to   = GRAPH_NODES.find((n) => n.id === edge[1])!;
              const s = edgeS(i);
              const mx = (from.cx + to.cx) / 2;
              const my = (from.cy + to.cy) / 2;
              return (
                <line
                  key={i}
                  x1={interpolate(s, [0, 1], [from.cx, from.cx])}
                  y1={from.cy}
                  x2={interpolate(s, [0, 1], [from.cx, to.cx])}
                  y2={interpolate(s, [0, 1], [from.cy, to.cy])}
                  stroke={`${theme.border}`}
                  strokeWidth={1.5}
                  opacity={s}
                />
              );
            })}
            {/* nodes */}
            {GRAPH_NODES.map((node, i) => {
              const s = nodeS(i);
              const color = KIND_COLOR[node.kind] ?? theme.textDim;
              const isCenter = node.id === 'n0';
              const r = isCenter ? 14 : 10;
              return (
                <g key={node.id} opacity={s} transform={`translate(${node.cx}, ${node.cy}) scale(${interpolate(s, [0, 1], [0.3, 1])})`}>
                  <circle r={r} fill={`${color}22`} stroke={color} strokeWidth={isCenter ? 2.5 : 1.5} />
                  {isCenter && <circle r={5} fill={color} />}
                  <text textAnchor="middle" y={r + 18} fill={color} fontFamily={theme.font} fontSize={isCenter ? 13 : 11} fontWeight={isCenter ? 700 : 400}>
                    {node.label.length > 24 ? node.label.slice(0, 22) + '…' : node.label}
                  </text>
                </g>
              );
            })}
          </svg>
        </div>
      </div>

      <Caption step={3} title="Knowledge graph" sub="See how decisions, entities, and Q&A connect at a glance" delay={50} />
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
      <div style={{ opacity: t1, transform: `scale(${interpolate(t1, [0, 1], [0.5, 1])})`, fontSize: 80 }}>🧠</div>
      <div style={{ opacity: t2, transform: `translateY(${interpolate(t2, [0, 1], [24, 0])}px)`, color: theme.text, fontFamily: theme.font, fontSize: 64, fontWeight: 800, textAlign: 'center' }}>
        Memory that grows with you.
      </div>
      <div style={{ opacity: t3, transform: `translateY(${interpolate(t3, [0, 1], [16, 0])}px)`, color: theme.textDim, fontFamily: theme.font, fontSize: 24, textAlign: 'center' }}>
        Notes · [[Backlinks]] · Semantic search · Knowledge graph
      </div>
    </div>
  );
};

// ─── Root composition ─────────────────────────────────────────────────────────
export const Vault: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>

      <Sequence durationInFrames={TITLE_DUR}>
        <TitleCard kicker="OTTO ADE" title="Vault" subtitle="Your workspace knowledge store" />
      </Sequence>

      <Sequence from={S1_START} durationInFrames={S1_DUR + S2_DUR + S3_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Vault">
            <Sequence durationInFrames={S1_DUR}>
              <Scene1List />
            </Sequence>
            <Sequence from={S1_DUR} durationInFrames={S2_DUR}>
              <Scene2Note />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR} durationInFrames={S3_DUR}>
              <Scene3Graph />
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
