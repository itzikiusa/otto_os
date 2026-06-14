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
import { Navigator } from '../components/Navigator';
import { Appear, Caption, Cursor, TitleCard } from '../components/ui';

// ─── Helpers ─────────────────────────────────────────────────────────────────

const FadeIn: React.FC<{ children: React.ReactNode; durationFrames?: number }> = ({
  children,
  durationFrames = 18,
}) => {
  const frame = useCurrentFrame();
  const opacity = interpolate(frame, [0, durationFrames], [0, 1], {
    extrapolateRight: 'clamp',
    extrapolateLeft: 'clamp',
  });
  return <div style={{ opacity, width: '100%', height: '100%' }}>{children}</div>;
};

// ─── Shared sub-components ────────────────────────────────────────────────────

/** A toolbar button pill */
const ToolBtn: React.FC<{ label: string; accent?: boolean; delay?: number }> = ({
  label,
  accent,
  delay = 0,
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 200 } });
  const op = interpolate(s, [0, 1], [0, 1]);
  const y = interpolate(s, [0, 1], [10, 0]);
  return (
    <div
      style={{
        opacity: op,
        transform: `translateY(${y}px)`,
        padding: '5px 14px',
        borderRadius: 7,
        background: accent ? theme.accent : theme.surface2,
        border: `1px solid ${accent ? theme.accent : theme.border}`,
        color: accent ? '#fff' : theme.text,
        fontFamily: theme.font,
        fontSize: 13,
        fontWeight: 600,
        whiteSpace: 'nowrap' as const,
        boxShadow: accent ? `0 4px 18px ${theme.accent}44` : 'none',
      }}
    >
      {label}
    </div>
  );
};

/** Branch-tree leaf row */
const BranchRow: React.FC<{
  name: string;
  current?: boolean;
  remote?: boolean;
  delay?: number;
}> = ({ name, current, remote, delay = 0 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 200 } });
  return (
    <div
      style={{
        opacity: interpolate(s, [0, 1], [0, 1]),
        transform: `translateX(${interpolate(s, [0, 1], [-8, 0])}px)`,
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        padding: '3px 8px 3px 20px',
        borderRadius: 5,
        background: current ? `${theme.accent}22` : 'transparent',
        fontFamily: theme.mono,
        fontSize: 12,
        color: current ? theme.text : theme.textDim,
      }}
    >
      <span
        style={{
          width: 7,
          height: 7,
          borderRadius: '50%',
          background: remote ? theme.warn : current ? theme.accent2 : theme.textDim,
          flexShrink: 0,
        }}
      />
      {name}
    </div>
  );
};

/** Left branch-tree panel */
const BranchTree: React.FC = () => (
  <div
    style={{
      width: 200,
      height: '100%',
      background: theme.surface,
      borderRight: `1px solid ${theme.border}`,
      flexShrink: 0,
      paddingTop: 10,
      overflowY: 'hidden' as const,
    }}
  >
    <div
      style={{
        color: theme.textDim,
        fontFamily: theme.font,
        fontSize: 10,
        fontWeight: 700,
        letterSpacing: 1.2,
        textTransform: 'uppercase' as const,
        padding: '6px 12px 4px',
      }}
    >
      LOCAL
    </div>
    <BranchRow name="develop" current delay={0} />
    <BranchRow name="master" delay={4} />
    <BranchRow name="feat/setup" delay={8} />
    <div
      style={{
        color: theme.textDim,
        fontFamily: theme.font,
        fontSize: 10,
        fontWeight: 700,
        letterSpacing: 1.2,
        textTransform: 'uppercase' as const,
        padding: '10px 12px 4px',
      }}
    >
      REMOTE
    </div>
    <BranchRow name="origin/develop" remote delay={12} />
    <BranchRow name="origin/master" remote delay={16} />
    <BranchRow name="origin/feat/setup" remote delay={20} />
    <div
      style={{
        color: theme.textDim,
        fontFamily: theme.font,
        fontSize: 10,
        fontWeight: 700,
        letterSpacing: 1.2,
        textTransform: 'uppercase' as const,
        padding: '10px 12px 4px',
      }}
    >
      TAGS
    </div>
    <BranchRow name="v1.4.0" delay={24} />
    <BranchRow name="v1.3.2" delay={28} />
  </div>
);

// Commit data for the graph
const COMMITS = [
  { sha: 'a1b2c3d', msg: 'feat(setup): automate env provisioning', author: 'dev', lane: 0, color: theme.accent },
  { sha: 'e4f5a6b', msg: 'fix(auth): add missing JWT validation', author: 'sara',  lane: 1, color: theme.accent2 },
  { sha: 'c7d8e9f', msg: 'refactor: split service layer', author: 'dev', lane: 0, color: theme.accent },
  { sha: 'b0a1b2c', msg: 'chore: update deps, bump go 1.22', author: 'bot',   lane: 2, color: '#bf7aff' },
  { sha: 'f3e4d5c', msg: 'docs: add README for gateway module', author: 'sara',  lane: 1, color: theme.accent2 },
  { sha: '69a0b1c', msg: 'feat(wallet): multi-currency support', author: 'dev', lane: 0, color: theme.accent },
];

