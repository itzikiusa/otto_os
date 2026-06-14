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
import { Appear, Caption, Cursor, TitleCard } from '../components/ui';

// ─── timing constants (frames @ 30fps) ────────────────────────────────────────
const TITLE_DUR  = 75;   // 0–75  (2.5s title card)
const APP_START  = 75;   // 75…   (appearance scene)
const APP_DUR    = 165;  // 2.75s → ends at 240
const PROV_START = 240;
const PROV_DUR   = 180;  // ends 420
const GIT_START  = 420;
const GIT_DUR    = 165;  // ends 585
const CHAN_START  = 585;
const CHAN_DUR    = 150;  // ends 735
const OUTRO_START = 735;
const OUTRO_DUR  = 345;  // ends 1080

// ─── shared left nav ──────────────────────────────────────────────────────────
const NAV_ITEMS = [
  'Appearance',
  'Providers',
  'Git Accounts',
  'Jira',
  'Channels',
  'Users',
];

const SettingsNav: React.FC<{ active: string }> = ({ active }) => (
  <div style={{ padding: '24px 0', display: 'flex', flexDirection: 'column', gap: 2 }}>
    <div style={{ color: theme.textDim, fontSize: 11, fontWeight: 700, letterSpacing: 2, padding: '0 20px 12px', textTransform: 'uppercase' }}>
      Settings
    </div>
    {NAV_ITEMS.map((item) => {
      const isActive = item === active;
      return (
        <div
          key={item}
          style={{
            padding: '10px 20px',
            borderRadius: 8,
            margin: '0 8px',
            background: isActive ? `${theme.accent}22` : 'transparent',
            color: isActive ? theme.accent : theme.textDim,
            fontFamily: theme.font,
            fontSize: 15,
            fontWeight: isActive ? 700 : 400,
            borderLeft: isActive ? `3px solid ${theme.accent}` : '3px solid transparent',
            cursor: 'default',
            transition: 'all 0.2s',
          }}
        >
          {item}
        </div>
      );
    })}
  </div>
);

