import React from 'react';
import { AbsoluteFill, useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, radius, navActive } from '../theme';
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
  Icon,
  Toggle,
  Segmented,
  track,
} from '../components/kit';

// ── Scene 1 — Title (~70f) ────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Skills & Self-Improvement"
    title="Skills Library"
    subtitle="A versioned skill library that powers review, product & insights — and quietly sharpens itself from how you work."
  />
);

// ── Scene 2 — Library (~165f) ─────────────────────────────────────────────────

const SETTINGS_SECTIONS = ['General', 'Appearance', 'Providers', 'Skills', 'Plugins', 'Integrations'];

interface SkillEntry {
  id: string;
  version: string;
  desc: string;
  installed: boolean;
  icon: string;
  color: string;
}

const SKILL_ENTRIES: SkillEntry[] = [
  {
    id: 'review/security',
    version: 'v2.1',
    desc: 'Vulnerability patterns, SSRF & injection detection lenses',
    installed: true,
    icon: 'eye',
    color: brand.violet,
  },
  {
    id: 'review/performance',
    version: 'v1.4',
    desc: 'Hotpath detection, N+1 queries & bundle size analysis',
    installed: false,
    icon: 'gauge',
    color: '#febc2e',
  },
  {
    id: 'product/analysis',
    version: 'v3.0',
    desc: 'Discovery chat, story decomposition & mockup generation',
    installed: true,
    icon: 'square',
    color: brand.purple,
  },
  {
    id: 'insights/catch-up',
    version: 'v1.2',
    desc: 'Daily digest, agent summaries & session anomaly alerts',
    installed: false,
    icon: 'chart',
    color: brand.cyan,
  },
];

const SkillRow: React.FC<{ entry: SkillEntry; delay: number }> = ({ entry, delay }) => (
  <Appear delay={delay} y={12}>
    <Card
      t={T}
      pad={14}
      style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 8 }}
    >
      {/* icon badge */}
      <div
        style={{
          width: 42,
          height: 42,
          borderRadius: radius.m,
          flexShrink: 0,
          background: alpha(entry.color, 0.14),
          border: `1px solid ${alpha(entry.color, 0.38)}`,
          display: 'grid',
          placeItems: 'center',
        }}
      >
        <Icon name={entry.icon} size={20} color={entry.color} />
      </div>

      {/* name + desc */}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
          <span
            style={{
              fontFamily: fonts.mono,
              fontSize: 14,
              fontWeight: 700,
              color: T.text,
            }}
          >
            {entry.id}
          </span>
          <Chip color={entry.color}>{entry.version}</Chip>
          {entry.installed && <Chip tone="ok">installed</Chip>}
        </div>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 13,
            color: T.textDim,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {entry.desc}
        </div>
      </div>

      {/* action */}
      <div style={{ flexShrink: 0 }}>
        {entry.installed ? (
          <Button variant="default" size="s" icon="check">
            Installed
          </Button>
        ) : (
          <Button variant="primary" size="s" icon="arrowDown">
            Install
          </Button>
        )}
      </div>
    </Card>
  </Appear>
);

const SettingsSidebar: React.FC<{ activeIdx: number }> = ({ activeIdx }) => (
  <div
    style={{
      width: 210,
      flexShrink: 0,
      borderRight: `1px solid ${T.border}`,
      background: T.bgSidebar,
      padding: '10px 0',
    }}
  >
    {SETTINGS_SECTIONS.map((s, i) => {
      const on = i === activeIdx;
      return (
        <div
          key={s}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 9,
            margin: '0 8px 2px',
            padding: '7px 12px',
            borderRadius: 6,
            background: on ? navActive.bg : 'transparent',
            color: on ? navActive.fg : T.textDim,
            fontFamily: fonts.ui,
            fontSize: 13.5,
            fontWeight: on ? 600 : 500,
          }}
        >
          {s}
        </div>
      );
    })}
  </div>
);

