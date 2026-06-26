import React from 'react';
import { AbsoluteFill } from 'remotion';
import { T, brand, fonts, radius, alpha } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  TitleCard,
  Caption,
  Card,
  Button,
  Chip,
  Toggle,
  Icon,
  Toast,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  CUSTOM PLUGINS — runtime out-of-process sidecars in any language, with a
//  scoped host API, reverse-proxied iframe UI, and slug-keyed RBAC.
// ════════════════════════════════════════════════════════════════════════════

const VIOLET = '#a78bfa';

// ── Shared diagram helpers ────────────────────────────────────────────────────

/** Glowing labelled box for the architecture diagram. */
const ArchBox: React.FC<{ label: string; sub: string; color: string }> = ({ label, sub, color }) => (
  <div
    style={{
      padding: '20px 28px',
      borderRadius: radius.l,
      background: alpha(color, 0.1),
      border: `1.5px solid ${alpha(color, 0.5)}`,
      boxShadow: `0 8px 32px ${alpha(color, 0.18)}`,
      textAlign: 'center',
      minWidth: 200,
    }}
  >
    <div style={{ fontFamily: fonts.mono, fontSize: 15, fontWeight: 700, color }}>{label}</div>
    <div style={{ fontFamily: fonts.mono, fontSize: 11.5, color: alpha(color, 0.65), marginTop: 5 }}>{sub}</div>
  </div>
);

/** Gradient arrow with a label above the line. */
const Connector: React.FC<{ label: string; fromColor: string; toColor: string }> = ({
  label,
  fromColor,
  toColor,
}) => (
  <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 5, width: 136 }}>
    <span
      style={{
        fontFamily: fonts.mono,
        fontSize: 11,
        color: alpha('#ffffff', 0.48),
        letterSpacing: 0.5,
        whiteSpace: 'nowrap',
      }}
    >
      {label}
    </span>
    <div style={{ display: 'flex', alignItems: 'center', width: '100%' }}>
      <div
        style={{
          flex: 1,
          height: 2,
          background: `linear-gradient(90deg, ${fromColor}, ${toColor})`,
        }}
      />
      <div
        style={{
          width: 0,
          height: 0,
          borderTop: '6px solid transparent',
          borderBottom: '6px solid transparent',
          borderLeft: `8px solid ${toColor}`,
        }}
      />
    </div>
  </div>
);

