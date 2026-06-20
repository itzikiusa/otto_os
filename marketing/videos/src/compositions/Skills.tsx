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

// ─── Skills — bundled, versioned skill library — ~28s ────────────────────────
// Browse the skill catalogue, install a skill, run a review lens,
// then show the eval / compare output.
// ─────────────────────────────────────────────────────────────────────────────

const TITLE_DUR  = 75;
const S1_DUR     = 195;  // skill catalogue
const S2_DUR     = 150;  // skill detail / install
const S3_DUR     = 120;  // eval output
const OUTRO_DUR  = 90;

const S1_START   = TITLE_DUR;
const S2_START   = S1_START + S1_DUR;
const S3_START   = S2_START + S2_DUR;
const OUTRO_START = S3_START + S3_DUR;

// ─── Data ─────────────────────────────────────────────────────────────────────
const SKILLS = [
  { name: 'golang-code-review',         tag: 'review',   installed: true,  desc: 'Three-pillar Go code review: logging, docs, structure.' },
  { name: 'golang-feature-impl',        tag: 'feature',  installed: true,  desc: 'Full implementation workflow for Casino Platform Go services.' },
  { name: 'golang-testing',             tag: 'testing',  installed: true,  desc: 'Component, integration, and unit test patterns.' },
  { name: 'casino-prd-creator',         tag: 'product',  installed: false, desc: 'Turn a raw idea into a build-ready PRD with acceptance criteria.' },
  { name: 'weekly-insights',            tag: 'insights', installed: false, desc: 'Generate a formatted weekly summary from activity data.' },
  { name: 'architecture-review',        tag: 'review',   installed: true,  desc: 'System-level design and scalability review lens.' },
  { name: 'acceptance-test-creator',    tag: 'testing',  installed: false, desc: 'Convert acceptance criteria into a runnable test plan.' },
];

const TAG_COLOR: Record<string, string> = {
  review:   theme.accent,
  feature:  theme.accent2,
  testing:  '#bf7aff',
  product:  theme.warn,
  insights: '#63e6be',
};

const TagBadge: React.FC<{ tag: string }> = ({ tag }) => {
  const c = TAG_COLOR[tag] ?? theme.textDim;
  return <span style={{ fontFamily: theme.mono, fontSize: 11, fontWeight: 700, color: c, background: `${c}22`, border: `1px solid ${c}44`, borderRadius: 6, padding: '2px 8px', letterSpacing: 0.4 }}>{tag}</span>;
};

// ─── Scene 1 – Skill catalogue ────────────────────────────────────────────────
const FILTER_TAGS = ['All', 'review', 'feature', 'testing', 'product', 'insights'] as const;

