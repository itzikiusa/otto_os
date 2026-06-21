import React from 'react';
import { AbsoluteFill, useCurrentFrame } from 'remotion';
import { T, brand, fonts, radius, alpha, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Stagger,
  TitleCard,
  Caption,
  Card,
  Chip,
  Button,
  Field,
  Toggle,
  StatusDot,
  MetricStat,
  BarChart,
  Icon,
  track,
} from '../components/kit';

// ════════════════════════════════════════════════════════════════════════════
//  CUSTOM PLUGINS — runtime out-of-process sidecars with their own UI + scoped
//  host API. Install from a GitHub URL or local path; enable / disable / remove.
// ════════════════════════════════════════════════════════════════════════════

const PLUGIN_VIOLET = '#a78bfa';

// ── Scene 1 — title card ─────────────────────────────────────────────────────
const Title: React.FC = () => (
  <TitleCard
    kicker="CUSTOM PLUGINS"
    title="Extend Otto — no rebuild"
    subtitle="Drop-in sidecars with their own UI and a scoped host API"
  />
);

// ── Scene 2 — Settings → Plugins: install & manage at runtime ────────────────
const InstalledRow: React.FC<{
  name: string;
  slug: string;
  version: string;
  port: string;
  kind: string;
  kindColor: string;
  on?: boolean;
}> = ({ name, slug, version, port, kind, kindColor, on = true }) => (
  <Card
    pad={13}
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 14,
      background: T.surface2,
    }}
  >
    <div
      style={{
        width: 38,
        height: 38,
        borderRadius: radius.m,
        background: alpha(kindColor, 0.16),
        border: `1px solid ${alpha(kindColor, 0.4)}`,
        display: 'grid',
        placeItems: 'center',
        flexShrink: 0,
      }}
    >
      <Icon name="zap" size={19} color={kindColor} />
    </div>
    <div style={{ flex: 1, minWidth: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
        <span style={{ fontFamily: fonts.ui, fontSize: 16, fontWeight: 650 as never, color: T.text }}>{name}</span>
        <Chip color={kindColor}>{kind}</Chip>
        <span style={{ fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim }}>v{version}</span>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 6 }}>
        <span style={{ fontFamily: fonts.mono, fontSize: 11.5, color: T.textDim }}>#/plugin/{slug}</span>
        {on && (
          <span
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              fontFamily: fonts.mono,
              fontSize: 11.5,
              color: status.working,
            }}
          >
            <StatusDot kind="working" size={7} />
            running · {port}
          </span>
        )}
      </div>
    </div>
    <Toggle on={on} />
    <Button variant="ghost" size="s" icon="trash">
      Remove
    </Button>
  </Card>
);

const ManageScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="settings" />}
          title="Otto — Settings"
        >
          <div style={{ padding: '24px 30px', height: '100%', boxSizing: 'border-box', overflow: 'hidden' }}>
            {/* page heading */}
            <Appear delay={6} y={14}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 11, marginBottom: 4 }}>
                <Icon name="gear" size={18} color={T.textDim} />
                <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>Settings</span>
                <Icon name="chevronRight" size={13} color={T.textDim} />
                <span style={{ fontFamily: fonts.ui, fontSize: 22, fontWeight: 750 as never, color: T.text }}>
                  Plugins
                </span>
              </div>
            </Appear>
            <Appear delay={10} y={10}>
              <div style={{ fontFamily: fonts.ui, fontSize: 14, color: T.textDim, marginBottom: 18 }}>
                Out-of-process sidecars, reverse-proxied through the daemon — installed and toggled at runtime.
              </div>
            </Appear>

            {/* install field */}
            <Appear delay={16} y={14}>
              <Card pad={16} style={{ marginBottom: 18 }}>
                <div style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 600, color: T.text, marginBottom: 11 }}>
                  Install plugin
                </div>
                <div style={{ display: 'flex', alignItems: 'flex-end', gap: 12 }}>
                  <Field
                    label="GitHub URL or local path"
                    value="github.com/acme/dora-metrics"
                    icon="globe"
                    mono
                    focused
                    caret
                    style={{ flex: 1 }}
                  />
                  <Button variant="primary" icon="plus" style={{ height: 32 }}>
                    Install
                  </Button>
                </div>
              </Card>
            </Appear>

            {/* installed list */}
            <Appear delay={22} y={12}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  marginBottom: 10,
                }}
              >
                <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 600, color: T.text }}>
                  Installed
                </span>
                <span
                  style={{
                    display: 'inline-flex',
                    alignItems: 'center',
                    gap: 6,
                    fontFamily: fonts.mono,
                    fontSize: 11.5,
                    color: T.textDim,
                  }}
                >
                  <Icon name="link" size={12} color={T.textDim} />
                  reverse-proxied
                </span>
              </div>
            </Appear>

            <Stagger delay={28} step={8} y={16} style={{ display: 'flex', flexDirection: 'column', gap: 11 }}>
              <InstalledRow
                name="team-performance"
                slug="team-performance"
                version="1.2.0"
                port=":49217"
                kind="Node"
                kindColor="#9ee039"
              />
              <InstalledRow
                name="dora-metrics"
                slug="dora-metrics"
                version="0.4.1"
                port=":49183"
                kind="Rust"
                kindColor="#ff8a65"
              />
            </Stagger>

            {/* footnote */}
            <Appear delay={48} y={10}>
              <div
                style={{
                  marginTop: 16,
                  fontFamily: fonts.ui,
                  fontSize: 12,
                  color: alpha(T.textDim, 0.9),
                  opacity: track(frame, [48, 58], [0, 1]),
                }}
              >
                Each plugin runs as a child process on a loopback port — no app rebuild, no restart.
              </div>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={1}
        title="Install at runtime — enable, disable, remove"
        sub="Node or Rust sidecars · no app rebuild"
      />
    </>
  );
};

// ── Scene 3 — a plugin's iframe UI rendered in the sidebar ───────────────────
const PluginUIScene: React.FC = () => {
  const frame = useCurrentFrame();
  const grow = track(frame, [30, 70], [0, 1]);
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="walkthroughs" />}
          tabs={[{ label: 'DORA Metrics', icon: 'zap', active: true }]}
          title="Otto — Plugin · dora-metrics"
        >
          <div style={{ padding: '22px 30px', height: '100%', boxSizing: 'border-box', overflow: 'hidden' }}>
            {/* iframe chrome bar — routed under #/plugin/dora-metrics */}
            <Appear delay={6} y={12}>
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  height: 32,
                  padding: '0 12px',
                  borderRadius: `${radius.m}px ${radius.m}px 0 0`,
                  background: T.surface2,
                  border: `1px solid ${T.border}`,
                  borderBottom: 'none',
                }}
              >
                <Icon name="zap" size={14} color={PLUGIN_VIOLET} />
                <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600, color: T.text }}>
                  DORA Metrics
                </span>
                <Chip color={PLUGIN_VIOLET}>plugin</Chip>
                <span style={{ flex: 1 }} />
                <span style={{ fontFamily: fonts.mono, fontSize: 11.5, color: T.textDim }}>#/plugin/dora-metrics</span>
                <Icon name="external" size={12} color={T.textDim} />
              </div>
            </Appear>

            {/* the iframe body */}
            <Appear delay={10} y={12}>
              <div
                style={{
                  borderRadius: `0 0 ${radius.m}px ${radius.m}px`,
                  border: `1px solid ${T.border}`,
                  background: T.termBg,
                  padding: 22,
                  boxSizing: 'border-box',
                }}
              >
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, marginBottom: 18 }}>
                  <span style={{ fontFamily: fonts.ui, fontSize: 20, fontWeight: 750 as never, color: T.text }}>
                    DORA · last 30 days
                  </span>
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>
                    sinatra-users-go · main
                  </span>
                  <Chip tone="ok" style={{ marginLeft: 'auto' }}>
                    Elite
                  </Chip>
                </div>

                {/* metric row */}
                <Stagger delay={16} step={6} y={16} style={{ display: 'flex', gap: 14, marginBottom: 18 }}>
                  <MetricStat
                    label="Deploy frequency"
                    value="4.2 / day"
                    delta="▲ 18% vs prev"
                    deltaTone="ok"
                    accent={PLUGIN_VIOLET}
                    style={{ flex: 1 }}
                  />
                  <MetricStat
                    label="Lead time"
                    value="6.1 h"
                    delta="▼ 22% faster"
                    deltaTone="ok"
                    style={{ flex: 1 }}
                  />
                  <MetricStat
                    label="Change-fail %"
                    value="3.4%"
                    delta="▼ 1.2 pts"
                    deltaTone="ok"
                    style={{ flex: 1 }}
                  />
                  <MetricStat
                    label="MTTR"
                    value="41 min"
                    delta="▲ 5 min"
                    deltaTone="bad"
                    style={{ flex: 1 }}
                  />
                </Stagger>

                {/* deploy frequency bar chart */}
                <Card pad={16} style={{ background: T.surface }}>
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      marginBottom: 12,
                    }}
                  >
                    <span style={{ fontFamily: fonts.ui, fontSize: 13.5, fontWeight: 600, color: T.text }}>
                      Deploys per week
                    </span>
                    <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>last 8 weeks</span>
                  </div>
                  <BarChart
                    data={[14, 18, 16, 22, 19, 26, 24, 30]}
                    labels={['w1', 'w2', 'w3', 'w4', 'w5', 'w6', 'w7', 'w8']}
                    color={PLUGIN_VIOLET}
                    height={150}
                    grow={grow}
                  />
                </Card>
              </div>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption
        step={2}
        title="Plugins render right in the sidebar"
        sub="A scoped host API: read repos, spawn agents — role-gated"
      />
    </>
  );
};