/** SVG commit graph lanes */
const CommitGraph: React.FC<{ selectedIdx?: number; frame: number }> = ({ selectedIdx, frame }) => {
  const rowH = 48;
  const laneW = 18;
  const lanes = [theme.accent, theme.accent2, '#bf7aff'];

  return (
    <div style={{ display: 'flex', flex: 1, overflow: 'hidden' as const }}>
      {/* SVG lane track */}
      <svg width={80} height={COMMITS.length * rowH} style={{ flexShrink: 0, marginTop: 0 }}>
        {/* Lane lines */}
        {lanes.map((color, li) => {
          const x = 18 + li * laneW;
          const lineLen = interpolate(frame, [li * 5, li * 5 + 40], [0, COMMITS.length * rowH], {
            extrapolateRight: 'clamp',
            extrapolateLeft: 'clamp',
          });
          return (
            <line
              key={li}
              x1={x} y1={0}
              x2={x} y2={lineLen}
              stroke={color}
              strokeWidth={1.5}
              strokeOpacity={0.35}
            />
          );
        })}
        {/* Dots */}
        {COMMITS.map((c, i) => {
          const cx = 18 + c.lane * laneW;
          const cy = i * rowH + rowH / 2;
          const dotFrame = frame - i * 6;
          const r = interpolate(dotFrame, [0, 14], [0, 6], { extrapolateRight: 'clamp', extrapolateLeft: 'clamp' });
          const selected = selectedIdx === i;
          return (
            <g key={i}>
              {selected && (
                <circle cx={cx} cy={cy} r={11} fill={c.color} fillOpacity={0.18} />
              )}
              <circle cx={cx} cy={cy} r={r} fill={c.color}
                stroke={selected ? '#fff' : c.color}
                strokeWidth={selected ? 2 : 0}
              />
            </g>
          );
        })}
      </svg>

      {/* Commit rows */}
      <div style={{ flex: 1, overflowY: 'hidden' as const }}>
        {COMMITS.map((c, i) => {
          const rowOpacity = interpolate(frame, [i * 6, i * 6 + 14], [0, 1], {
            extrapolateRight: 'clamp',
            extrapolateLeft: 'clamp',
          });
          const selected = selectedIdx === i;
          return (
            <div
              key={i}
              style={{
                height: rowH,
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '0 12px',
                opacity: rowOpacity,
                background: selected ? `${theme.accent}1a` : 'transparent',
                borderLeft: selected ? `2px solid ${theme.accent}` : '2px solid transparent',
              }}
            >
              <span
                style={{
                  fontFamily: theme.mono,
                  fontSize: 11,
                  color: c.color,
                  background: `${c.color}22`,
                  padding: '2px 7px',
                  borderRadius: 4,
                  flexShrink: 0,
                }}
              >
                {c.sha.slice(0, 7)}
              </span>
              <span
                style={{
                  fontFamily: theme.font,
                  fontSize: 13,
                  color: selected ? theme.text : theme.textDim,
                  flex: 1,
                  overflow: 'hidden' as const,
                  textOverflow: 'ellipsis' as const,
                  whiteSpace: 'nowrap' as const,
                }}
              >
                {c.msg}
              </span>
              <span
                style={{
                  fontFamily: theme.font,
                  fontSize: 11,
                  color: theme.textDim,
                  flexShrink: 0,
                }}
              >
                {c.author}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
};

// ─── Scene 1: Title (~0–75f) ──────────────────────────────────────────────────

const SceneTitle: React.FC = () => (
  <AbsoluteFill>
    <TitleCard
      kicker="OTTO ADE"
      title="Git & Pull Requests"
      subtitle="Commit graph · AI review agents · Ship with confidence"
    />
  </AbsoluteFill>
);

// ─── Scene 2: Repo view — branch tree + commit graph + toolbar ────────────────
// frames 60–240 (6s)

const SceneRepoView: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const winS = spring({ frame, fps, config: { damping: 22, stiffness: 100 } });
  const winY = interpolate(winS, [0, 1], [80, 0]);
  const winOp = interpolate(frame, [0, 18], [0, 1], { extrapolateRight: 'clamp' });

  // Toolbar draws in after 40f
  const toolbarOp = interpolate(frame, [40, 55], [0, 1], { extrapolateRight: 'clamp', extrapolateLeft: 'clamp' });

  const TOOLBAR = ['Fetch', 'Pull', 'Push', 'Branch', 'Stash', 'Pop'];

  return (
    <AbsoluteFill
      style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}
    >
      <div
        style={{
          opacity: winOp,
          transform: `translateY(${winY}px)`,
        }}
      >
        <OttoWindow
          sidebar={<Navigator active="git" />}
          title="Otto — sinatra-users-go"
          style={{ width: 1440, height: 820 }}
        >
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
            {/* Toolbar */}
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                padding: '8px 14px',
                borderBottom: `1px solid ${theme.border}`,
                background: theme.surface,
                opacity: toolbarOp,
                flexShrink: 0,
              }}
            >
              <span style={{ fontFamily: theme.mono, fontSize: 12, color: theme.accent2, marginRight: 6 }}>
                ⎇ develop
              </span>
              <div style={{ width: 1, height: 20, background: theme.border, margin: '0 4px' }} />
              {TOOLBAR.map((btn, i) => (
                <ToolBtn key={btn} label={btn} accent={btn === 'Push'} delay={i * 4} />
              ))}
            </div>

            {/* Graph area */}
            <div style={{ flex: 1, display: 'flex', overflow: 'hidden' as const }}>
              <BranchTree />
              <div style={{ flex: 1, padding: '12px 0', overflow: 'hidden' as const }}>
                {/* Header row */}
                <div
                  style={{
                    display: 'flex',
                    padding: '0 12px 8px 92px',
                    borderBottom: `1px solid ${theme.border}`,
                    gap: 10,
                  }}
                >
                  {['SHA', 'Message', 'Author'].map((h) => (
                    <span
                      key={h}
                      style={{
                        fontFamily: theme.font,
                        fontSize: 11,
                        fontWeight: 700,
                        color: theme.textDim,
                        textTransform: 'uppercase' as const,
                        letterSpacing: 1,
                        flex: h === 'Message' ? 1 : 0,
                        minWidth: h === 'SHA' ? 80 : undefined,
                      }}
                    >
                      {h}
                    </span>
                  ))}
                </div>
                <CommitGraph frame={frame - 10} />
              </div>
            </div>
          </div>
        </OttoWindow>
      </div>

      <Caption step={1} title="Commit graph" sub="Branch tree · LOCAL / REMOTE / TAGS · Fetch, Pull, Push" delay={50} />
    </AbsoluteFill>
  );
};

// ─── Scene 3: Click a commit → diff panel ─────────────────────────────────────
// frames 235–360 (4s)

const DIFF_LINES = [
  { t: 'hunk', text: '@@ -12,7 +12,14 @@ func ProvisionEnv(ctx context.Context, cfg Config) error {' },
  { t: 'ctx', text: '   if cfg.BrandID == 0 {' },
  { t: 'ctx', text: '     return ErrMissingBrand' },
  { t: 'ctx', text: '   }' },
  { t: 'add', text: '+  svc, err := discovery.GetService(ctx, cfg.BrandID, "ENV_MANAGER")' },
  { t: 'add', text: '+  if err != nil { return err }' },
  { t: 'add', text: '+  return svc.Provision(ctx, cfg)' },
  { t: 'del', text: '-  return legacyProvision(cfg)' },
  { t: 'ctx', text: '}' },
];

const SceneCommitDiff: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const panelS = spring({ frame: frame - 10, fps, config: { damping: 200 } });
  const panelW = interpolate(panelS, [0, 1], [0, 560]);
  const panelOp = interpolate(panelS, [0, 1], [0, 1]);

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <OttoWindow
        sidebar={<Navigator active="git" />}
        title="Otto — sinatra-users-go"
        style={{ width: 1440, height: 820 }}
      >
        <div style={{ display: 'flex', height: '100%' }}>
          {/* left: branch tree + commit list (selected row) */}
          <div style={{ width: 680, display: 'flex', flexDirection: 'column', borderRight: `1px solid ${theme.border}` }}>
            <div style={{ display: 'flex', height: '100%' }}>
              <BranchTree />
              <div style={{ flex: 1, padding: '12px 0', overflow: 'hidden' as const }}>
                {/* header */}
                <div style={{ display: 'flex', padding: '0 12px 8px 92px', borderBottom: `1px solid ${theme.border}`, gap: 10 }}>
                  {['SHA', 'Message', 'Author'].map((h) => (
                    <span key={h} style={{ fontFamily: theme.font, fontSize: 11, fontWeight: 700, color: theme.textDim, textTransform: 'uppercase' as const, letterSpacing: 1, flex: h === 'Message' ? 1 : 0, minWidth: h === 'SHA' ? 80 : undefined }}>{h}</span>
                  ))}
                </div>
                <CommitGraph frame={80} selectedIdx={0} />
              </div>
            </div>
          </div>

          {/* right: diff panel sliding in */}
          <div
            style={{
              width: panelW,
              opacity: panelOp,
              overflow: 'hidden' as const,
              display: 'flex',
              flexDirection: 'column',
            }}
          >
            {/* commit meta */}
            <div style={{ padding: '14px 18px', borderBottom: `1px solid ${theme.border}`, flexShrink: 0 }}>
              <div style={{ fontFamily: theme.mono, fontSize: 12, color: theme.accent, marginBottom: 4 }}>a1b2c3d</div>
              <div style={{ fontFamily: theme.font, fontSize: 14, fontWeight: 700, color: theme.text, marginBottom: 6 }}>
                feat(setup): automate env provisioning
              </div>
              <div style={{ display: 'flex', gap: 10 }}>
                <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.textDim }}>dev · just now</span>
                <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.accent2, background: `${theme.accent2}18`, padding: '1px 7px', borderRadius: 4 }}>+7 −1</span>
              </div>
            </div>

            {/* diff */}
            <div style={{ flex: 1, overflow: 'hidden' as const, padding: '10px 0', fontFamily: theme.mono, fontSize: 12 }}>
              <div style={{ padding: '0 16px 6px', fontFamily: theme.font, fontSize: 12, color: theme.textDim }}>
                env/provisioner.go
              </div>
              {DIFF_LINES.map((line, i) => {
                const lineOp = interpolate(frame, [20 + i * 5, 30 + i * 5], [0, 1], { extrapolateRight: 'clamp', extrapolateLeft: 'clamp' });
                const bg = line.t === 'add' ? `${theme.accent2}18` : line.t === 'del' ? `${theme.danger}18` : 'transparent';
                const color = line.t === 'add' ? theme.accent2 : line.t === 'del' ? theme.danger : line.t === 'hunk' ? theme.accent : theme.textDim;
                return (
                  <div key={i} style={{ opacity: lineOp, background: bg, padding: '1px 16px', color, lineHeight: 1.6, fontSize: 12 }}>
                    {line.text}
                  </div>
                );
              })}
            </div>
          </div>
        </div>

        {/* cursor click animation */}
        <Cursor from={[480, 260]} to={[480, 208]} startAt={2} duration={18} click />
      </OttoWindow>

      <Caption step={2} title="Click any commit → instant diff" sub="sha · message · +/− file diff" delay={40} />
    </AbsoluteFill>
  );
};