const Scene1Catalogue: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const activeTag = frame < 90 ? 'All' : 'review';

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      <Appear delay={4}>
        <div style={{ padding: '22px 28px 0', display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between' }}>
          <div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 26, fontWeight: 800 }}>Skills</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 15, marginTop: 4 }}>Bundled, versioned — drive reviews, analysis, and automation</div>
          </div>
          <div style={{ padding: '9px 20px', borderRadius: 10, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 14 }}>Check for updates</div>
        </div>
      </Appear>

      {/* tag filter */}
      <Appear delay={14}>
        <div style={{ display: 'flex', gap: 8, padding: '16px 28px 0' }}>
          {FILTER_TAGS.map((t) => {
            const isActive = t === activeTag || (activeTag === 'All' && t === 'All');
            const c = TAG_COLOR[t] ?? theme.textDim;
            return (
              <div key={t} style={{ padding: '6px 14px', borderRadius: 20, background: isActive ? `${c}18` : 'transparent', border: `1px solid ${isActive ? c : theme.border}`, color: isActive ? c : theme.textDim, fontFamily: theme.font, fontSize: 12, fontWeight: isActive ? 700 : 400 }}>{t}</div>
            );
          })}
        </div>
      </Appear>

      {/* skill cards */}
      <div style={{ flex: 1, overflow: 'hidden', padding: '16px 28px 24px', display: 'flex', flexDirection: 'column', gap: 10 }}>
        {SKILLS.map((sk, i) => {
          const s = spring({ frame: frame - (i * 10 + 24), fps, config: { damping: 200 } });
          return (
            <div key={sk.name} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [12, 0])}px)`, background: theme.surface2, borderRadius: 12, border: `1px solid ${theme.border}`, padding: '14px 20px', display: 'flex', alignItems: 'center', gap: 18 }}>
              <div style={{ width: 10, height: 10, borderRadius: '50%', background: TAG_COLOR[sk.tag] ?? theme.textDim, flexShrink: 0 }} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 4 }}>
                  <span style={{ color: theme.text, fontFamily: theme.mono, fontSize: 14, fontWeight: 700 }}>{sk.name}</span>
                  <TagBadge tag={sk.tag} />
                  {sk.installed && (
                    <span style={{ color: theme.accent2, fontFamily: theme.mono, fontSize: 11 }}>● installed</span>
                  )}
                </div>
                <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>{sk.desc}</div>
              </div>
              <div style={{ padding: '6px 16px', borderRadius: 8, background: sk.installed ? 'transparent' : theme.accent, border: sk.installed ? `1px solid ${theme.border}` : 'none', color: sk.installed ? theme.textDim : '#fff', fontFamily: theme.font, fontSize: 13, fontWeight: sk.installed ? 400 : 700, boxShadow: sk.installed ? 'none' : `0 4px 14px ${theme.accent}44`, flexShrink: 0 }}>
                {sk.installed ? 'Update' : 'Install'}
              </div>
            </div>
          );
        })}
      </div>

      <Caption step={1} title="Skill catalogue" sub="Browse, install, and update bundled skill lenses" delay={55} />
    </div>
  );
};

// ─── Scene 2 – Skill detail + invoke ──────────────────────────────────────────
const Scene2Detail: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const detailS = spring({ frame, fps, config: { damping: 180 } });
  const INVOKE_START = 80;
  const showInvoke = frame >= INVOKE_START;
  const invokeS = spring({ frame: frame - INVOKE_START, fps, config: { damping: 180 } });

  return (
    <div style={{ display: 'flex', height: '100%', alignItems: 'center', justifyContent: 'center', padding: 32 }}>
      <div style={{ opacity: detailS, transform: `scale(${interpolate(detailS, [0, 1], [0.92, 1])})`, width: '90%', maxWidth: 760, background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: 18, boxShadow: '0 40px 100px rgba(0,0,0,0.6)', overflow: 'hidden' }}>
        {/* header */}
        <div style={{ padding: '24px 28px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'flex-start', gap: 18 }}>
          <div style={{ width: 52, height: 52, borderRadius: 14, background: `${theme.accent}22`, border: `1px solid ${theme.accent}44`, display: 'grid', placeItems: 'center', fontSize: 26, flexShrink: 0 }}>🔍</div>
          <div style={{ flex: 1 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 4 }}>
              <span style={{ color: theme.text, fontFamily: theme.mono, fontSize: 18, fontWeight: 800 }}>golang-code-review</span>
              <TagBadge tag="review" />
              <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12 }}>v2.4.1</span>
              <span style={{ color: theme.accent2, fontFamily: theme.mono, fontSize: 11, marginLeft: 4 }}>● up to date</span>
            </div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14 }}>
              Three-pillar Go code review: logging correctness, documentation quality, code structure. Enforces Casino Platform standards.
            </div>
          </div>
        </div>
        {/* detail body */}
        <div style={{ padding: '20px 28px', display: 'flex', flexDirection: 'column', gap: 14 }}>
          <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 1, textTransform: 'uppercase', marginBottom: 2 }}>Pillars</div>
          {[
            { icon: '📋', label: 'Logging', desc: 'context-aware · level correctness · no Printf' },
            { icon: '📖', label: 'Documentation', desc: 'exported symbols · package-level comments' },
            { icon: '🏗', label: 'Code structure', desc: 'SOLID · naming · file organization' },
          ].map(({ icon, label, desc }) => (
            <div key={label} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '10px 16px', background: theme.surface2, borderRadius: 10, border: `1px solid ${theme.border}` }}>
              <span style={{ fontSize: 18 }}>{icon}</span>
              <div>
                <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 14, fontWeight: 700 }}>{label}</div>
                <div style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12, marginTop: 2 }}>{desc}</div>
              </div>
            </div>
          ))}
        </div>
        {/* invoke footer */}
        <div style={{ padding: '16px 28px', borderTop: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{ flex: 1, background: theme.surface2, borderRadius: 8, padding: '8px 14px', fontFamily: theme.mono, fontSize: 14, color: theme.textDim }}>
            /golang-code-review
          </div>
          {showInvoke && (
            <div style={{ opacity: invokeS, transform: `scale(${interpolate(invokeS, [0, 1], [0.8, 1])})`, padding: '9px 24px', background: theme.accent, borderRadius: 10, color: '#fff', fontFamily: theme.font, fontSize: 14, fontWeight: 700, boxShadow: `0 4px 16px ${theme.accent}44`, flexShrink: 0 }}>
              Run skill
            </div>
          )}
        </div>
      </div>

      <Caption step={2} title="Skill detail" sub="Invoke with /skill-name — or from any review flow" delay={50} />
    </div>
  );
};

// ─── Scene 3 – Eval output (compare view) ─────────────────────────────────────
const EVAL_ROWS = [
  { file: 'service/player_service.go',  issue: 'Missing ctx argument in logger.ErrorF call',       severity: 'error' },
  { file: 'dao/wallet_dao.go',          issue: 'Exported function GetBalance lacks godoc comment',  severity: 'warn'  },
  { file: 'controller/auth.go',         issue: 'Hard-coded timeout; use config parameter instead',  severity: 'info'  },
  { file: 'dao/bonus_dao.go',           issue: 'Loop SQL query — replace with IN-clause batch',     severity: 'warn'  },
];

const SEV_COLOR: Record<string, string> = {
  error: theme.danger,
  warn:  theme.warn,
  info:  theme.textDim,
};

const Scene3Eval: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      {/* toolbar */}
      <div style={{ padding: '14px 24px', borderBottom: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 12, flexShrink: 0 }}>
        <span style={{ color: theme.accent2, fontSize: 16 }}>✓</span>
        <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 15, fontWeight: 700 }}>golang-code-review</span>
        <span style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 13 }}>sinatra-users-go · 4 findings</span>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          {['error', 'warn', 'info'].map((sev) => {
            const count = EVAL_ROWS.filter((r) => r.severity === sev).length;
            return (
              <div key={sev} style={{ display: 'flex', alignItems: 'center', gap: 5, padding: '4px 12px', borderRadius: 8, background: `${SEV_COLOR[sev]}18`, border: `1px solid ${SEV_COLOR[sev]}44` }}>
                <span style={{ color: SEV_COLOR[sev], fontFamily: theme.mono, fontSize: 13, fontWeight: 700 }}>{count}</span>
                <span style={{ color: SEV_COLOR[sev], fontFamily: theme.font, fontSize: 12 }}>{sev}</span>
              </div>
            );
          })}
        </div>
      </div>

      {/* findings list */}
      <div style={{ flex: 1, overflow: 'hidden', padding: '12px 0' }}>
        {EVAL_ROWS.map((row, i) => {
          const s = spring({ frame: frame - i * 14, fps, config: { damping: 200 } });
          return (
            <div key={i} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [12, 0])}px)`, padding: '14px 24px', borderBottom: `1px solid ${theme.border}22`, display: 'flex', alignItems: 'flex-start', gap: 16 }}>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: SEV_COLOR[row.severity], marginTop: 5, flexShrink: 0 }} />
              <div style={{ flex: 1 }}>
                <div style={{ color: theme.textDim, fontFamily: theme.mono, fontSize: 12, marginBottom: 4 }}>{row.file}</div>
                <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 14 }}>{row.issue}</div>
              </div>
              <div style={{ padding: '4px 12px', borderRadius: 8, background: `${SEV_COLOR[row.severity]}18`, border: `1px solid ${SEV_COLOR[row.severity]}44`, color: SEV_COLOR[row.severity], fontFamily: theme.mono, fontSize: 11, fontWeight: 700, flexShrink: 0 }}>
                {row.severity}
              </div>
            </div>
          );
        })}
      </div>

      <Caption step={3} title="Skill output" sub="Structured findings — file, issue, severity — ready to act on" delay={45} />
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
      <div style={{ opacity: t1, transform: `scale(${interpolate(t1, [0, 1], [0.5, 1])})`, fontSize: 80 }}>⚡</div>
      <div style={{ opacity: t2, transform: `translateY(${interpolate(t2, [0, 1], [24, 0])}px)`, color: theme.text, fontFamily: theme.font, fontSize: 64, fontWeight: 800, textAlign: 'center' }}>
        Expert lenses, on demand.
      </div>
      <div style={{ opacity: t3, transform: `translateY(${interpolate(t3, [0, 1], [16, 0])}px)`, color: theme.textDim, fontFamily: theme.font, fontSize: 24, textAlign: 'center' }}>
        Code review · Testing · Product analysis · Insights · Architecture
      </div>
    </div>
  );
};

// ─── Root composition ─────────────────────────────────────────────────────────
export const Skills: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>

      <Sequence durationInFrames={TITLE_DUR}>
        <TitleCard kicker="OTTO ADE" title="Skills" subtitle="Bundled, versioned expertise" />
      </Sequence>

      <Sequence from={S1_START} durationInFrames={S1_DUR + S2_DUR + S3_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Skills">
            <Sequence durationInFrames={S1_DUR}>
              <Scene1Catalogue />
            </Sequence>
            <Sequence from={S1_DUR} durationInFrames={S2_DUR}>
              <Scene2Detail />
            </Sequence>
            <Sequence from={S1_DUR + S2_DUR} durationInFrames={S3_DUR}>
              <Scene3Eval />
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
