import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow, RightPanel } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Field,
  Chip,
  Icon,
  track,
  useTyped,
} from '../components/kit';

// ── Shared atom ───────────────────────────────────────────────────────────────

/** Wiki-style [[backlink]] span, inline. */
const Backlink: React.FC<{ children: string }> = ({ children }) => (
  <span
    style={{
      display: 'inline',
      background: alpha(brand.violet, 0.18),
      border: `1px solid ${alpha(brand.violet, 0.45)}`,
      borderRadius: 4,
      padding: '1px 5px',
      color: '#a78bfa',
      fontFamily: fonts.mono,
      fontSize: '0.88em',
      fontWeight: 600,
    }}
  >
    [[{children}]]
  </span>
);

// ── Scene 1 — Title (~80f) ────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Vault"
    title="Knowledge Vault"
    subtitle="A workspace knowledge store — notes with backlinks, hybrid recall, and a live graph"
  />
);

// ── Scene 2 — Notes + backlinks (~180f) ──────────────────────────────────────

const LINKED_FROM = [
  { title: 'jwt-claims',       sub: 'references claim schema'     },
  { title: 'oauth2-flow',      sub: 'uses token rotation step'    },
  { title: 'security-headers', sub: 'post-rotation header policy' },
];

const LinkedFromPanel: React.FC = () => (
  <div style={{ padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 4 }}>
    <div
      style={{
        fontFamily: fonts.ui,
        fontSize: 11,
        fontWeight: 600,
        letterSpacing: 0.5,
        textTransform: 'uppercase',
        color: T.textDim,
        padding: '0 2px',
        marginBottom: 4,
      }}
    >
      3 notes link here
    </div>
    {LINKED_FROM.map((item, i) => (
      <Appear key={item.title} delay={52 + i * 12} y={8}>
        <div
          style={{
            padding: '8px 10px',
            borderRadius: 7,
            background: T.surface,
            border: `1px solid ${T.border}`,
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 3 }}>
            <Icon name="note" size={12} color={brand.violet} />
            <span
              style={{
                fontFamily: fonts.mono,
                fontSize: 12,
                color: '#a78bfa',
                fontWeight: 600,
              }}
            >
              [[{item.title}]]
            </span>
          </div>
          <div
            style={{
              fontFamily: fonts.ui,
              fontSize: 11.5,
              color: T.textDim,
              marginLeft: 18,
            }}
          >
            {item.sub}
          </div>
        </div>
      </Appear>
    ))}
  </div>
);

const NotesScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="vault" />}
        tabs={[
          { label: 'Auth token rotation', icon: 'note', active: true },
          { label: 'refresh-tokens',      icon: 'note'               },
          { label: 'Team onboarding',     icon: 'note'               },
        ]}
        title="Otto — Vault"
        right={
          <RightPanel title="Linked from (3)" icon="link" width={262}>
            <LinkedFromPanel />
          </RightPanel>
        }
      >
        <div
          style={{
            padding: '22px 30px',
            height: '100%',
            boxSizing: 'border-box',
            overflow: 'hidden',
            display: 'flex',
            flexDirection: 'column',
          }}
        >
          {/* Breadcrumb */}
          <Appear delay={6} y={8}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 5,
                fontFamily: fonts.ui,
                fontSize: 12,
                color: T.textDim,
                marginBottom: 16,
              }}
            >
              <Icon name="globe" size={11} color={T.textDim} />
              <span>vault</span>
              <span style={{ opacity: 0.45 }}>/</span>
              <span>security</span>
              <span style={{ opacity: 0.45 }}>/</span>
              <span style={{ color: T.text, fontWeight: 500 }}>auth-token-rotation</span>
            </div>
          </Appear>

          {/* Note title */}
          <Appear delay={12} y={14}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 28,
                fontWeight: 700,
                color: T.text,
                letterSpacing: -0.4,
                marginBottom: 18,
              }}
            >
              Auth Token Rotation
            </div>
          </Appear>

          {/* First paragraph with inline backlinks */}
          <Appear delay={20} y={10}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 15,
                lineHeight: 1.75,
                color: T.text,
                marginBottom: 16,
              }}
            >
              Tokens must be rotated before expiry to prevent session fixation.
              {' '}The flow is defined in <Backlink>refresh-tokens</Backlink>,
              {' '}with claim structure from <Backlink>jwt-claims</Backlink>.
            </div>
          </Appear>

          {/* Section: Rotation window */}
          <Appear delay={30} y={8}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 18,
                fontWeight: 600,
                color: T.text,
                marginBottom: 8,
                paddingTop: 4,
              }}
            >
              Rotation window
            </div>
          </Appear>

          <Appear delay={36} y={8}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 15,
                lineHeight: 1.7,
                color: alpha('#ffffff', 0.82),
                marginBottom: 14,
              }}
            >
              Rotate when TTL {'<'} 5 min or on each API response (sliding window).
              {' '}Short-lived JWTs (15 min) reduce blast radius on leaked credentials.
            </div>
          </Appear>

          {/* Section: See also */}
          <Appear delay={44} y={8}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 18,
                fontWeight: 600,
                color: T.text,
                marginBottom: 8,
                paddingTop: 4,
              }}
            >
              See also
            </div>
          </Appear>

          <Appear delay={50} y={6}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
              <div style={{ fontFamily: fonts.ui, fontSize: 14, color: T.text, lineHeight: 1.5 }}>
                {'→ '}<Backlink>oauth2-flow</Backlink>{' — the full authorization grant sequence'}
              </div>
              <div style={{ fontFamily: fonts.ui, fontSize: 14, color: T.text, lineHeight: 1.5 }}>
                {'→ '}<Backlink>security-headers</Backlink>{' — required response headers post-rotation'}
              </div>
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="A workspace knowledge store — notes with [[backlinks]]"
      sub="Every note links to related notes. Backlinks build the graph automatically."
    />
  </>
);

// ── Scene 3 — Hybrid recall (~150f) ──────────────────────────────────────────

interface RecallResult {
  title: string;
  excerpt: string;
  score: number;
  kind: 'keyword' | 'semantic';
}

const RECALL_RESULTS: RecallResult[] = [
  { title: 'Auth Token Rotation',   excerpt: '…rotate when TTL < 5 min or on each API response (sliding…',      score: 0.94, kind: 'keyword'  },
  { title: 'Token expiry handling', excerpt: '…sliding window reduces unauthorized access window on leaks…',     score: 0.87, kind: 'semantic' },
  { title: 'refresh-tokens',        excerpt: '…rotate() → issue short-lived JWT, revoke old refresh token…',    score: 0.82, kind: 'keyword'  },
  { title: 'OAuth2 grant flow',     excerpt: '…authorization_code + PKCE; access token issued post-auth…',      score: 0.74, kind: 'semantic' },
  { title: 'JWT best practices',    excerpt: '…sign with RS256, short exp (15 min), rotate refresh regularly…', score: 0.68, kind: 'semantic' },
];

const QUERY = 'how do we rotate refresh tokens?';

const ResultRow: React.FC<{ result: RecallResult; delay: number }> = ({ result, delay }) => {
  const kindColor = result.kind === 'keyword' ? T.accent : brand.violet;
  return (
    <Appear delay={delay} y={10}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 14,
          padding: '10px 14px',
          borderRadius: 8,
          background: T.surface,
          border: `1px solid ${T.border}`,
        }}
      >
        <Icon name="note" size={14} color={T.textDim} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontFamily: fonts.ui,
              fontSize: 14,
              fontWeight: 600,
              color: T.text,
              marginBottom: 2,
            }}
          >
            {result.title}
          </div>
          <div
            style={{
              fontFamily: fonts.mono,
              fontSize: 11.5,
              color: T.textDim,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {result.excerpt}
          </div>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexShrink: 0 }}>
          <Chip color={kindColor}>{result.kind}</Chip>
          <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
            <div
              style={{
                width: 56,
                height: 5,
                borderRadius: 3,
                background: alpha(T.textDim, 0.18),
                overflow: 'hidden',
              }}
            >
              <div
                style={{
                  width: `${result.score * 100}%`,
                  height: '100%',
                  borderRadius: 3,
                  background: kindColor,
                }}
              />
            </div>
            <span
              style={{
                fontFamily: fonts.mono,
                fontSize: 11,
                color: T.textDim,
                minWidth: 28,
              }}
            >
              {result.score.toFixed(2)}
            </span>
          </div>
        </div>
      </div>
    </Appear>
  );
};