// ─── Scene 4: Pull Requests list → open PR → tabs ────────────────────────────
// frames 355–510 (5.2s)

const PR_TABS = ['Summary', 'Files', 'Commits', 'Review'];

const ScenePullRequests: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const prPanelS = spring({ frame: frame - 20, fps, config: { damping: 200 } });
  const prPanelOp = interpolate(prPanelS, [0, 1], [0, 1]);
  const prPanelY = interpolate(prPanelS, [0, 1], [20, 0]);

  // After frame 60, show open PR view
  const openPrS = spring({ frame: frame - 60, fps, config: { damping: 200 } });
  const openPrOp = interpolate(openPrS, [0, 1], [0, 1]);

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <OttoWindow
        sidebar={<Navigator active="git" />}
        title="Otto — sinatra-users-go · Pull Requests"
        style={{ width: 1440, height: 820 }}
      >
        <div style={{ display: 'flex', height: '100%', flexDirection: 'column' }}>
          {/* PR list header */}
          <div style={{ padding: '12px 20px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 12, flexShrink: 0, background: theme.surface }}>
            <span style={{ fontFamily: theme.font, fontSize: 15, fontWeight: 700, color: theme.text }}>Pull Requests</span>
            <span style={{ fontFamily: theme.font, fontSize: 12, color: theme.textDim, background: `${theme.accent}22`, padding: '2px 9px', borderRadius: 10 }}>3 open</span>
          </div>

          {/* PR rows */}
          <div style={{ opacity: prPanelOp, transform: `translateY(${prPanelY}px)`, flex: openPrOp < 0.1 ? 1 : undefined, overflow: 'hidden' as const }}>
            {[
              { num: '#22', title: 'feat(setup-automation): provision env on brand init', status: 'OPEN', color: theme.accent2, author: 'dev', comments: 4 },
              { num: '#21', title: 'fix(auth): validate JWT expiry in middleware', status: 'MERGED', color: theme.textDim, author: 'sara', comments: 2 },
              { num: '#20', title: 'chore: upgrade go 1.22, update CI matrix', status: 'OPEN', color: theme.accent2, author: 'bot', comments: 0 },
            ].map((pr, i) => {
              const rowS = spring({ frame: frame - 10 - i * 10, fps, config: { damping: 200 } });
              const rowOp = interpolate(rowS, [0, 1], [0, 1]);
              const isSelected = pr.num === '#22';
              return (
                <div
                  key={pr.num}
                  style={{
                    opacity: rowOp,
                    display: 'flex',
                    alignItems: 'center',
                    gap: 12,
                    padding: '14px 20px',
                    borderBottom: `1px solid ${theme.border}`,
                    background: isSelected && frame > 55 ? `${theme.accent}14` : 'transparent',
                    borderLeft: isSelected && frame > 55 ? `3px solid ${theme.accent}` : '3px solid transparent',
                  }}
                >
                  <span style={{ fontFamily: theme.mono, fontSize: 12, color: theme.textDim, flexShrink: 0 }}>{pr.num}</span>
                  <span style={{ fontFamily: theme.font, fontSize: 14, color: theme.text, flex: 1 }}>{pr.title}</span>
                  <span style={{ fontFamily: theme.font, fontSize: 11, fontWeight: 700, color: pr.color, background: `${pr.color}22`, padding: '2px 9px', borderRadius: 8 }}>{pr.status}</span>
                  <span style={{ fontFamily: theme.font, fontSize: 12, color: theme.textDim }}>{pr.author}</span>
                  {pr.comments > 0 && <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.textDim }}>💬 {pr.comments}</span>}
                </div>
              );
            })}
          </div>

          {/* Open PR detail view */}
          {frame > 55 && (
            <div style={{ opacity: openPrOp, flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' as const }}>
              {/* PR title bar */}
              <div style={{ padding: '14px 20px', borderBottom: `1px solid ${theme.border}`, background: theme.surface, flexShrink: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 6 }}>
                  <span style={{ fontFamily: theme.mono, fontSize: 12, color: theme.textDim }}>#22</span>
                  <span style={{ fontFamily: theme.font, fontSize: 16, fontWeight: 700, color: theme.text }}>feat(setup-automation): provision env on brand init</span>
                  <span style={{ fontFamily: theme.font, fontSize: 11, fontWeight: 700, color: theme.accent2, background: `${theme.accent2}22`, padding: '3px 10px', borderRadius: 8 }}>OPEN</span>
                </div>
                <div style={{ display: 'flex', gap: 6 }}>
                  {PR_TABS.map((tab, i) => {
                    const tabS = spring({ frame: frame - 65 - i * 8, fps, config: { damping: 200 } });
                    const tabOp = interpolate(tabS, [0, 1], [0, 1]);
                    const isActive = tab === 'Summary';
                    return (
                      <div
                        key={tab}
                        style={{
                          opacity: tabOp,
                          padding: '5px 14px',
                          borderRadius: 7,
                          background: isActive ? `${theme.accent}22` : 'transparent',
                          border: `1px solid ${isActive ? theme.accent : 'transparent'}`,
                          fontFamily: theme.font,
                          fontSize: 13,
                          fontWeight: isActive ? 700 : 500,
                          color: isActive ? theme.accent : theme.textDim,
                          cursor: 'default',
                        }}
                      >
                        {tab}
                      </div>
                    );
                  })}
                </div>
              </div>

              {/* PR summary body */}
              <div style={{ flex: 1, padding: '18px 24px', overflow: 'hidden' as const }}>
                <Appear delay={80} y={12}>
                  <div style={{ fontFamily: theme.font, fontSize: 14, color: theme.textDim, lineHeight: 1.7 }}>
                    <p style={{ margin: '0 0 10px', color: theme.text, fontWeight: 600 }}>What changed</p>
                    <p style={{ margin: '0 0 8px' }}>Automates full environment provisioning on new brand creation. Replaces legacy <code style={{ fontFamily: theme.mono, color: theme.accent2, background: `${theme.accent2}18`, padding: '1px 5px', borderRadius: 3 }}>legacyProvision</code> with service-discovery–aware call.</p>
                    <div style={{ display: 'flex', gap: 14, marginTop: 14 }}>
                      <div style={{ background: theme.surface2, border: `1px solid ${theme.border}`, borderRadius: 8, padding: '8px 14px', fontFamily: theme.mono, fontSize: 12 }}>
                        <div style={{ color: theme.accent2 }}>+37</div>
                        <div style={{ color: theme.textDim, fontSize: 10 }}>additions</div>
                      </div>
                      <div style={{ background: theme.surface2, border: `1px solid ${theme.border}`, borderRadius: 8, padding: '8px 14px', fontFamily: theme.mono, fontSize: 12 }}>
                        <div style={{ color: theme.danger }}>−12</div>
                        <div style={{ color: theme.textDim, fontSize: 10 }}>deletions</div>
                      </div>
                      <div style={{ background: theme.surface2, border: `1px solid ${theme.border}`, borderRadius: 8, padding: '8px 14px', fontFamily: theme.font, fontSize: 12 }}>
                        <div style={{ color: theme.text }}>4 files</div>
                        <div style={{ color: theme.textDim, fontSize: 10 }}>changed</div>
                      </div>
                    </div>
                  </div>
                </Appear>
              </div>
            </div>
          )}
        </div>

        <Cursor from={[700, 500]} to={[700, 390]} startAt={50} duration={20} click />
      </OttoWindow>

      <Caption step={3} title="Pull Requests" sub="#22 feat(setup-automation) — Summary / Files / Commits / Review" delay={20} />
    </AbsoluteFill>
  );
};

