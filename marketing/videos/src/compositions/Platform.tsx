import React from 'react';
import { T, brand, fonts, radius, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator, NavSession } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Keys,
  Toggle,
  Segmented,
  Toast,
  Icon,
  Terminal,
  TermLine,
  navActive,
} from '../components/kit';

// ── Shared helpers ────────────────────────────────────────────────────────────

const SectionHead: React.FC<{ label: string; delay?: number }> = ({ label, delay = 0 }) => (
  <Appear delay={delay} y={8}>
    <div
      style={{
        fontFamily: fonts.ui,
        fontSize: 11,
        fontWeight: 600,
        letterSpacing: 1.2,
        textTransform: 'uppercase',
        color: T.textDim,
        marginBottom: 6,
        marginTop: 16,
      }}
    >
      {label}
    </div>
  </Appear>
);

const SettingRow: React.FC<{
  label: string;
  sub?: string;
  delay?: number;
  children: React.ReactNode;
}> = ({ label, sub, delay = 0, children }) => (
  <Appear delay={delay} y={8}>
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '10px 0',
        borderBottom: `1px solid ${alpha(T.border, 0.6)}`,
      }}
    >
      <div style={{ flex: 1 }}>
        <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.text }}>{label}</div>
        {sub && (
          <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim, marginTop: 2 }}>
            {sub}
          </div>
        )}
      </div>
      {children}
    </div>
  </Appear>
);

// ── Scene 1 — Title card (~80f) ───────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Platform & Shortcuts"
    title="Platform"
    subtitle="The native layer that ties everything together"
  />
);

// ── Scene 2 — Command Palette (~160f) ─────────────────────────────────────────

const paletteSessions: NavSession[] = [
  { title: 'fix(auth): token expiry',  provider: 'claude', status: 'working', tasks: [3, 6] },
  { title: 'migrate postgres schema',  provider: 'codex',  status: 'idle',    tasks: [4, 4] },
  { title: 'add rate-limit headers',   provider: 'claude', status: 'idle',    tasks: [2, 3] },
];

const PALETTE_RESULTS: { icon: string; label: string; hint: string; active?: boolean }[] = [
  { icon: 'terminal', label: 'Open session…',       hint: 'Agents',  active: true },
  { icon: 'branch',   label: 'Go to Git',           hint: 'Git'                   },
  { icon: 'eye',      label: 'New code review…',    hint: 'Review'                },
  { icon: 'grid',     label: 'Walkthrough: Swarm',  hint: 'Help'                  },
];

const PaletteBgContent: React.FC = () => (
  <div
    style={{
      position: 'absolute',
      top: 0,
      left: 0,
      right: 0,
      bottom: 0,
      display: 'flex',
      gap: 12,
      padding: 16,
      boxSizing: 'border-box',
      opacity: 0.38,
      filter: 'blur(1.5px)',
    }}
  >
    <div style={{ flex: 1, background: T.termBg, borderRadius: 8, padding: 14 }}>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: '#28c840', marginBottom: 6 }}>
        $ go test ./internal/auth/...
      </div>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim }}>
        {'  → reading: middleware.go, jwt.go'}
      </div>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: '#ff5f57', marginTop: 4 }}>
        {'  FAIL TestTokenValidation'}
      </div>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.accent, marginTop: 4 }}>
        {'  ↳ applying patch…'}
      </div>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: '#28c840', marginTop: 4 }}>
        {'  ✓ PASS — 142 tests (3.2s)'}
      </div>
    </div>
    <div style={{ flex: 1, background: T.termBg, borderRadius: 8, padding: 14 }}>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: '#28c840', marginBottom: 6 }}>
        $ codex run migrate.md
      </div>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim }}>
        {'  reading schema.sql, types.ts'}
      </div>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.text, marginTop: 4 }}>
        {'  writing migration_0042.sql'}
      </div>
      <div style={{ fontFamily: fonts.mono, fontSize: 12.5, color: '#28c840', marginTop: 4 }}>
        {'  ✓ build ok · vet clean'}
      </div>
    </div>
  </div>
);