/** Mock iframe panel — a mini burndown chart as the plugin UI. */
const PluginIframe: React.FC = () => (
  <div
    style={{
      width: 260,
      background: T.bg,
      border: `1.5px solid ${VIOLET}`,
      borderRadius: radius.m,
      boxShadow: `0 12px 40px ${alpha(VIOLET, 0.22)}, 0 0 0 1px ${alpha(VIOLET, 0.14)}`,
      overflow: 'hidden',
    }}
  >
    <div
      style={{
        height: 33,
        display: 'flex',
        alignItems: 'center',
        gap: 7,
        padding: '0 10px',
        background: T.bgSidebar,
        borderBottom: `1px solid ${T.border}`,
      }}
    >
      <div
        style={{
          width: 7,
          height: 7,
          borderRadius: '50%',
          background: VIOLET,
          boxShadow: `0 0 7px ${VIOLET}`,
        }}
      />
      <span
        style={{
          flex: 1,
          fontFamily: fonts.ui,
          fontSize: 11.5,
          fontWeight: 600,
          color: T.text,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        jira-burndown
      </span>
      <Chip style={{ marginLeft: 'auto' }}>iframe</Chip>
    </div>
    <div style={{ padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div style={{ fontFamily: fonts.ui, fontSize: 11, color: T.textDim }}>Sprint 43 · 8 days remaining</div>
      <div style={{ display: 'flex', alignItems: 'flex-end', gap: 3, height: 56 }}>
        {[42, 36, 31, 28, 22, 18, 14, 10].map((v, i) => (
          <div
            key={i}
            style={{
              flex: 1,
              height: `${(v / 42) * 100}%`,
              borderRadius: 3,
              background:
                i < 4
                  ? `linear-gradient(180deg, #863bff, ${alpha('#863bff', 0.46)})`
                  : alpha('#863bff', 0.25),
            }}
          />
        ))}
      </div>
      <div style={{ fontFamily: fonts.mono, fontSize: 10, color: T.textDim }}>14 pts remaining · ideal: 10</div>
    </div>
  </div>
);

// ── Scene 1 — Title (~80 f) ───────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Custom Plugins"
    title="Extend Otto"
    subtitle="Out-of-process sidecar plugins in any language — install, enable, and remove at runtime."
  />
);

// ── Scene 2 — Install & manage plugins (~180 f) ───────────────────────────────

const LANG_COLOR: Record<string, string> = {
  node:   '#28c840',
  rust:   '#febc2e',
  python: '#0a84ff',
};

const PLUGINS: Array<{
  name: string;
  lang: string;
  ver: string;
  desc: string;
  enabled: boolean;
}> = [
  {
    name: 'jira-burndown',
    lang: 'node',
    ver: 'v1.4.2',
    desc: 'Burndown & velocity charts pulled live from Jira sprints',
    enabled: true,
  },
  {
    name: 'cost-alerts',
    lang: 'rust',
    ver: 'v0.9.0',
    desc: 'AWS cost-anomaly detection with configurable threshold webhooks',
    enabled: true,
  },
  {
    name: 'screenshot-diff',
    lang: 'python',
    ver: 'v2.1.0',
    desc: 'Visual regression diff for UI snapshot comparisons',
    enabled: false,
  },
];

const ManageScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={<Navigator active="settings" />}
        title="Otto — Settings · Plugins"
      >
        {/* positioning context so the Toast can be placed absolutely */}
        <div style={{ position: 'relative', height: '100%', overflow: 'hidden' }}>
          <div style={{ padding: '28px 32px' }}>
            {/* Page header */}
            <Appear delay={8} y={14}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'flex-start',
                  justifyContent: 'space-between',
                  marginBottom: 28,
                }}
              >
                <div>
                  <div
                    style={{
                      fontFamily: fonts.ui,
                      fontSize: 22,
                      fontWeight: 700,
                      color: T.text,
                      letterSpacing: -0.3,
                    }}
                  >
                    Plugins
                  </div>
                  <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginTop: 4 }}>
                    Out-of-process sidecar plugins — any language, no rebuild required
                  </div>
                </div>
                <Button variant="primary" icon="plus">
                  Install plugin
                </Button>
              </div>
            </Appear>

            {/* Section label */}
            <Appear delay={16} y={8}>
              <div
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 11.5,
                  fontWeight: 600,
                  color: T.textDim,
                  textTransform: 'uppercase',
                  letterSpacing: 0.8,
                  marginBottom: 12,
                }}
              >
                Installed · 3
              </div>
            </Appear>

            {/* Plugin rows */}
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {PLUGINS.map((p, i) => (
                <Appear key={p.name} delay={24 + i * 16} y={14}>
                  <Card pad={14} style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
                    {/* status glow dot */}
                    <div
                      style={{
                        width: 8,
                        height: 8,
                        borderRadius: '50%',
                        background: p.enabled ? '#28c840' : T.textDim,
                        flexShrink: 0,
                        boxShadow: p.enabled ? '0 0 7px #28c840' : 'none',
                      }}
                    />
                    {/* name + meta */}
                    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
                        <span
                          style={{
                            fontFamily: fonts.ui,
                            fontSize: 14,
                            fontWeight: 650 as never,
                            color: T.text,
                          }}
                        >
                          {p.name}
                        </span>
                        <Chip color={LANG_COLOR[p.lang]}>{p.lang}</Chip>
                        <Chip>{p.ver}</Chip>
                      </div>
                      <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>{p.desc}</div>
                    </div>
                    {/* controls */}
                    <Toggle on={p.enabled} />
                    <Button variant="ghost" size="s" icon="trash">
                      Remove
                    </Button>
                  </Card>
                </Appear>
              ))}
            </div>
          </div>

          {/* Toast — appears once the list is settled */}
          <Toast
            text="cost-alerts v0.9.0 · enabled · listening on :9201"
            tone="ok"
            delay={104}
            style={{ position: 'absolute', bottom: 28, right: 28 }}
          />
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Install out-of-process sidecar plugins — any language, no rebuild"
      sub="Enable, disable, or remove at runtime. The daemon handles the lifecycle."
    />
  </>
);

// ── Scene 3 — Supervision chain + reverse-proxied iframe panel (~150 f) ───────

const ArchScene: React.FC = () => (
  <>
    <AbsoluteFill
      style={{ alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 52 }}
    >
      <Appear delay={4} y={22}>
        <div
          style={{
            fontFamily: fonts.ui,
            fontSize: 46,
            fontWeight: 800,
            letterSpacing: -1.2,
            color: '#ffffff',
            textAlign: 'center',
          }}
        >
          Supervised. Proxied. Contained.
        </div>
      </Appear>

      {/* Architecture flow: daemon → plugin process → iframe panel */}
      <div style={{ display: 'flex', alignItems: 'center' }}>
        <Appear delay={18} y={14}>
          <ArchBox label="daemon" sub="ottod · :7700" color={brand.cyan} />
        </Appear>

        <Appear delay={32} y={0} x={-16}>
          <Connector label="supervises" fromColor={brand.cyan} toColor={brand.purple} />
        </Appear>

        <Appear delay={46} y={14}>
          <ArchBox label="plugin process" sub="node / rust / python" color={brand.purple} />
        </Appear>

        <Appear delay={60} y={0} x={-16}>
          <Connector label="reverse-proxy" fromColor={brand.purple} toColor={VIOLET} />
        </Appear>

        <Appear delay={74} y={14}>
          <PluginIframe />
        </Appear>
      </div>
    </AbsoluteFill>
    <Caption
      step={3}
      title="The daemon supervises each plugin & reverse-proxies its UI into a panel"
      sub="Plugins run in isolated processes — crash-safe, language-agnostic, zero rebuild"
    />
  </>
);