// ─── panel fade/slide transition helper ──────────────────────────────────────
const PanelFade: React.FC<{ children: React.ReactNode; delay?: number }> = ({ children, delay = 0 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const s = spring({ frame: frame - delay, fps, config: { damping: 180 } });
  return (
    <div
      style={{
        opacity: s,
        transform: `translateX(${interpolate(s, [0, 1], [18, 0])}px)`,
        height: '100%',
        overflow: 'hidden',
      }}
    >
      {children}
    </div>
  );
};

// ─── scene 1: Appearance ──────────────────────────────────────────────────────
const THEMES = [
  { name: 'Native',   accent: '#3d5bff', card: '#11151c' },
  { name: 'Pro Dark', accent: '#a259ff', card: '#0e0e16' },
  { name: 'Warm',     accent: '#ff7940', card: '#1a1410' },
];

const AppearancePanel: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  // cycle through themes based on time
  const idx = Math.floor(interpolate(frame, [0, APP_DUR - 30], [0, 2.99], { extrapolateRight: 'clamp' }));
  const th = THEMES[idx];
  return (
    <PanelFade>
      <div style={{ padding: '36px 40px', height: '100%', boxSizing: 'border-box', overflow: 'hidden' }}>
        <div style={{ color: theme.text, fontSize: 22, fontWeight: 800, marginBottom: 28 }}>Appearance</div>

        {/* theme swatches */}
        <div style={{ color: theme.textDim, fontSize: 13, fontWeight: 700, letterSpacing: 1.5, textTransform: 'uppercase', marginBottom: 14 }}>Theme</div>
        <div style={{ display: 'flex', gap: 16, marginBottom: 36 }}>
          {THEMES.map((t, i) => {
            const s = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
            const active = i === idx;
            return (
              <div key={t.name}
                style={{
                  opacity: s,
                  transform: `scale(${interpolate(s, [0, 1], [0.85, 1])})`,
                  flex: 1,
                  borderRadius: 14,
                  border: `2px solid ${active ? t.accent : theme.border}`,
                  overflow: 'hidden',
                  boxShadow: active ? `0 0 0 2px ${t.accent}55, 0 12px 40px ${t.accent}33` : 'none',
                  cursor: 'pointer',
                  transition: 'all 0.3s',
                }}
              >
                {/* mini mock preview */}
                <div style={{ background: t.card, padding: 14 }}>
                  <div style={{ height: 8, borderRadius: 4, background: t.accent, marginBottom: 7, width: '60%' }} />
                  <div style={{ height: 6, borderRadius: 3, background: `${theme.textDim}33`, marginBottom: 5, width: '80%' }} />
                  <div style={{ height: 6, borderRadius: 3, background: `${theme.textDim}22`, width: '55%' }} />
                </div>
                <div style={{ padding: '10px 14px', background: theme.surface2, display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <span style={{ color: active ? theme.text : theme.textDim, fontSize: 14, fontWeight: active ? 700 : 400 }}>{t.name}</span>
                  {active && <div style={{ width: 10, height: 10, borderRadius: '50%', background: t.accent }} />}
                </div>
              </div>
            );
          })}
        </div>

        {/* active theme accent preview bar */}
        <div style={{ background: theme.surface2, borderRadius: 12, padding: '18px 24px', marginBottom: 28, border: `1px solid ${theme.border}`, display: 'flex', alignItems: 'center', gap: 18 }}>
          <div style={{ width: 36, height: 36, borderRadius: 10, background: th.accent, boxShadow: `0 6px 20px ${th.accent}55` }} />
          <div>
            <div style={{ color: theme.text, fontSize: 15, fontWeight: 600 }}>Accent colour</div>
            <div style={{ color: theme.textDim, fontSize: 13, fontFamily: theme.mono }}>{th.accent}</div>
          </div>
        </div>

        {/* light/dark toggle */}
        <div style={{ color: theme.textDim, fontSize: 13, fontWeight: 700, letterSpacing: 1.5, textTransform: 'uppercase', marginBottom: 14 }}>Mode</div>
        <div style={{ display: 'flex', gap: 12 }}>
          {['Light', 'Dark', 'System'].map((m, i) => {
            const active = m === 'Dark';
            return (
              <div key={m} style={{
                padding: '10px 22px',
                borderRadius: 10,
                border: `1.5px solid ${active ? theme.accent : theme.border}`,
                background: active ? `${theme.accent}18` : 'transparent',
                color: active ? theme.accent : theme.textDim,
                fontSize: 15,
                fontWeight: active ? 700 : 400,
              }}>
                {m}
              </div>
            );
          })}
        </div>
      </div>
    </PanelFade>
  );
};

// ─── scene 2: Providers ───────────────────────────────────────────────────────
const BUILTIN_PROVIDERS = [
  { name: 'claude',  label: 'Claude (Anthropic)', cmd: 'claude update', color: theme.accent },
  { name: 'codex',   label: 'Codex CLI (OpenAI)',  cmd: 'codex upgrade', color: '#10b981' },
  { name: 'agy',     label: 'Agy',                 cmd: 'agy self-update', color: '#f59e0b' },
];

const ProvidersPanel: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const showForm = frame > 100;
  return (
    <PanelFade>
      <div style={{ padding: '36px 40px', height: '100%', boxSizing: 'border-box', overflow: 'hidden' }}>
        <div style={{ color: theme.text, fontSize: 22, fontWeight: 800, marginBottom: 28 }}>Providers</div>

        {/* built-in list */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginBottom: 28 }}>
          {BUILTIN_PROVIDERS.map((p, i) => {
            const s = spring({ frame: frame - i * 8, fps, config: { damping: 200 } });
            return (
              <div key={p.name} style={{
                opacity: s,
                transform: `translateX(${interpolate(s, [0, 1], [20, 0])}px)`,
                background: theme.surface2,
                borderRadius: 12,
                border: `1px solid ${theme.border}`,
                padding: '16px 20px',
                display: 'flex',
                alignItems: 'center',
                gap: 16,
              }}>
                <div style={{ width: 10, height: 10, borderRadius: '50%', background: p.color, flexShrink: 0 }} />
                <div style={{ flex: 1 }}>
                  <div style={{ color: theme.text, fontSize: 16, fontWeight: 600 }}>{p.label}</div>
                </div>
                <div style={{
                  background: theme.surface,
                  border: `1px solid ${theme.border}`,
                  borderRadius: 8,
                  padding: '6px 14px',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                }}>
                  <span style={{ color: theme.textDim, fontSize: 12 }}>update:</span>
                  <span style={{ color: theme.accent2, fontSize: 13, fontFamily: theme.mono }}>{p.cmd}</span>
                </div>
              </div>
            );
          })}
        </div>

        {/* + Add custom provider row / form */}
        {!showForm && (
          <div style={{
            border: `1.5px dashed ${theme.border}`,
            borderRadius: 12,
            padding: '16px 20px',
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            cursor: 'pointer',
          }}>
            <span style={{ color: theme.accent, fontSize: 22, fontWeight: 700, lineHeight: 1 }}>+</span>
            <span style={{ color: theme.accent, fontSize: 16, fontWeight: 600 }}>Add custom provider</span>
          </div>
        )}
        {showForm && (() => {
          const fS = spring({ frame: frame - 100, fps, config: { damping: 180 } });
          return (
            <div style={{
              opacity: fS,
              transform: `translateY(${interpolate(fS, [0, 1], [16, 0])}px)`,
              background: `${theme.accent}11`,
              border: `1.5px solid ${theme.accent}55`,
              borderRadius: 12,
              padding: '20px 24px',
            }}>
              <div style={{ color: theme.accent, fontSize: 15, fontWeight: 700, marginBottom: 16 }}>New Custom Provider</div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                {[['Name', 'opencode'], ['Command', 'opencode'], ['Update command', 'opencode update']].map(([label, val]) => (
                  <div key={label} style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
                    <div style={{ color: theme.textDim, fontSize: 13, width: 150, flexShrink: 0 }}>{label}</div>
                    <div style={{ flex: 1, background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: 8, padding: '8px 14px', color: theme.text, fontSize: 14, fontFamily: theme.mono }}>
                      {val}
                    </div>
                  </div>
                ))}
              </div>
              <div style={{ marginTop: 16, display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
                <div style={{ padding: '8px 20px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontSize: 14, cursor: 'pointer' }}>Cancel</div>
                <div style={{ padding: '8px 20px', borderRadius: 8, background: theme.accent, color: '#fff', fontSize: 14, fontWeight: 700, cursor: 'pointer', boxShadow: `0 6px 20px ${theme.accent}55` }}>Add Provider</div>
              </div>
            </div>
          );
        })()}
      </div>
    </PanelFade>
  );
};

// ─── scene 3: Git Accounts ────────────────────────────────────────────────────
const GitAccountsPanel: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const showKeychain = frame > 70;
  return (
    <PanelFade>
      <div style={{ padding: '36px 40px', height: '100%', boxSizing: 'border-box', overflow: 'hidden' }}>
        <div style={{ color: theme.text, fontSize: 22, fontWeight: 800, marginBottom: 28 }}>Git Accounts</div>

        {/* header row with Add button */}
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 18 }}>
          <div style={{ color: theme.textDim, fontSize: 13 }}>Linked accounts — tokens stored in macOS Keychain</div>
          <div style={{ padding: '8px 18px', borderRadius: 8, background: theme.accent, color: '#fff', fontSize: 13, fontWeight: 700, boxShadow: `0 4px 16px ${theme.accent}44` }}>+ Add Account</div>
        </div>

        {/* account rows */}
        {[
          { provider: 'Bitbucket', icon: '🗂', email: 'you@example.com', ns: 'your-org', color: '#2684ff' },
          { provider: 'GitHub',    icon: '🐙', email: 'you@example.com', ns: 'your-handle', color: theme.textDim },
          { provider: 'GitLab',    icon: '🦊', email: 'you@example.com', ns: 'otto-team', color: '#fc6d26' },
        ].map((acc, i) => {
          const s = spring({ frame: frame - i * 10, fps, config: { damping: 200 } });
          return (
            <div key={acc.provider} style={{
              opacity: s,
              transform: `translateX(${interpolate(s, [0, 1], [24, 0])}px)`,
              background: theme.surface2,
              borderRadius: 12,
              border: `1px solid ${theme.border}`,
              padding: '16px 20px',
              marginBottom: 10,
              display: 'flex',
              alignItems: 'center',
              gap: 16,
            }}>
              <div style={{ width: 40, height: 40, borderRadius: 10, background: `${acc.color}22`, border: `1px solid ${acc.color}44`, display: 'grid', placeItems: 'center', fontSize: 20, flexShrink: 0 }}>
                {acc.icon}
              </div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 4 }}>
                  <span style={{ color: theme.text, fontSize: 16, fontWeight: 700 }}>{acc.provider}</span>
                  <span style={{ background: `${acc.color}22`, color: acc.color, fontSize: 12, fontWeight: 600, padding: '2px 10px', borderRadius: 6, border: `1px solid ${acc.color}44` }}>{acc.ns}</span>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 20 }}>
                  <span style={{ color: theme.textDim, fontSize: 13 }}>{acc.email}</span>
                  <span style={{ color: theme.textDim, fontSize: 13, fontFamily: theme.mono, letterSpacing: 2 }}>token ••••••••</span>
                </div>
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <div style={{ padding: '6px 14px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontSize: 13, cursor: 'pointer' }}>Edit</div>
                <div style={{ padding: '6px 14px', borderRadius: 8, border: `1px solid ${theme.danger}44`, color: theme.danger, fontSize: 13, cursor: 'pointer' }}>Delete</div>
              </div>
            </div>
          );
        })}

        {/* Keychain note */}
        {showKeychain && (() => {
          const kS = spring({ frame: frame - 70, fps, config: { damping: 160 } });
          return (
            <div style={{
              opacity: kS,
              transform: `translateY(${interpolate(kS, [0, 1], [12, 0])}px)`,
              marginTop: 20,
              padding: '14px 20px',
              borderRadius: 12,
              background: `${theme.accent2}11`,
              border: `1px solid ${theme.accent2}33`,
              display: 'flex',
              alignItems: 'center',
              gap: 14,
            }}>
              <span style={{ fontSize: 18 }}>🔒</span>
              <span style={{ color: theme.accent2, fontSize: 14 }}>Tokens are encrypted and stored in the macOS Keychain — never written to disk.</span>
            </div>
          );
        })()}
      </div>
    </PanelFade>
  );
};