const RecallScene: React.FC = () => {
  const frame = useCurrentFrame();
  const query = useTyped(QUERY, 12, 25);

  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow nav={<Navigator active="vault" />} title="Otto — Vault">
          <div
            style={{
              padding: '24px 28px',
              height: '100%',
              boxSizing: 'border-box',
              display: 'flex',
              flexDirection: 'column',
              gap: 14,
            }}
          >
            {/* Search / recall field */}
            <Appear delay={4} y={10}>
              <Field
                label="Vault Recall"
                icon="search"
                value={query || undefined}
                placeholder="Search notes, ask a question, or recall context…"
                focused
                caret={frame < 74}
              />
            </Appear>

            {/* Results header */}
            <Appear delay={68} y={6}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 12,
                  color: T.textDim,
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                }}
              >
                <span>5 results</span>
                <span style={{ opacity: 0.4 }}>·</span>
                <span>
                  <span style={{ color: T.accent, fontWeight: 600 }}>keyword</span>
                  {' + '}
                  <span style={{ color: brand.violet, fontWeight: 600 }}>semantic</span>
                  {' hybrid'}
                </span>
              </div>
            </Appear>

            {/* Result rows */}
            <div
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: 6,
                flex: 1,
                overflow: 'hidden',
              }}
            >
              {RECALL_RESULTS.map((r, i) => (
                <ResultRow key={r.title} result={r} delay={72 + i * 9} />
              ))}
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={2}
        title="Keyword + semantic (vector) hybrid recall"
        sub="Exact text matches and embedding-space similarity — one query, one ranked list."
      />
    </>
  );
};

// ── Scene 4 — Knowledge graph + reuse (~150f) ─────────────────────────────────

interface GNode { id: string; label: string; x: number; y: number; primary?: boolean }
interface GEdge { from: string; to: string }

const NW = 142; // pill width
const NH = 28;  // pill height
const ncx = (n: GNode) => n.x + NW / 2;
const ncy = (n: GNode) => n.y + NH / 2;

const GNODES: GNode[] = [
  { id: 'auth',     label: 'Auth tokens',     x: 238, y: 156, primary: true },
  { id: 'refresh',  label: 'refresh-tokens',  x: 20,  y: 60  },
  { id: 'jwt',      label: 'jwt-claims',       x: 432, y: 60  },
  { id: 'rotation', label: 'token-rotation',  x: 178, y: 14  },
  { id: 'oauth',    label: 'oauth2-flow',      x: 20,  y: 256 },
  { id: 'security', label: 'security-headers', x: 378, y: 256 },
];

const GEDGES: GEdge[] = [
  { from: 'auth',    to: 'refresh'  },
  { from: 'auth',    to: 'jwt'      },
  { from: 'auth',    to: 'rotation' },
  { from: 'auth',    to: 'oauth'    },
  { from: 'auth',    to: 'security' },
  { from: 'refresh', to: 'rotation' },
  { from: 'jwt',     to: 'security' },
];