// ── Scene 4 — host API + RBAC diagram ────────────────────────────────────────
const NodeBox: React.FC<{
  icon: string;
  label: string;
  color: string;
  sub?: string;
  delay?: number;
}> = ({ icon, label, color, sub, delay = 0 }) => (
  <Appear delay={delay} y={18} scale={0.92}>
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '18px 24px',
        borderRadius: radius.l,
        background: alpha(color, 0.1),
        border: `1px solid ${alpha(color, 0.42)}`,
        boxShadow: `0 10px 40px ${alpha(color, 0.18)}`,
        minWidth: 240,
      }}
    >
      <div
        style={{
          width: 46,
          height: 46,
          borderRadius: radius.m,
          background: alpha(color, 0.2),
          display: 'grid',
          placeItems: 'center',
          flexShrink: 0,
        }}
      >
        <Icon name={icon} size={24} color={color} />
      </div>
      <div>
        <div style={{ fontFamily: fonts.ui, fontSize: 19, fontWeight: 700, color: '#fff' }}>{label}</div>
        {sub && <div style={{ fontFamily: fonts.mono, fontSize: 13, color: alpha('#fff', 0.6), marginTop: 3 }}>{sub}</div>}
      </div>
    </div>
  </Appear>
);

const ScopeChip: React.FC<{ icon: string; label: string; color: string; delay: number }> = ({
  icon,
  label,
  color,
  delay,
}) => (
  <Appear delay={delay} y={10} scale={0.9}>
    <div
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 8,
        padding: '8px 15px',
        borderRadius: 999,
        background: alpha(color, 0.12),
        border: `1px solid ${alpha(color, 0.4)}`,
        fontFamily: fonts.mono,
        fontSize: 15,
        fontWeight: 600,
        color: '#fff',
      }}
    >
      <Icon name={icon} size={15} color={color} />
      {label}
    </div>
  </Appear>
);