// ─── scene 4: Channels ────────────────────────────────────────────────────────
const ChannelsPanel: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const CHANNELS = [
    { name: 'Slack',    icon: '💬', desc: 'Post agent completions and PR reviews', color: '#4a154b', enabled: true },
    { name: 'Telegram', icon: '✈️',  desc: 'Mobile push for critical events',       color: '#2ca5e0', enabled: false },
  ];
  return (
    <PanelFade>
      <div style={{ padding: '36px 40px', height: '100%', boxSizing: 'border-box', overflow: 'hidden' }}>
        <div style={{ color: theme.text, fontSize: 22, fontWeight: 800, marginBottom: 28 }}>Channels</div>

        {CHANNELS.map((ch, i) => {
          const s = spring({ frame: frame - i * 12, fps, config: { damping: 200 } });
          const toggleS = spring({ frame: frame - i * 12 - 8, fps, config: { damping: 200 } });
          return (
            <div key={ch.name} style={{
              opacity: s,
              transform: `translateX(${interpolate(s, [0, 1], [20, 0])}px)`,
              background: theme.surface2,
              borderRadius: 14,
              border: `1px solid ${theme.border}`,
              padding: '22px 24px',
              marginBottom: 16,
            }}>
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 16 }}>
                <div style={{ width: 48, height: 48, borderRadius: 12, background: `${ch.color}33`, border: `1px solid ${ch.color}66`, display: 'grid', placeItems: 'center', fontSize: 24, flexShrink: 0 }}>
                  {ch.icon}
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
                    <div style={{ color: theme.text, fontSize: 17, fontWeight: 700 }}>{ch.name}</div>
                    {/* toggle */}
                    <div style={{
                      width: 52,
                      height: 28,
                      borderRadius: 14,
                      background: ch.enabled ? theme.accent : theme.border,
                      position: 'relative',
                      transition: 'background 0.3s',
                      flexShrink: 0,
                      boxShadow: ch.enabled ? `0 0 12px ${theme.accent}55` : 'none',
                    }}>
                      <div style={{
                        position: 'absolute',
                        top: 4,
                        left: ch.enabled ? 26 : 4,
                        width: 20,
                        height: 20,
                        borderRadius: '50%',
                        background: '#fff',
                        transition: 'left 0.3s',
                      }} />
                    </div>
                  </div>
                  <div style={{ color: theme.textDim, fontSize: 14, marginBottom: 14 }}>{ch.desc}</div>
                  {/* Seed from loom button */}
                  <div style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '8px 16px',
                    borderRadius: 8,
                    border: `1px solid ${theme.border}`,
                    color: theme.textDim,
                    fontSize: 13,
                    cursor: 'pointer',
                    background: theme.surface,
                  }}>
                    <span style={{ fontSize: 14 }}>🌱</span>
                    <span>Seed from loom</span>
                  </div>
                </div>
              </div>
            </div>
          );
        })}

        {/* Jira teaser card */}
        {frame > 60 && (() => {
          const jS = spring({ frame: frame - 60, fps, config: { damping: 180 } });
          return (
            <div style={{
              opacity: jS,
              transform: `translateY(${interpolate(jS, [0, 1], [14, 0])}px)`,
              background: theme.surface2,
              borderRadius: 14,
              border: `1px solid ${theme.border}`,
              padding: '22px 24px',
              display: 'flex',
              alignItems: 'center',
              gap: 16,
            }}>
              <div style={{ width: 48, height: 48, borderRadius: 12, background: '#0052cc22', border: '1px solid #0052cc66', display: 'grid', placeItems: 'center', fontSize: 24, flexShrink: 0 }}>
                📋
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ color: theme.text, fontSize: 17, fontWeight: 700, marginBottom: 4 }}>Jira</div>
                <div style={{ color: theme.textDim, fontSize: 14 }}>Link issues to branches · resolve on merge</div>
              </div>
              <div style={{ padding: '8px 18px', borderRadius: 8, background: '#0052cc', color: '#fff', fontSize: 13, fontWeight: 700 }}>Configure</div>
            </div>
          );
        })()}
      </div>
    </PanelFade>
  );
};