// ─── Scene 5: Files tab — file tree + diff + inline comment ──────────────────
// frames 505–660 (5.2s)

const SceneFilesTab: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const commentS = spring({ frame: frame - 70, fps, config: { damping: 180 } });
  const commentOp = interpolate(commentS, [0, 1], [0, 1]);
  const commentY = interpolate(commentS, [0, 1], [14, 0]);

  const FILE_TREE = [
    { name: 'env/provisioner.go', badge: '+7' },
    { name: 'env/provisioner_test.go', badge: '+18' },
    { name: 'discovery/locator.go', badge: '+12' },
    { name: 'go.mod', badge: '+1' },
  ];

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <OttoWindow
        sidebar={<Navigator active="git" />}
        title="Otto — #22 · Files"
        style={{ width: 1440, height: 820 }}
      >
        <div style={{ display: 'flex', height: '100%' }}>
          {/* File tree */}
          <div style={{ width: 260, borderRight: `1px solid ${theme.border}`, background: theme.surface, flexShrink: 0, paddingTop: 10 }}>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase' as const, padding: '6px 12px 8px' }}>Changed Files</div>
            {FILE_TREE.map((f, i) => {
              const s = spring({ frame: frame - i * 8, fps, config: { damping: 200 } });
              return (
                <div
                  key={f.name}
                  style={{
                    opacity: interpolate(s, [0, 1], [0, 1]),
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '6px 12px',
                    background: i === 0 ? `${theme.accent}18` : 'transparent',
                    borderLeft: i === 0 ? `2px solid ${theme.accent}` : '2px solid transparent',
                  }}
                >
                  <span style={{ fontFamily: theme.mono, fontSize: 12, color: theme.textDim, flex: 1, overflow: 'hidden' as const, textOverflow: 'ellipsis' as const, whiteSpace: 'nowrap' as const }}>{f.name}</span>
                  <span style={{ fontFamily: theme.mono, fontSize: 11, color: theme.accent2, background: `${theme.accent2}18`, padding: '1px 5px', borderRadius: 4 }}>{f.badge}</span>
                </div>
              );
            })}
          </div>

          {/* Diff view */}
          <div style={{ flex: 1, overflow: 'hidden' as const, display: 'flex', flexDirection: 'column' }}>
            <div style={{ padding: '10px 16px', borderBottom: `1px solid ${theme.border}`, background: theme.surface, flexShrink: 0 }}>
              <span style={{ fontFamily: theme.mono, fontSize: 13, color: theme.textDim }}>env/provisioner.go</span>
            </div>
            <div style={{ flex: 1, overflow: 'hidden' as const, fontFamily: theme.mono, fontSize: 12 }}>
              {DIFF_LINES.map((line, i) => {
                const op = interpolate(frame, [i * 5, i * 5 + 14], [0, 1], { extrapolateRight: 'clamp', extrapolateLeft: 'clamp' });
                const bg = line.t === 'add' ? `${theme.accent2}18` : line.t === 'del' ? `${theme.danger}18` : 'transparent';
                const color = line.t === 'add' ? theme.accent2 : line.t === 'del' ? theme.danger : line.t === 'hunk' ? theme.accent : theme.textDim;
                return (
                  <div key={i} style={{ opacity: op, background: bg, padding: '2px 16px', color, lineHeight: 1.65 }}>{line.text}</div>
                );
              })}

              {/* inline comment thread */}
              <div
                style={{
                  opacity: commentOp,
                  transform: `translateY(${commentY}px)`,
                  margin: '10px 16px',
                  background: theme.surface2,
                  border: `1px solid ${theme.border}`,
                  borderRadius: 8,
                  overflow: 'hidden' as const,
                }}
              >
                <div style={{ padding: '8px 12px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span style={{ fontSize: 14 }}>💬</span>
                  <span style={{ fontFamily: theme.font, fontSize: 12, color: theme.textDim }}>sara left a comment</span>
                </div>
                <div style={{ padding: '10px 12px', fontFamily: theme.font, fontSize: 13, color: theme.text, lineHeight: 1.5 }}>
                  Should we add a timeout on the Provision call? Service discovery might be slow at startup.
                </div>
                <div style={{ padding: '6px 12px 8px', display: 'flex', gap: 8 }}>
                  <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.accent, background: `${theme.accent}18`, padding: '2px 8px', borderRadius: 6 }}>Reply</span>
                  <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.textDim, background: `${theme.textDim}18`, padding: '2px 8px', borderRadius: 6 }}>Resolve</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </OttoWindow>

      <Caption step={4} title="Files — inline comment threads" sub="Diff view · 💬 comments · Resolve & Reply" delay={20} />
    </AbsoluteFill>
  );
};