const HostApiScene: React.FC = () => {
  const frame = useCurrentFrame();
  const flow = track(frame, [24, 50], [0, 1]);
  return (
    <>
      <AbsoluteFill style={{ alignItems: 'center', justifyContent: 'center', padding: '0 120px' }}>
        <Appear delay={4} y={14}>
          <div
            style={{
              fontFamily: fonts.ui,
              fontSize: 40,
              fontWeight: 800,
              letterSpacing: -1,
              color: '#fff',
              textAlign: 'center',
              marginBottom: 50,
            }}
          >
            Sandboxed by design —{' '}
            <span
              style={{
                backgroundImage: brand.gradSoft,
                WebkitBackgroundClip: 'text',
                backgroundClip: 'text',
                color: 'transparent',
                WebkitTextFillColor: 'transparent',
              }}
            >
              scoped & RBAC-gated
            </span>
          </div>
        </Appear>

        {/* plugin → host API flow */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 0 }}>
          <NodeBox icon="zap" label="Plugin sidecar" color={PLUGIN_VIOLET} sub="dora-metrics" delay={10} />
          {/* connector */}
          <div style={{ position: 'relative', width: 130, height: 4, margin: '0 4px' }}>
            <div
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                height: 4,
                width: `${flow * 100}%`,
                borderRadius: 999,
                background: `linear-gradient(90deg, ${PLUGIN_VIOLET}, ${brand.cyan})`,
                boxShadow: `0 0 12px ${alpha(brand.cyan, 0.6)}`,
              }}
            />
            <Appear delay={26} scale={0.7}>
              <div
                style={{
                  position: 'absolute',
                  top: -19,
                  left: '50%',
                  transform: 'translateX(-50%)',
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: 6,
                  padding: '3px 10px',
                  borderRadius: 999,
                  background: alpha(status.needsYou, 0.14),
                  border: `1px solid ${alpha(status.needsYou, 0.5)}`,
                  fontFamily: fonts.mono,
                  fontSize: 12,
                  fontWeight: 600,
                  color: status.needsYou,
                  whiteSpace: 'nowrap',
                }}
              >
                <Icon name="key" size={12} color={status.needsYou} />
                scoped
              </div>
            </Appear>
          </div>
          <NodeBox icon="gear" label="Host API" color={brand.cyan} sub="via the daemon" delay={18} />
        </div>

        {/* granted scopes */}
        <Appear delay={32} y={10}>
          <div style={{ fontFamily: fonts.ui, fontSize: 13, color: alpha('#fff', 0.55), marginTop: 34, marginBottom: 12 }}>
            GRANTED SCOPES
          </div>
        </Appear>
        <div style={{ display: 'flex', gap: 12, justifyContent: 'center', flexWrap: 'wrap', maxWidth: 760 }}>
          <ScopeChip icon="folder" label="repo.read" color={brand.cyan} delay={36} />
          <ScopeChip icon="ticket" label="jira.accounts.list" color="#2684ff" delay={40} />
          <ScopeChip icon="terminal" label="agent.spawn" color={PLUGIN_VIOLET} delay={44} />
        </div>

        {/* per-role grants */}
        <Appear delay={50} y={10}>
          <div style={{ fontFamily: fonts.ui, fontSize: 13, color: alpha('#fff', 0.55), marginTop: 30, marginBottom: 12 }}>
            PER-ROLE GRANTS · slug-keyed in Settings → Plugins
          </div>
        </Appear>
        <div style={{ display: 'flex', gap: 12, justifyContent: 'center', flexWrap: 'wrap' }}>
          <ScopeChip icon="user" label="admin · Edit" color={status.working} delay={54} />
          <ScopeChip icon="user" label="lead · View" color={brand.cyan} delay={58} />
          <ScopeChip icon="user" label="guest · None" color={status.exited} delay={62} />
        </div>
      </AbsoluteFill>
      <Caption step={3} title="Scoped & RBAC-gated" />
    </>
  );
};

// ── Scene 5 — outro ──────────────────────────────────────────────────────────
const SCENES: SceneDef[] = [
  { dur: 80, node: <Title />, name: 'Title' },
  { dur: 210, node: <ManageScene />, name: 'Manage' },
  { dur: 200, node: <PluginUIScene />, name: 'Plugin UI' },
  { dur: 100, node: <HostApiScene />, name: 'Host API' },
  {
    dur: 130,
    node: (
      <WalkOutro
        title="Custom Plugins"
        tagline="Make Otto yours."
        pills={[
          { label: 'Runtime install', color: '#0a84ff', icon: 'plus' },
          { label: 'Node or Rust', color: brand.cyan, icon: 'box' },
          { label: 'Iframe UI', color: brand.violet, icon: 'panel' },
          { label: 'Scoped host API', color: '#28c840', icon: 'key' },
          { label: 'RBAC', color: '#febc2e', icon: 'user' },
        ]}
      />
    ),
    name: 'Outro',
  },
];

export const pluginsDuration = scenesDuration(SCENES);
export const Plugins: React.FC = () => <Scenes scenes={SCENES} />;