const PaletteOverlay: React.FC = () => (
  <Appear
    delay={10}
    y={-10}
    scale={0.97}
    style={{ position: 'absolute', top: 0, left: 0, right: 0, bottom: 0 }}
  >
    {/* dark backdrop */}
    <div
      style={{
        position: 'absolute',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        background: alpha('#000', 0.52),
      }}
    />
    {/* palette card centered */}
    <div
      style={{
        position: 'absolute',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: 76,
      }}
    >
      <div
        style={{
          width: 544,
          background: T.surface,
          border: `1px solid ${T.border}`,
          borderRadius: radius.l,
          boxShadow: `0 32px 80px rgba(0,0,0,0.65), 0 0 0 1px ${alpha('#fff', 0.04)}`,
          overflow: 'hidden',
        }}
      >
        {/* search row */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '11px 14px',
            borderBottom: `1px solid ${T.border}`,
            background: T.surface2,
          }}
        >
          <Icon name="search" size={15} color={T.textDim} />
          <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 14.5, color: T.text }}>
            Open session…
          </span>
          <span
            style={{
              fontFamily: fonts.mono,
              fontSize: 11.5,
              color: T.textDim,
              background: T.bg,
              padding: '2px 7px',
              borderRadius: 5,
              border: `1px solid ${T.border}`,
            }}
          >
            ⌘K
          </span>
        </div>

        {/* section label */}
        <div
          style={{
            padding: '9px 14px 3px',
            fontFamily: fonts.ui,
            fontSize: 11,
            fontWeight: 600,
            letterSpacing: 0.9,
            textTransform: 'uppercase',
            color: T.textDim,
          }}
        >
          Quick actions
        </div>

        {/* result rows */}
        <div style={{ paddingBottom: 5 }}>
          {PALETTE_RESULTS.map((r, i) => (
            <Appear key={r.label} delay={18 + i * 7} y={6}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  height: 35,
                  padding: '0 12px',
                  margin: '1px 5px',
                  borderRadius: radius.s,
                  background: r.active ? navActive.bg : 'transparent',
                  color: r.active ? navActive.fg : T.text,
                }}
              >
                <Icon
                  name={r.icon}
                  size={14}
                  color={r.active ? navActive.fg : T.textDim}
                />
                <span
                  style={{
                    flex: 1,
                    fontFamily: fonts.ui,
                    fontSize: 13.5,
                    fontWeight: r.active ? 600 : 400,
                  }}
                >
                  {r.label}
                </span>
                <span
                  style={{
                    fontFamily: fonts.ui,
                    fontSize: 12,
                    color: r.active ? alpha(navActive.fg, 0.65) : T.textDim,
                  }}
                >
                  {r.hint}
                </span>
                {r.active && (
                  <span
                    style={{
                      fontFamily: fonts.mono,
                      fontSize: 11,
                      color: alpha(navActive.fg, 0.7),
                      background: alpha(navActive.fg, 0.16),
                      padding: '1px 6px',
                      borderRadius: 4,
                    }}
                  >
                    ↵
                  </span>
                )}
              </div>
            </Appear>
          ))}
        </div>

        {/* footer: ⌘I / ⌘F / ⌘T hints */}
        <div
          style={{
            display: 'flex',
            gap: 20,
            padding: '8px 14px',
            borderTop: `1px solid ${T.border}`,
            background: T.surface2,
          }}
        >
          {[['⌘I', 'Ask Otto'], ['⌘F', 'find'], ['⌘T', 'jump session']].map(([key, label]) => (
            <div key={key} style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
              <span
                style={{
                  fontFamily: fonts.mono,
                  fontSize: 11.5,
                  color: T.textDim,
                  background: T.bg,
                  padding: '2px 7px',
                  borderRadius: 4,
                  border: `1px solid ${T.border}`,
                }}
              >
                {key}
              </span>
              <span style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim }}>
                {label}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  </Appear>
);

const PaletteScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow
        nav={
          <Navigator
            active="agents"
            sessions={paletteSessions}
            activeSessionTitle="fix(auth): token expiry"
            workingCount={1}
          />
        }
        tabs={[
          { label: 'fix(auth): token expiry', icon: 'terminal', active: true, dot: 'working' },
          { label: 'migrate postgres schema',  icon: 'terminal' },
          { label: 'add rate-limit headers',   icon: 'terminal' },
        ]}
        title="Otto — sinatra-go"
      >
        <PaletteBgContent />
        <PaletteOverlay />
      </OttoWindow>
    </Stage>

    {/* ⌘K callout floating over the cinematic bg */}
    <Appear delay={34} y={-10} style={{ position: 'absolute', top: 68, right: 88 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
        <Keys keys={['⌘', 'K']} />
        <span
          style={{
            fontFamily: fonts.ui,
            fontSize: 20,
            color: alpha('#fff', 0.78),
            fontWeight: 500,
          }}
        >
          open from anywhere
        </span>
      </div>
    </Appear>

    <Caption
      step={1}
      title="⌘K runs anything · ⌘I Ask Otto · ⌘F find · ⌘T jump"
      sub="One palette to launch sessions, navigate, search, and run any command"
    />
  </>
);

// ── Scene 3 — Theming + Customizable Sidebar (~160f) ──────────────────────────

const THEME_PREVIEWS: { name: string; bg: string; text: string; selected: boolean }[] = [
  { name: 'Native',   bg: '#1e1e23', text: '#f2f2f5', selected: true  },
  { name: 'Pro Dark', bg: '#16161c', text: '#e8e8ee', selected: false },
  { name: 'Warm',     bg: '#faf9f7', text: '#3d3a35', selected: false },
];

const SIDEBAR_MODULES: { icon: string; label: string; visible: boolean }[] = [
  { icon: 'terminal', label: 'Agents',          visible: true  },
  { icon: 'branch',   label: 'Git',             visible: true  },
  { icon: 'grid',     label: 'Swarm',           visible: true  },
  { icon: 'db',       label: 'Database',        visible: true  },
  { icon: 'box',      label: 'Message Brokers', visible: false },
  { icon: 'globe',    label: 'Vault',           visible: true  },
  { icon: 'chart',    label: 'Usage',           visible: false },
];

const ThemingScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow nav={<Navigator active="settings" />} title="Otto — Settings · Appearance">
        <div style={{ display: 'flex', height: '100%' }}>
          {/* ── Left: Appearance ── */}
          <div
            style={{
              flex: 1,
              padding: '22px 26px',
              borderRight: `1px solid ${T.border}`,
              overflow: 'hidden',
            }}
          >
            <Appear delay={8} y={14}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 17,
                  fontWeight: 700,
                  color: T.text,
                  marginBottom: 2,
                }}
              >
                Appearance
              </div>
              <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginBottom: 4 }}>
                Customize how Otto looks and feels.
              </div>
            </Appear>

            <SectionHead label="Theme" delay={18} />

            <Appear delay={22} y={8}>
              <Segmented options={['Native', 'Pro Dark', 'Warm']} active={0} />
              {/* theme preview tiles */}
              <div style={{ display: 'flex', gap: 9, marginTop: 12 }}>
                {THEME_PREVIEWS.map((th) => (
                  <div
                    key={th.name}
                    style={{
                      flex: 1,
                      height: 50,
                      borderRadius: 9,
                      background: th.bg,
                      border: `2px solid ${th.selected ? brand.cyan : alpha('#fff', 0.1)}`,
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      gap: 6,
                      boxShadow: th.selected
                        ? `0 0 14px ${alpha(brand.cyan, 0.35)}`
                        : 'none',
                    }}
                  >
                    <div
                      style={{
                        width: 7,
                        height: 7,
                        borderRadius: '50%',
                        background: th.text,
                        opacity: 0.65,
                      }}
                    />
                    <span
                      style={{
                        fontFamily: fonts.ui,
                        fontSize: 12,
                        color: th.text,
                        opacity: 0.75,
                        fontWeight: 500,
                      }}
                    >
                      {th.name}
                    </span>
                  </div>
                ))}
              </div>
            </Appear>

            <SectionHead label="Color scheme" delay={34} />

            <SettingRow label="Light / Dark mode" delay={38}>
              <Segmented options={['Auto', 'Light', 'Dark']} active={2} />
            </SettingRow>

            <SettingRow
              label="RTL layout"
              sub="Mirror the interface for right-to-left languages"
              delay={46}
            >
              <Toggle on={false} />
            </SettingRow>
          </div>

          {/* ── Right: Sidebar ── */}
          <div
            style={{
              width: 316,
              flexShrink: 0,
              padding: '22px 20px',
              overflow: 'hidden',
            }}
          >
            <Appear delay={14} y={14}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 17,
                  fontWeight: 700,
                  color: T.text,
                  marginBottom: 2,
                }}
              >
                Sidebar
              </div>
              <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginBottom: 14 }}>
                Drag to reorder · toggle to hide.
              </div>
            </Appear>

            {SIDEBAR_MODULES.map((m, i) => (
              <Appear key={m.label} delay={22 + i * 8} y={7}>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 9,
                    height: 34,
                    padding: '0 6px',
                    borderRadius: radius.s,
                    background: i === 0 ? alpha(T.accent, 0.07) : 'transparent',
                    marginBottom: 2,
                    opacity: m.visible ? 1 : 0.44,
                  }}
                >
                  <span
                    style={{
                      fontFamily: fonts.mono,
                      fontSize: 14,
                      color: T.textDim,
                      cursor: 'grab',
                      userSelect: 'none',
                      lineHeight: 1,
                    }}
                  >
                    ⠿
                  </span>
                  <Icon name={m.icon} size={13} color={T.textDim} />
                  <span
                    style={{ flex: 1, fontFamily: fonts.ui, fontSize: 12.5, color: T.text }}
                  >
                    {m.label}
                  </span>
                  <Toggle on={m.visible} />
                </div>
              </Appear>
            ))}
          </div>
        </div>
      </OttoWindow>
    </Stage>

    <Caption
      step={2}
      title="Make it yours"
      sub="Themes, light/dark, RTL, a sidebar you reorder and trim"
    />
  </>
);