// ─── Scene 6: AI Review Agents panel ─────────────────────────────────────────
// frames 655–930 (9.2s)

const AGENT_CONFIGS = [
  { name: 'Correctness', model: 'claude', color: theme.accent },
  { name: 'Security', model: 'codex', color: '#bf7aff' },
];

type AgentStatus = 'idle' | 'running' | 'done';

const AgentCard: React.FC<{
  name: string;
  model: string;
  color: string;
  status: AgentStatus;
  findingCount?: number;
  delay?: number;
}> = ({ name, model, color, status, findingCount, delay = 0 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 200 } });

  const statusLabel = status === 'running' ? 'running…' : status === 'done' ? 'done' : 'idle';
  const statusColor = status === 'running' ? theme.warn : status === 'done' ? theme.accent2 : theme.textDim;

  // pulsing dot for running
  const pulse = status === 'running' ? 0.6 + Math.sin(frame / 6) * 0.4 : 1;

  return (
    <div
      style={{
        opacity: interpolate(s, [0, 1], [0, 1]),
        transform: `translateY(${interpolate(s, [0, 1], [18, 0])}px)`,
        background: theme.surface2,
        border: `1px solid ${color}44`,
        borderRadius: 10,
        padding: '12px 16px',
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        boxShadow: status === 'done' ? `0 0 20px ${color}22` : 'none',
      }}
    >
      <div
        style={{
          width: 36,
          height: 36,
          borderRadius: 8,
          background: `${color}22`,
          display: 'grid',
          placeItems: 'center',
          fontSize: 18,
          flexShrink: 0,
        }}
      >
        {name === 'Correctness' ? '✓' : '🛡'}
      </div>
      <div style={{ flex: 1 }}>
        <div style={{ fontFamily: theme.font, fontSize: 14, fontWeight: 700, color: theme.text }}>{name}</div>
        <div style={{ fontFamily: theme.mono, fontSize: 11, color: theme.textDim }}>{model}</div>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <div style={{ width: 8, height: 8, borderRadius: '50%', background: statusColor, opacity: pulse }} />
        <span style={{ fontFamily: theme.font, fontSize: 12, color: statusColor, fontWeight: 600 }}>{statusLabel}</span>
      </div>
      {status === 'done' && findingCount != null && (
        <span style={{ fontFamily: theme.font, fontSize: 11, color: color, background: `${color}22`, padding: '2px 9px', borderRadius: 8, fontWeight: 700 }}>
          {findingCount} finding{findingCount !== 1 ? 's' : ''}
        </span>
      )}
    </div>
  );
};