const KnowledgeGraph: React.FC = () => {
  const frame = useCurrentFrame();
  const nodeMap: Record<string, GNode> = {};
  for (const n of GNODES) nodeMap[n.id] = n;

  return (
    <div style={{ position: 'relative', width: 600, height: 314, flexShrink: 0 }}>
      {/* SVG edges — fade in one by one */}
      <svg
        width={600}
        height={314}
        style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}
      >
        {GEDGES.map((e, i) => {
          const a = nodeMap[e.from];
          const b = nodeMap[e.to];
          const op = track(frame, [10 + i * 5, 22 + i * 5], [0, 1]);
          return (
            <line
              key={i}
              x1={ncx(a)} y1={ncy(a)}
              x2={ncx(b)} y2={ncy(b)}
              stroke={alpha(brand.violet, 0.42)}
              strokeWidth={1.5}
              opacity={op}
            />
          );
        })}
      </svg>

      {/* Node pills */}
      {GNODES.map((n, i) => {
        const op = track(frame, [i * 7, i * 7 + 14], [0, 1]);
        const ty = track(frame, [i * 7, i * 7 + 14], [10, 0]);
        return (
          <div
            key={n.id}
            style={{
              position: 'absolute',
              left: n.x,
              top: n.y,
              width: NW,
              height: NH,
              borderRadius: 14,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              background: n.primary
                ? alpha(brand.purple, 0.28)
                : alpha(T.surface2, 0.9),
              border: `1px solid ${n.primary ? alpha(brand.purple, 0.68) : alpha(T.border, 0.85)}`,
              boxShadow: n.primary ? `0 0 22px ${alpha(brand.purple, 0.38)}` : 'none',
              fontFamily: fonts.mono,
              fontSize: 11,
              fontWeight: n.primary ? 700 : 500,
              color: n.primary ? brand.mist : T.text,
              opacity: op,
              transform: `translateY(${ty}px)`,
            }}
          >
            {!n.primary && (
              <span style={{ opacity: 0.5, marginRight: 2, fontSize: 10 }}>[[</span>
            )}
            {n.label}
            {!n.primary && (
              <span style={{ opacity: 0.5, marginLeft: 2, fontSize: 10 }}>]]</span>
            )}
          </div>
        );
      })}
    </div>
  );
};

const GraphScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={<Navigator active="vault" />} title="Otto — Vault">
        <div
          style={{
            height: '100%',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 56,
            padding: '16px 44px',
          }}
        >
          {/* Knowledge graph */}
          <KnowledgeGraph />

          {/* Side callout */}
          <Appear delay={84} y={18} style={{ maxWidth: 340, flexShrink: 0 }}>
            <div
              style={{
                background: alpha(brand.purple, 0.1),
                border: `1px solid ${alpha(brand.purple, 0.3)}`,
                borderRadius: 12,
                padding: '20px 24px',
              }}
            >
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  marginBottom: 12,
                }}
              >
                <Icon name="zap" size={15} color={brand.violet} />
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 13,
                    fontWeight: 700,
                    color: brand.mist,
                    letterSpacing: 0.1,
                  }}
                >
                  Shared memory
                </span>
              </div>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 14.5,
                  lineHeight: 1.65,
                  color: alpha('#ffffff', 0.82),
                  marginBottom: 14,
                }}
              >
                Product, Swarm, and Review agents{' '}
                <span style={{ color: '#a78bfa', fontWeight: 600 }}>
                  recall from the Vault
                </span>
                {' '}instead of re-fetching context each turn.
              </div>
              <div
                style={{
                  fontFamily: fonts.mono,
                  fontSize: 12,
                  color: T.textDim,
                  background: T.termBg,
                  borderRadius: 6,
                  padding: '8px 10px',
                  border: `1px solid ${T.border}`,
                  lineHeight: 1.7,
                }}
              >
                {'vault.recall("auth token rotation")'}
                <br />
                <span style={{ color: alpha('#28c840', 0.9) }}>
                  {'→ 3 notes, score ≥ 0.80'}
                </span>
              </div>
            </div>
          </Appear>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="A live knowledge graph · other features recall from it"
      sub="Notes link automatically. Product, Swarm, and Review pull from the Vault — no context re-fetch."
    />
  </>
);

// ── Composition ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,  name: 'Title'     },
  { dur: 180, node: <NotesScene />,  name: 'Backlinks' },
  { dur: 150, node: <RecallScene />, name: 'Recall'    },
  { dur: 150, node: <GraphScene />,  name: 'Graph'     },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Vault"
        tagline="Your team's knowledge — linked, searchable, and shared with every agent"
        pills={[
          { label: 'Backlinked notes', icon: 'link'   },
          { label: 'Hybrid recall',    icon: 'search' },
          { label: 'Knowledge graph',  icon: 'globe'  },
          { label: 'Shared memory',    icon: 'box'    },
        ]}
      />
    ),
  },
];

export const vaultDuration = scenesDuration(SCENES);
export const Vault: React.FC = () => <Scenes scenes={SCENES} />;