const LibraryScene: React.FC = () => (
  <>
    <Stage scale={0.88} enter="up">
      <OttoWindow
        nav={<Navigator active="settings" />}
        title="Otto — Settings · Skills"
        width={1560}
        height={884}
      >
        <div style={{ display: 'flex', height: '100%' }}>
          <SettingsSidebar activeIdx={3} />

          {/* Catalog content */}
          <div
            style={{
              flex: 1,
              padding: '24px 28px',
              overflowY: 'hidden' as const,
            }}
          >
            {/* Section header */}
            <Appear delay={14} y={14}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  marginBottom: 20,
                }}
              >
                <div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 20,
                      fontWeight: 700,
                      color: T.text,
                    }}
                  >
                    Skills Library
                  </div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 13,
                      color: T.textDim,
                      marginTop: 3,
                    }}
                  >
                    48 skills available · 2 installed
                  </div>
                </div>
                <Segmented options={['Installed', 'All 48']} active={1} />
              </div>
            </Appear>

            {/* Skill rows */}
            {SKILL_ENTRIES.map((entry, i) => (
              <SkillRow key={entry.id} entry={entry} delay={26 + i * 15} />
            ))}
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="A bundled, versioned skill library — browse & install"
      sub="Skills ship with Otto and update independently. Install to activate review lenses, product templates & insight prompts."
    />
  </>
);

// ── Scene 3 — Skills drive features (~120f) ───────────────────────────────────

const DRIVE_TARGETS = [
  { label: 'Review lenses',    icon: 'eye',    color: brand.violet, delay: 34 },
  { label: 'Product analysis', icon: 'square', color: brand.purple, delay: 50 },
  { label: 'Insights',         icon: 'chart',  color: brand.cyan,   delay: 66 },
] as const;

// Fan-out SVG lines from skill card center to each feature target
const SVG_LINES = [
  { x1: 0, y1: 120, x2: 200, y2: 42  },
  { x1: 0, y1: 120, x2: 200, y2: 120 },
  { x1: 0, y1: 120, x2: 200, y2: 196 },
] as const;

const LINE_COLORS = [brand.violet, brand.purple, brand.cyan] as const;

const DriveScene: React.FC = () => {
  const frame = useCurrentFrame();

  const p0 = track(frame, [20, 52], [0, 1]);
  const p1 = track(frame, [32, 64], [0, 1]);
  const p2 = track(frame, [44, 76], [0, 1]);
  const lineProgress = [p0, p1, p2];

  return (
    <>
      <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
        <div style={{ display: 'flex', alignItems: 'center' }}>
          {/* Selected skill card */}
          <Appear delay={8} y={20}>
            <Card
              t={T}
              pad={24}
              style={{
                width: 248,
                textAlign: 'center',
                background: alpha(brand.purple, 0.12),
                border: `1.5px solid ${alpha(brand.purple, 0.46)}`,
                boxShadow: `0 0 44px ${alpha(brand.purple, 0.22)}`,
              }}
            >
              <div
                style={{
                  display: 'flex',
                  justifyContent: 'center',
                  marginBottom: 14,
                }}
              >
                <div
                  style={{
                    width: 56,
                    height: 56,
                    borderRadius: radius.l,
                    background: alpha(brand.purple, 0.18),
                    border: `1px solid ${alpha(brand.purple, 0.42)}`,
                    display: 'grid',
                    placeItems: 'center',
                  }}
                >
                  <Icon name="zap" size={26} color={brand.purple} />
                </div>
              </div>
              <div
                style={{
                  fontFamily: fonts.mono,
                  fontSize: 14.5,
                  fontWeight: 700,
                  color: T.text,
                  marginBottom: 10,
                }}
              >
                review/security
              </div>
              <div style={{ display: 'flex', justifyContent: 'center', gap: 6 }}>
                <Chip color={brand.purple}>v2.1</Chip>
                <Chip tone="ok">installed</Chip>
              </div>
            </Card>
          </Appear>

          {/* Animated SVG connector lines */}
          <svg
            width={200}
            height={240}
            viewBox="0 0 200 240"
            style={{ overflow: 'visible', flexShrink: 0 }}
          >
            {SVG_LINES.map((l, i) => {
              const dx = l.x2 - l.x1;
              const dy = l.y2 - l.y1;
              const len = Math.sqrt(dx * dx + dy * dy);
              const drawn = len * lineProgress[i];
              return (
                <line
                  key={i}
                  x1={l.x1}
                  y1={l.y1}
                  x2={l.x2}
                  y2={l.y2}
                  stroke={alpha(LINE_COLORS[i], 0.55)}
                  strokeWidth={1.5}
                  strokeDasharray={len}
                  strokeDashoffset={len - drawn}
                />
              );
            })}
          </svg>

          {/* Feature target chips */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 28 }}>
            {DRIVE_TARGETS.map((tgt) => (
              <Appear key={tgt.label} delay={tgt.delay} x={-16} y={0}>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 12,
                    padding: '14px 20px',
                    borderRadius: radius.m,
                    background: alpha(tgt.color, 0.11),
                    border: `1px solid ${alpha(tgt.color, 0.38)}`,
                    boxShadow: `0 4px 22px ${alpha(tgt.color, 0.14)}`,
                    fontFamily: fonts.ui,
                    fontSize: 16,
                    fontWeight: 600,
                    color: '#ffffff',
                    minWidth: 220,
                  }}
                >
                  <Icon name={tgt.icon} size={20} color={tgt.color} />
                  {tgt.label}
                </div>
              </Appear>
            ))}
          </div>
        </div>
      </AbsoluteFill>
      <Caption
        step={3}
        title="Installed skills power review lenses, product analysis & insights"
        sub="One install wires the skill into the review engine, the product canvas, and your daily insights feed."
      />
    </>
  );
};