const DraftComment: React.FC<{ delay?: number }> = ({ delay = 0 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 180 } });

  return (
    <div
      style={{
        opacity: interpolate(s, [0, 1], [0, 1]),
        transform: `translateY(${interpolate(s, [0, 1], [20, 0])}px)`,
        background: theme.surface2,
        border: `1px solid ${theme.border}`,
        borderLeft: `3px solid ${theme.warn}`,
        borderRadius: 8,
        overflow: 'hidden' as const,
        marginTop: 8,
      }}
    >
      <div style={{ padding: '8px 12px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 8 }}>
        <span style={{ fontFamily: theme.font, fontSize: 12, fontWeight: 700, color: theme.warn }}>⚠ MEDIUM</span>
        <span style={{ fontFamily: theme.font, fontSize: 12, color: theme.textDim }}>Correctness · env/provisioner.go:18</span>
      </div>
      {/* mini diff */}
      <div style={{ background: `${theme.danger}10`, padding: '4px 12px', fontFamily: theme.mono, fontSize: 11, color: theme.danger }}>
        - return legacyProvision(cfg)
      </div>
      <div style={{ background: `${theme.accent2}10`, padding: '4px 12px', fontFamily: theme.mono, fontSize: 11, color: theme.accent2 }}>
        + return svc.Provision(ctx, cfg)
      </div>
      <div style={{ padding: '8px 12px', fontFamily: theme.font, fontSize: 13, color: theme.text, lineHeight: 1.5 }}>
        Context cancellation is not propagated — wrap with a timeout before calling Provision.
      </div>
      <div style={{ padding: '8px 12px', display: 'flex', gap: 8, borderTop: `1px solid ${theme.border}` }}>
        <div style={{ padding: '5px 14px', borderRadius: 6, background: `${theme.accent2}22`, border: `1px solid ${theme.accent2}55`, fontFamily: theme.font, fontSize: 12, fontWeight: 700, color: theme.accent2 }}>Approve</div>
        <div style={{ padding: '5px 14px', borderRadius: 6, background: `${theme.danger}22`, border: `1px solid ${theme.danger}55`, fontFamily: theme.font, fontSize: 12, fontWeight: 700, color: theme.danger }}>Request Changes</div>
        <div style={{ padding: '5px 14px', borderRadius: 6, background: 'transparent', border: `1px solid ${theme.border}`, fontFamily: theme.font, fontSize: 12, color: theme.textDim }}>Decline</div>
      </div>
    </div>
  );
};

const SceneReviewAgents: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // Phase 1: configure panel (0-50f)
  const configOp = interpolate(frame, [0, 16], [0, 1], { extrapolateRight: 'clamp' });
  // Phase 2: "Send to review agents" click at 50f
  const sendS = spring({ frame: frame - 50, fps, config: { damping: 200 } });
  const sendOp = interpolate(sendS, [0, 1], [0, 1]);
  // Agent cards running state at 65f, done state at 120f
  const agent1Status: AgentStatus = frame < 65 ? 'idle' : frame < 120 ? 'running' : 'done';
  const agent2Status: AgentStatus = frame < 70 ? 'idle' : frame < 130 ? 'running' : 'done';

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <OttoWindow
        sidebar={<Navigator active="git" />}
        title="Otto — #22 · Review"
        style={{ width: 1440, height: 820 }}
      >
        <div style={{ display: 'flex', height: '100%' }}>
          {/* Left: PR tab bar + configure panel */}
          <div style={{ width: 520, borderRight: `1px solid ${theme.border}`, display: 'flex', flexDirection: 'column', padding: '0' }}>
            {/* Tab bar */}
            <div style={{ padding: '10px 16px', borderBottom: `1px solid ${theme.border}`, display: 'flex', gap: 6, flexShrink: 0, background: theme.surface }}>
              {PR_TABS.map((tab) => {
                const isActive = tab === 'Review';
                return (
                  <div
                    key={tab}
                    style={{
                      padding: '4px 12px',
                      borderRadius: 6,
                      background: isActive ? `${theme.accent}22` : 'transparent',
                      border: `1px solid ${isActive ? theme.accent : 'transparent'}`,
                      fontFamily: theme.font,
                      fontSize: 13,
                      fontWeight: isActive ? 700 : 500,
                      color: isActive ? theme.accent : theme.textDim,
                    }}
                  >
                    {tab}
                  </div>
                );
              })}
            </div>

            {/* Configure agents section */}
            <div style={{ opacity: configOp, padding: '16px', flex: 1 }}>
              <div style={{ fontFamily: theme.font, fontSize: 13, fontWeight: 700, color: theme.textDim, letterSpacing: 1, textTransform: 'uppercase' as const, marginBottom: 12 }}>
                ⚙ Configure Review Agents
              </div>
              {AGENT_CONFIGS.map((cfg, i) => {
                const cfgS = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
                return (
                  <div
                    key={cfg.name}
                    style={{
                      opacity: interpolate(cfgS, [0, 1], [0, 1]),
                      display: 'flex',
                      alignItems: 'center',
                      gap: 10,
                      padding: '10px 12px',
                      background: theme.surface2,
                      border: `1px solid ${cfg.color}44`,
                      borderRadius: 8,
                      marginBottom: 8,
                    }}
                  >
                    <div style={{ width: 8, height: 8, borderRadius: '50%', background: cfg.color }} />
                    <span style={{ fontFamily: theme.font, fontSize: 14, color: theme.text, flex: 1, fontWeight: 600 }}>{cfg.name}</span>
                    <span style={{ fontFamily: theme.mono, fontSize: 12, color: theme.textDim }}>run on</span>
                    <span style={{ fontFamily: theme.mono, fontSize: 12, color: cfg.color, background: `${cfg.color}18`, padding: '2px 8px', borderRadius: 5 }}>{cfg.model}</span>
                  </div>
                );
              })}

              {/* Send button */}
              <div
                style={{
                  opacity: sendOp,
                  marginTop: 16,
                  padding: '10px 18px',
                  background: theme.accent,
                  border: 'none',
                  borderRadius: 8,
                  fontFamily: theme.font,
                  fontSize: 14,
                  fontWeight: 700,
                  color: '#fff',
                  textAlign: 'center' as const,
                  boxShadow: `0 6px 24px ${theme.accent}55`,
                  cursor: 'default',
                }}
              >
                Send to review agents →
              </div>
            </div>
          </div>

          {/* Right: Live agent cards + draft comments */}
          <div style={{ flex: 1, padding: '16px 18px', overflow: 'hidden' as const }}>
            {frame > 55 && (
              <Appear delay={0} y={12}>
                <div style={{ fontFamily: theme.font, fontSize: 13, fontWeight: 700, color: theme.textDim, letterSpacing: 1, textTransform: 'uppercase' as const, marginBottom: 12 }}>
                  Review Agents
                </div>
              </Appear>
            )}

            {frame > 58 && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                <AgentCard
                  name="Correctness"
                  model="claude"
                  color={theme.accent}
                  status={agent1Status}
                  findingCount={2}
                  delay={4}
                />
                <AgentCard
                  name="Security"
                  model="codex"
                  color="#bf7aff"
                  status={agent2Status}
                  findingCount={1}
                  delay={12}
                />
              </div>
            )}

            {/* Draft comments appear after both done */}
            {frame > 130 && (
              <div style={{ marginTop: 16 }}>
                <div style={{ fontFamily: theme.font, fontSize: 12, fontWeight: 700, color: theme.textDim, letterSpacing: 1, textTransform: 'uppercase' as const, marginBottom: 8 }}>
                  Draft Comments — approve or request changes
                </div>
                <DraftComment delay={0} />
              </div>
            )}
          </div>
        </div>

        {frame > 44 && frame < 70 && (
          <Cursor from={[400, 580]} to={[280, 560]} startAt={0} duration={20} click />
        )}
      </OttoWindow>

      {frame < 120 && (
        <Caption step={5} title="⚙ Configure review agents" sub="Correctness + Security — claude · codex · live progress" delay={10} />
      )}
      {frame >= 120 && (
        <Caption step={5} title="Draft comments — approve or decline" sub="Severity chip · diff snippet · Request Changes" delay={0} />
      )}
    </AbsoluteFill>
  );
};