// ─── outro ────────────────────────────────────────────────────────────────────
const OutroScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // Final "make it yours" reveal
  const t1 = spring({ frame: frame - 20, fps, config: { damping: 160 } });
  const t2 = spring({ frame: frame - 44, fps, config: { damping: 160 } });
  const t3 = spring({ frame: frame - 62, fps, config: { damping: 160 } });

  return (
    <AbsoluteFill
      style={{
        background: theme.bgGradient,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 0,
      }}
    >
      {/* Glowing ring */}
      <div style={{
        opacity: t1,
        transform: `scale(${interpolate(t1, [0, 1], [0.6, 1])})`,
        width: 160,
        height: 160,
        borderRadius: 40,
        background: `linear-gradient(135deg, ${theme.accent} 0%, ${theme.accent2} 100%)`,
        boxShadow: `0 0 80px ${theme.accent}66, 0 0 160px ${theme.accent}33`,
        display: 'grid',
        placeItems: 'center',
        marginBottom: 36,
      }}>
        <span style={{ color: '#fff', fontSize: 72, fontWeight: 900, fontFamily: theme.font, letterSpacing: -4 }}>O</span>
      </div>

      <div style={{
        opacity: t2,
        transform: `translateY(${interpolate(t2, [0, 1], [24, 0])}px)`,
        color: theme.text,
        fontFamily: theme.font,
        fontSize: 76,
        fontWeight: 900,
        letterSpacing: -2,
        textAlign: 'center',
        lineHeight: 1.1,
        marginBottom: 20,
      }}>
        Make it yours.
      </div>

      <div style={{
        opacity: t3,
        transform: `translateY(${interpolate(t3, [0, 1], [16, 0])}px)`,
        color: theme.textDim,
        fontFamily: theme.font,
        fontSize: 28,
        textAlign: 'center',
        letterSpacing: 0.3,
      }}>
        Appearance · Providers · Git · Jira · Channels · Users
      </div>
    </AbsoluteFill>
  );
};