// ── Scene 4 — Daily CLI Auto-update (~110f) ────────────────────────────────────

const updateLines: TermLine[] = [
  { text: '[03:00 UTC] checking for CLI updates…',          tone: 'dim'    },
  { text: '  claude:  v1.0.8 → v1.1.2',                    tone: 'text'   },
  { text: '  codex:   v0.9.3 → v0.9.5',                    tone: 'text'   },
  { text: '  downloading claude@1.1.2…',                    tone: 'dim'    },
  { text: '  downloading codex@0.9.5…',                     tone: 'dim'    },
  { text: '  ✓ claude updated',                             tone: 'ok'     },
  { text: '  ✓ codex updated',                              tone: 'ok'     },
  { text: '  reloading 2 resumed session(s)…',              tone: 'accent' },
  { text: '  ✓ done · next update: 03:00 UTC tomorrow',     tone: 'ok'     },
];

const AutoUpdateScene: React.FC = () => (
  <>
    <Stage scale={0.87}>
      <OttoWindow nav={<Navigator active="settings" />} title="Otto — Settings · Updates">
        <div
          style={{
            padding: '24px 28px',
            height: '100%',
            boxSizing: 'border-box',
            position: 'relative',
            overflow: 'hidden',
          }}
        >
          <Appear delay={6} y={14}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 17,
                fontWeight: 700,
                color: T.text,
                marginBottom: 2,
              }}
            >
              Updates
            </div>
            <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginBottom: 20 }}>
              Otto automatically keeps your agent CLIs up to date.
            </div>
          </Appear>

          {/* setting card */}
          <Appear delay={16} y={10}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 14,
                padding: '14px 16px',
                background: T.surface,
                border: `1px solid ${T.border}`,
                borderRadius: radius.m,
                marginBottom: 20,
              }}
            >
              <Icon name="refresh" size={18} color={brand.cyan} />
              <div style={{ flex: 1 }}>
                <div
                  style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 600, color: T.text }}
                >
                  Daily CLI auto-update
                </div>
                <div
                  style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim, marginTop: 3 }}
                >
                  Update claude, codex and other CLIs at a fixed time each day
                </div>
              </div>
              <Toggle on />
              <span
                style={{
                  fontFamily: fonts.mono,
                  fontSize: 12,
                  color: T.textDim,
                  background: T.surface2,
                  padding: '3px 10px',
                  borderRadius: 5,
                  border: `1px solid ${T.border}`,
                  whiteSpace: 'nowrap',
                }}
              >
                03:00 UTC
              </span>
            </div>
          </Appear>

          {/* last-run log */}
          <Appear delay={24} y={8}>
            <div
              style={{
                fontFamily: fonts.ui,
                fontSize: 11.5,
                fontWeight: 600,
                letterSpacing: 0.8,
                textTransform: 'uppercase',
                color: T.textDim,
                marginBottom: 9,
              }}
            >
              Last run log
            </div>
            <Terminal
              lines={updateLines}
              delay={30}
              step={7}
              fontSize={13}
              style={{ maxHeight: 280 }}
            />
          </Appear>

          {/* toast notification */}
          <Toast
            text="claude & codex updated · 2 sessions reloaded"
            tone="ok"
            delay={68}
            style={{ position: 'absolute', top: 14, right: 14 }}
          />
        </div>
      </OttoWindow>
    </Stage>

    <Caption
      step={3}
      title="Agent CLIs auto-update daily"
      sub="Resumed sessions reload cleanly — no manual maintenance needed"
    />
  </>
);

// ── Scene 5 — Outro (~110f) ───────────────────────────────────────────────────

const OutroScene: React.FC = () => (
  <WalkOutro
    title="Platform"
    tagline="A native macOS app that gets out of your way"
    pills={[
      { label: '⌘K palette',          icon: 'command', color: brand.cyan   },
      { label: 'Themes + RTL',         icon: 'gear',    color: brand.purple },
      { label: 'Customizable sidebar', icon: 'sidebar', color: brand.violet },
      { label: 'CLI auto-update',      icon: 'refresh', color: '#28c840'   },
    ]}
  />
);

// ── Composition ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,      name: 'Title'      },
  { dur: 160, node: <PaletteScene />,    name: 'Palette'    },
  { dur: 160, node: <ThemingScene />,    name: 'Theming'    },
  { dur: 110, node: <AutoUpdateScene />, name: 'AutoUpdate' },
  { dur: 110, node: <OutroScene />,      name: 'Outro'      },
];

export const platformDuration = scenesDuration(SCENES);
export const Platform: React.FC = () => <Scenes scenes={SCENES} />;