// ─── Scene 7: Review local changes → send findings to coding agent ─────────────
// frames 925–1080 (5.2s)

const FINDINGS = [
  { severity: 'HIGH', text: 'Missing context.WithTimeout in ProvisionEnv', file: 'env/provisioner.go:18' },
  { severity: 'MEDIUM', text: 'Unchecked error return from svc.Provision', file: 'env/provisioner.go:21' },
  { severity: 'LOW', text: 'Dead code: legacyProvision is never called', file: 'env/provisioner.go:34' },
];

const SceneLocalReview: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const agentSpawnS = spring({ frame: frame - 80, fps, config: { damping: 180 } });
  const agentSpawnOp = interpolate(agentSpawnS, [0, 1], [0, 1]);
  const agentSpawnX = interpolate(agentSpawnS, [0, 1], [40, 0]);

  return (
    <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <OttoWindow
        sidebar={<Navigator active="git" />}
        title="Otto — Review Local Changes vs origin/develop"
        style={{ width: 1440, height: 820 }}
      >
        <div style={{ display: 'flex', height: '100%', padding: '24px', gap: 24 }}>
          {/* Findings list */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 10 }}>
            <Appear delay={10} y={12}>
              <div style={{ fontFamily: theme.font, fontSize: 15, fontWeight: 700, color: theme.text, marginBottom: 6 }}>
                Review: local vs origin/develop
              </div>
            </Appear>
            <Appear delay={18} y={8}>
              <div style={{ fontFamily: theme.font, fontSize: 13, color: theme.textDim, marginBottom: 14 }}>
                3 findings from claude · 2 files changed
              </div>
            </Appear>
            {FINDINGS.map((f, i) => {
              const s = spring({ frame: frame - 24 - i * 14, fps, config: { damping: 200 } });
              const sevColor = f.severity === 'HIGH' ? theme.danger : f.severity === 'MEDIUM' ? theme.warn : theme.textDim;
              return (
                <div
                  key={i}
                  style={{
                    opacity: interpolate(s, [0, 1], [0, 1]),
                    transform: `translateX(${interpolate(s, [0, 1], [-14, 0])}px)`,
                    background: theme.surface2,
                    border: `1px solid ${sevColor}44`,
                    borderLeft: `3px solid ${sevColor}`,
                    borderRadius: 8,
                    padding: '12px 14px',
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
                    <span style={{ fontFamily: theme.font, fontSize: 11, fontWeight: 800, color: sevColor, background: `${sevColor}22`, padding: '2px 8px', borderRadius: 6 }}>{f.severity}</span>
                    <span style={{ fontFamily: theme.mono, fontSize: 11, color: theme.textDim }}>{f.file}</span>
                  </div>
                  <div style={{ fontFamily: theme.font, fontSize: 13, color: theme.text }}>{f.text}</div>
                </div>
              );
            })}

            {/* Send to coding agent button */}
            {frame > 70 && (
              <Appear delay={0} y={10}>
                <div
                  style={{
                    marginTop: 8,
                    padding: '10px 18px',
                    background: `${theme.accent2}22`,
                    border: `1px solid ${theme.accent2}`,
                    borderRadius: 8,
                    fontFamily: theme.font,
                    fontSize: 14,
                    fontWeight: 700,
                    color: theme.accent2,
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    cursor: 'default',
                  }}
                >
                  <span>▸_</span>
                  <span>Send findings to coding agent →</span>
                </div>
              </Appear>
            )}
          </div>

          {/* Spawned claude agent tile */}
          {frame > 80 && (
            <div
              style={{
                opacity: agentSpawnOp,
                transform: `translateX(${agentSpawnX}px)`,
                width: 380,
                background: `${theme.accent}0e`,
                border: `1px solid ${theme.accent}44`,
                borderRadius: 12,
                padding: '16px',
                flexShrink: 0,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
                <div style={{ width: 32, height: 32, borderRadius: 8, background: `${theme.accent}22`, display: 'grid', placeItems: 'center', fontSize: 16 }}>▸_</div>
                <div>
                  <div style={{ fontFamily: theme.font, fontSize: 13, fontWeight: 700, color: theme.text }}>claude · fix-review-findings</div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 5, marginTop: 2 }}>
                    <div style={{ width: 7, height: 7, borderRadius: '50%', background: theme.accent2, opacity: 0.7 + Math.sin(frame / 6) * 0.3 }} />
                    <span style={{ fontFamily: theme.font, fontSize: 11, color: theme.accent2 }}>working</span>
                  </div>
                </div>
              </div>
              {[
                { t: 0, text: 'Received 3 findings from review agent', color: theme.textDim },
                { t: 10, text: '→ Adding context.WithTimeout…', color: theme.text },
                { t: 22, text: '→ Wrapping error returns…', color: theme.text },
                { t: 36, text: '→ Removing dead code…', color: theme.text },
              ].map((line, i) => {
                const lineOp = interpolate(frame - 85, [line.t, line.t + 10], [0, 1], { extrapolateRight: 'clamp', extrapolateLeft: 'clamp' });
                return (
                  <div key={i} style={{ opacity: lineOp, fontFamily: theme.mono, fontSize: 12, color: line.color, lineHeight: 1.7 }}>{line.text}</div>
                );
              })}
            </div>
          )}
        </div>
      </OttoWindow>

      <Caption step={6} title="Review local changes → code agent" sub="Findings list → spawn claude to apply fixes" delay={20} />
    </AbsoluteFill>
  );
};

// ─── Scene 8: Outro — "Ship with confidence." ─────────────────────────────────
// frames 1075–1380 (10.2s)