// ── Scene 4 — Self-improvement (~150f) ────────────────────────────────────────

interface Proposal {
  target: string;
  change: string;
  tier: 'safe' | 'risky';
}

const PROPOSALS: Proposal[] = [
  {
    target: 'review/security SKILL.md',
    change: 'Added SSRF detection pattern for Go net/http clients',
    tier: 'safe',
  },
  {
    target: 'memory/go-patterns.md',
    change: 'New: nil pointer guard patterns in multi-tenant service layer',
    tier: 'safe',
  },
  {
    target: 'product/analysis v3.0',
    change: 'Expand discovery prompt — 2 extra clarification rounds before decomposing',
    tier: 'risky',
  },
  {
    target: 'insights/catch-up',
    change: 'Merge weekly digest template with daily summary format',
    tier: 'risky',
  },
];

const ProposalRow: React.FC<{ p: Proposal; delay: number }> = ({ p, delay }) => (
  <Appear delay={delay} y={10}>
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '12px 14px',
        borderRadius: radius.m,
        background: T.surface,
        border: `1px solid ${T.border}`,
        marginBottom: 8,
      }}
    >
      {/* tier icon */}
      <div
        style={{
          width: 34,
          height: 34,
          borderRadius: radius.s,
          flexShrink: 0,
          background:
            p.tier === 'safe' ? alpha('#28c840', 0.12) : alpha('#febc2e', 0.12),
          border: `1px solid ${
            p.tier === 'safe' ? alpha('#28c840', 0.3) : alpha('#febc2e', 0.3)
          }`,
          display: 'grid',
          placeItems: 'center',
        }}
      >
        <Icon
          name={p.tier === 'safe' ? 'check' : 'edit'}
          size={15}
          color={p.tier === 'safe' ? '#28c840' : '#febc2e'}
        />
      </div>

      {/* target + description */}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontFamily: fonts.mono,
            fontSize: 12.5,
            color: brand.cyan,
            marginBottom: 3,
          }}
        >
          {p.target}
        </div>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 13.5,
            color: T.text,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {p.change}
        </div>
      </div>

      {/* tier chip */}
      <Chip tone={p.tier === 'safe' ? 'ok' : 'warn'}>
        {p.tier === 'safe' ? 'safe · auto-applied' : 'needs approval'}
      </Chip>

      {/* approve / reject for risky only */}
      {p.tier === 'risky' && (
        <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
          <Button variant="primary" size="s" icon="check">
            Approve
          </Button>
          <Button variant="danger" size="s" icon="x">
            Reject
          </Button>
        </div>
      )}
    </div>
  </Appear>
);