// ─── the full settings app UI ─────────────────────────────────────────────────
interface SettingsAppProps {
  activeNav: string;
  panel: React.ReactNode;
  cursorFrom?: [number, number];
  cursorTo?: [number, number];
  cursorStart?: number;
}

const SettingsApp: React.FC<SettingsAppProps> = ({ activeNav, panel, cursorFrom, cursorTo, cursorStart }) => (
  <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
    <OttoWindow title="Otto — Settings" sidebar={<SettingsNav active={activeNav} />}>
      <div style={{ height: '100%', overflow: 'hidden' }}>
        {panel}
      </div>
    </OttoWindow>
    {cursorFrom && cursorTo && cursorStart !== undefined && (
      <div style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}>
        <Cursor from={cursorFrom} to={cursorTo} startAt={cursorStart} duration={22} click />
      </div>
    )}
  </AbsoluteFill>
);

// ─── main export ──────────────────────────────────────────────────────────────
export const Settings: React.FC = () => {
  const frame = useCurrentFrame();

  // cross-fade between scenes (used for the app sequences)
  const panelFadeOut = (start: number, len: number) =>
    interpolate(frame, [start + len - 12, start + len], [1, 0], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' });

  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>
      {/* ── TITLE CARD (0–75) ── */}
      <Sequence from={0} durationInFrames={TITLE_DUR + 10}>
        <AbsoluteFill style={{
          background: theme.bgGradient,
          opacity: interpolate(frame, [TITLE_DUR - 12, TITLE_DUR], [1, 0], { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' }),
        }}>
          <TitleCard kicker="OTTO ADE" title="Settings" subtitle="Set up Otto your way" />
        </AbsoluteFill>
      </Sequence>

      {/* ── SCENE 1: Appearance (75–240) ── */}
      <Sequence from={APP_START} durationInFrames={APP_DUR + 10}>
        <AbsoluteFill style={{ opacity: panelFadeOut(APP_START, APP_DUR) }}>
          <SettingsApp
            activeNav="Appearance"
            panel={<AppearancePanel />}
            cursorFrom={[330, 440]}
            cursorTo={[148, 144]}
            cursorStart={8}
          />
          <Caption step={1} title="Appearance" sub="Pick a theme, accent colour, and light/dark mode" delay={22} />
        </AbsoluteFill>
      </Sequence>

      {/* ── SCENE 2: Providers (240–420) ── */}
      <Sequence from={PROV_START} durationInFrames={PROV_DUR + 10}>
        <AbsoluteFill style={{ opacity: panelFadeOut(PROV_START, PROV_DUR) }}>
          <SettingsApp
            activeNav="Providers"
            panel={<ProvidersPanel />}
            cursorFrom={[148, 144]}
            cursorTo={[148, 186]}
            cursorStart={8}
          />
          <Caption step={2} title="Providers" sub="Built-in claude / codex / agy — or add opencode, kilo, and more" delay={22} />
        </AbsoluteFill>
      </Sequence>

      {/* ── SCENE 3: Git Accounts (420–585) ── */}
      <Sequence from={GIT_START} durationInFrames={GIT_DUR + 10}>
        <AbsoluteFill style={{ opacity: panelFadeOut(GIT_START, GIT_DUR) }}>
          <SettingsApp
            activeNav="Git Accounts"
            panel={<GitAccountsPanel />}
            cursorFrom={[148, 186]}
            cursorTo={[148, 228]}
            cursorStart={8}
          />
          <Caption step={3} title="Git Accounts" sub="GitHub · Bitbucket · GitLab — tokens live in the macOS Keychain" delay={22} />
        </AbsoluteFill>
      </Sequence>

      {/* ── SCENE 4: Channels (585–735) ── */}
      <Sequence from={CHAN_START} durationInFrames={CHAN_DUR + 10}>
        <AbsoluteFill style={{ opacity: panelFadeOut(CHAN_START, CHAN_DUR) }}>
          <SettingsApp
            activeNav="Channels"
            panel={<ChannelsPanel />}
            cursorFrom={[148, 228]}
            cursorTo={[148, 312]}
            cursorStart={8}
          />
          <Caption step={4} title="Channels &amp; Jira" sub="Slack · Telegram · Jira — stay in the loop wherever you are" delay={22} />
        </AbsoluteFill>
      </Sequence>

      {/* ── OUTRO (735–1080) ── */}
      <Sequence from={OUTRO_START} durationInFrames={OUTRO_DUR}>
        <OutroScene />
      </Sequence>
    </AbsoluteFill>
  );
};