const SceneOutro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const pulse = Math.sin(frame / 20) * 0.5 + 0.5;
  const glowSize = interpolate(pulse, [0, 1], [60, 110]);

  const logoS = spring({ frame, fps, config: { damping: 18, stiffness: 100 } });
  const logoScale = interpolate(logoS, [0, 1], [0.6, 1]);
  const logoOp = interpolate(frame, [0, 18], [0, 1], { extrapolateRight: 'clamp' });

  const textS = spring({ frame: frame - 16, fps, config: { damping: 200 } });
  const textOp = interpolate(textS, [0, 1], [0, 1]);
  const textY = interpolate(textS, [0, 1], [24, 0]);

  const subS = spring({ frame: frame - 32, fps, config: { damping: 200 } });
  const subOp = interpolate(subS, [0, 1], [0, 1]);

  const pillsS = spring({ frame: frame - 60, fps, config: { damping: 200 } });
  const pillsOp = interpolate(pillsS, [0, 1], [0, 1]);
  const pillsY = interpolate(pillsS, [0, 1], [16, 0]);

  const FEATURES = [
    { label: 'Commit graph', color: theme.accent },
    { label: 'Branch tree', color: theme.accent2 },
    { label: 'PR review', color: '#bf7aff' },
    { label: 'AI agents', color: theme.warn },
    { label: 'Inline comments', color: theme.accent },
  ];

  return (
    <AbsoluteFill
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 0,
      }}
    >
      <div style={{ opacity: logoOp, transform: `scale(${logoScale})`, marginBottom: 32 }}>
        <Img
          src={staticFile('otto-mark.png')}
          style={{
            width: 150,
            height: 150,
            borderRadius: 36,
            boxShadow: `0 0 0 1.5px ${theme.accent}55, 0 20px ${glowSize}px ${theme.accent}55`,
          }}
        />
      </div>

      <div
        style={{
          opacity: textOp,
          transform: `translateY(${textY}px)`,
          fontFamily: theme.font,
          fontSize: 100,
          fontWeight: 900,
          color: theme.text,
          letterSpacing: -3,
          lineHeight: 1,
          textAlign: 'center' as const,
        }}
      >
        Ship with confidence.
      </div>

      <div
        style={{
          opacity: subOp,
          fontFamily: theme.font,
          fontSize: 30,
          fontWeight: 500,
          color: theme.textDim,
          marginTop: 22,
          textAlign: 'center' as const,
        }}
      >
        Git & PRs — powered by AI review agents
      </div>

      {/* Feature pills */}
      <div
        style={{
          opacity: pillsOp,
          transform: `translateY(${pillsY}px)`,
          display: 'flex',
          gap: 14,
          flexWrap: 'wrap' as const,
          justifyContent: 'center',
          marginTop: 40,
          maxWidth: 900,
        }}
      >
        {FEATURES.map((f) => (
          <div
            key={f.label}
            style={{
              padding: '8px 20px',
              borderRadius: 30,
              background: `${f.color}18`,
              border: `1px solid ${f.color}44`,
              fontFamily: theme.font,
              fontSize: 18,
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

      {/* Accent line */}
      <div
        style={{
          position: 'absolute',
          bottom: '32%',
          left: '50%',
          transform: 'translateX(-50%)',
          width: interpolate(frame, [50, 100], [0, 500], { extrapolateRight: 'clamp', extrapolateLeft: 'clamp' }),
          height: 1.5,
          background: `linear-gradient(90deg, transparent, ${theme.accent}88, ${theme.accent2}88, transparent)`,
        }}
      />
    </AbsoluteFill>
  );
};

// ─── Root composition (46s = 1380 frames @ 30fps) ────────────────────────────
//
// Scene 1  Title           0   – 80    (2.7s)
// Scene 2  Repo + graph   60   – 250   (6.3s)  overlap
// Scene 3  Commit diff   235   – 370   (4.5s)  overlap
// Scene 4  PR list/open  355   – 520   (5.5s)  overlap
// Scene 5  Files tab     505   – 665   (5.3s)  overlap
// Scene 6  AI agents     650   – 935   (9.5s)  overlap
// Scene 7  Local review  920   – 1085  (5.5s)  overlap
// Scene 8  Outro        1070   – 1380  (10.3s) overlap

export const GitPr: React.FC = () => (
  <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>
    {/* Scene 1: Title */}
    <Sequence from={0} durationInFrames={90}>
      <FadeIn durationFrames={14}>
        <AbsoluteFill>
          <SceneTitle />
        </AbsoluteFill>
      </FadeIn>
    </Sequence>

    {/* Scene 2: Repo view */}
    <Sequence from={60} durationInFrames={195}>
      <FadeIn durationFrames={16}>
        <AbsoluteFill>
          <SceneRepoView />
        </AbsoluteFill>
      </FadeIn>
    </Sequence>

    {/* Scene 3: Commit diff */}
    <Sequence from={235} durationInFrames={140}>
      <FadeIn durationFrames={16}>
        <AbsoluteFill>
          <SceneCommitDiff />
        </AbsoluteFill>
      </FadeIn>
    </Sequence>

    {/* Scene 4: Pull Requests */}
    <Sequence from={355} durationInFrames={160}>
      <FadeIn durationFrames={16}>
        <AbsoluteFill>
          <ScenePullRequests />
        </AbsoluteFill>
      </FadeIn>
    </Sequence>

    {/* Scene 5: Files tab */}
    <Sequence from={505} durationInFrames={160}>
      <FadeIn durationFrames={16}>
        <AbsoluteFill>
          <SceneFilesTab />
        </AbsoluteFill>
      </FadeIn>
    </Sequence>

    {/* Scene 6: AI Review Agents */}
    <Sequence from={650} durationInFrames={290}>
      <FadeIn durationFrames={16}>
        <AbsoluteFill>
          <SceneReviewAgents />
        </AbsoluteFill>
      </FadeIn>
    </Sequence>

    {/* Scene 7: Local review → coding agent */}
    <Sequence from={920} durationInFrames={170}>
      <FadeIn durationFrames={16}>
        <AbsoluteFill>
          <SceneLocalReview />
        </AbsoluteFill>
      </FadeIn>
    </Sequence>

    {/* Scene 8: Outro */}
    <Sequence from={1070} durationInFrames={310}>
      <FadeIn durationFrames={20}>
        <AbsoluteFill>
          <SceneOutro />
        </AbsoluteFill>
      </FadeIn>
    </Sequence>
  </AbsoluteFill>
);