const ImproveScene: React.FC = () => (
  <>
    <Stage scale={0.88} enter="up">
      <OttoWindow
        nav={<Navigator active="settings" />}
        title="Otto — Settings · Self-Improvement"
        width={1560}
        height={884}
      >
        <div style={{ display: 'flex', height: '100%' }}>
          <SettingsSidebar activeIdx={3} />

          {/* Engine panel */}
          <div
            style={{
              flex: 1,
              padding: '24px 28px',
              overflowY: 'hidden' as const,
            }}
          >
            {/* Engine header */}
            <Appear delay={10} y={14}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'flex-start',
                  justifyContent: 'space-between',
                  marginBottom: 18,
                }}
              >
                <div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 20,
                      fontWeight: 700,
                      color: T.text,
                    }}
                  >
                    Self-Improvement Engine
                  </div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 13,
                      color: T.textDim,
                      marginTop: 3,
                      maxWidth: 640,
                    }}
                  >
                    Reflects on recent sessions · proposes edits to skills &amp; memory only ·
                    never touches your repo code
                  </div>
                </div>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    marginTop: 2,
                  }}
                >
                  <span
                    style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}
                  >
                    Enabled
                  </span>
                  <Toggle on={true} />
                </div>
              </div>
            </Appear>

            {/* Stats row */}
            <Appear delay={18} y={10}>
              <div style={{ display: 'flex', gap: 10, marginBottom: 22 }}>
                {[
                  { label: 'Sessions analysed', value: '47'        },
                  { label: 'Edits auto-applied', value: '12'        },
                  { label: 'Queued for review',  value: '2'         },
                  { label: 'Last run',            value: '4 min ago' },
                ].map((stat) => (
                  <div
                    key={stat.label}
                    style={{
                      flex: 1,
                      padding: '10px 14px',
                      borderRadius: radius.m,
                      background: T.surface,
                      border: `1px solid ${T.border}`,
                    }}
                  >
                    <div
                      style={{
                        fontFamily: fonts.ui,
                        fontSize: 11.5,
                        color: T.textDim,
                        marginBottom: 5,
                      }}
                    >
                      {stat.label}
                    </div>
                    <div
                      style={{
                        fontFamily: fonts.ui,
                        fontSize: 22,
                        fontWeight: 700,
                        color: T.text,
                      }}
                    >
                      {stat.value}
                    </div>
                  </div>
                ))}
              </div>
            </Appear>

            {/* Proposals section heading */}
            <Appear delay={24} y={8}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11.5,
                  fontWeight: 600,
                  color: T.textDim,
                  letterSpacing: 0.5,
                  textTransform: 'uppercase' as const,
                  marginBottom: 12,
                }}
              >
                Proposed edits · 4
              </div>
            </Appear>

            {/* Proposal rows */}
            {PROPOSALS.map((p, i) => (
              <ProposalRow key={p.target} p={p} delay={30 + i * 18} />
            ))}
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={4}
      title="Self-improvement proposes edits to skills & memory — safe auto-applied, risky queued"
      sub="The engine reflects across providers. Tiered autonomy: safe changes land immediately; risky ones wait for you."
    />
  </>
);

// ── Composition ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 70,  node: <TitleScene />,   name: 'Title'   },
  { dur: 165, node: <LibraryScene />, name: 'Library' },
  { dur: 120, node: <DriveScene />,   name: 'Drive'   },
  { dur: 150, node: <ImproveScene />, name: 'Improve' },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Skills & Self-Improvement"
        tagline="Otto sharpens its own skills from how you actually work"
        pills={[
          { label: 'Versioned library',              icon: 'zap'     },
          { label: 'Drives review/product/insights', icon: 'eye'     },
          { label: 'Auto-improves',                  icon: 'refresh' },
          { label: 'Approval-gated',                 icon: 'check'   },
        ]}
      />
    ),
  },
];

export const skillsDuration = scenesDuration(SCENES);
export const Skills: React.FC = () => <Scenes scenes={SCENES} />;