// ── Scene 4 — Scoped host API + slug-keyed RBAC (~120 f) ─────────────────────

const API_CAPS: Array<{ label: string; color: string }> = [
  { label: 'read:sessions',   color: brand.cyan   },
  { label: 'read:usage',      color: brand.cyan   },
  { label: 'webhook:inbound', color: brand.purple },
  { label: 'emit:toast',      color: VIOLET       },
  { label: 'write:none',      color: '#ff5f57'    },
];

const RBAC_ROWS: Array<{ slug: string; role: string; color: string }> = [
  { slug: 'plugin:jira-burndown',   role: 'View', color: brand.cyan   },
  { slug: 'plugin:cost-alerts',     role: 'Edit', color: brand.purple },
  { slug: 'plugin:screenshot-diff', role: 'None', color: '#ff5f57'    },
];

const RbacScene: React.FC = () => (
  <>
    <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ display: 'flex', gap: 64, alignItems: 'flex-start', width: 1360 }}>
        {/* ── Left: Scoped Host API ──────────────────────────────────────── */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 20 }}>
          <Appear delay={6} y={18}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 2 }}>
              <Icon name="plug" size={20} color={brand.cyan} />
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 22,
                  fontWeight: 700,
                  color: '#fff',
                  letterSpacing: -0.4,
                }}
              >
                Scoped Host API
              </span>
            </div>
            <div style={{ fontFamily: fonts.ui, fontSize: 14, color: alpha('#fff', 0.56), marginTop: 4 }}>
              Each plugin declares exactly what it needs — nothing more.
            </div>
          </Appear>

          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 10 }}>
            {API_CAPS.map((c, i) => (
              <Appear key={c.label} delay={18 + i * 9} y={10}>
                <div
                  style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    padding: '9px 16px',
                    borderRadius: radius.m,
                    background: alpha(c.color, 0.12),
                    border: `1px solid ${alpha(c.color, 0.45)}`,
                    fontFamily: fonts.mono,
                    fontSize: 14,
                    fontWeight: 600,
                    color: c.color,
                    whiteSpace: 'nowrap',
                  }}
                >
                  {c.label}
                </div>
              </Appear>
            ))}
          </div>
        </div>

        {/* Divider */}
        <Appear delay={12} y={0}>
          <div
            style={{
              width: 1,
              height: 240,
              background: alpha('#ffffff', 0.12),
              marginTop: 4,
              flexShrink: 0,
            }}
          />
        </Appear>

        {/* ── Right: Slug-keyed RBAC ─────────────────────────────────────── */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 20 }}>
          <Appear delay={10} y={18}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 2 }}>
              <Icon name="key" size={20} color={brand.purple} />
              <span
                style={{
                  fontFamily: fonts.ui,
                  fontSize: 22,
                  fontWeight: 700,
                  color: '#fff',
                  letterSpacing: -0.4,
                }}
              >
                Slug-keyed RBAC
              </span>
            </div>
            <div style={{ fontFamily: fonts.ui, fontSize: 14, color: alpha('#fff', 0.56), marginTop: 4 }}>
              Every plugin action is gated by a per-slug role grant.
            </div>
          </Appear>

          <div style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
            {RBAC_ROWS.map((r, i) => (
              <Appear key={r.slug} delay={28 + i * 12} y={10}>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '11px 14px',
                    borderRadius: radius.s,
                    background: T.surface,
                    border: `1px solid ${T.border}`,
                    gap: 16,
                  }}
                >
                  <span
                    style={{
                      fontFamily: fonts.mono,
                      fontSize: 13,
                      color: T.textDim,
                      flex: 1,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {r.slug}
                  </span>
                  <Chip color={r.color}>{r.role}</Chip>
                </div>
              </Appear>
            ))}
          </div>
        </div>
      </div>
    </AbsoluteFill>
    <Caption
      step={4}
      title="A scoped host API · slug-keyed RBAC per plugin"
      sub="Plugins can only access what you explicitly grant — least-privilege by default."
    />
  </>
);

// ── Composition ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 80,  node: <TitleScene />,  name: 'Title'  },
  { dur: 180, node: <ManageScene />, name: 'Manage' },
  { dur: 150, node: <ArchScene />,   name: 'Arch'   },
  { dur: 120, node: <RbacScene />,   name: 'RBAC'   },
  {
    dur: 130,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Custom Plugins"
        tagline="Make Otto yours — extend it at runtime, in any language"
        pills={[
          { label: 'Any language',      icon: 'box'  },
          { label: 'No rebuild',        icon: 'zap'  },
          { label: 'Sandboxed sidecar', icon: 'plug' },
          { label: 'Scoped RBAC',       icon: 'key'  },
        ]}
      />
    ),
  },
];

export const pluginsDuration = scenesDuration(SCENES);
export const Plugins: React.FC = () => <Scenes scenes={SCENES} />;
