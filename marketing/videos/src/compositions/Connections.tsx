import React from 'react';
import { useCurrentFrame } from 'remotion';
import { T, brand, fonts, alpha, status as STATUS } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow } from '../components/Frame';
import { Navigator, NavSession } from '../components/Nav';
import {
  TitleCard,
  Caption,
  Segmented,
  Field,
  Chip,
  Button,
  Toast,
  StatusDot,
  Ring,
  Terminal,
  Icon,
  Appear,
  track,
} from '../components/kit';

// ── Shared chrome ────────────────────────────────────────────────────────────
// Two live connection PTYs already open beside the agents, surfaced under the
// Connections module's nested list (counts.connections = 2).
const sessions: NavSession[] = [
  { title: 'prod-db · mysql', provider: 'mysql', status: 'working' },
  { title: 'cache-01 · redis', provider: 'redis', status: 'idle' },
  { title: 'fix auth tests', provider: 'claude', status: 'working', tasks: [2, 4] },
];

const navFor = (active: string) => (
  <Navigator
    active={active}
    sessions={sessions}
    activeSessionTitle="prod-db · mysql"
    workingCount={2}
    counts={{ connections: 2 }}
  />
);

// A little section heading used inside the content panes.
const PaneTitle: React.FC<{ icon: string; children: React.ReactNode; right?: React.ReactNode }> = ({
  icon,
  children,
  right,
}) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 16 }}>
    <Icon name={icon} size={17} color={T.textDim} />
    <span style={{ fontFamily: fonts.ui, fontSize: 17, fontWeight: 700, color: T.text, letterSpacing: -0.2 }}>
      {children}
    </span>
    <div style={{ flex: 1 }} />
    {right}
  </div>
);

// ── Scene 1 — title ──────────────────────────────────────────────────────────
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="CONNECTIONS"
    title="Every shell & database, one keystroke away"
    subtitle="SSH · MySQL · Redis · MongoDB · ClickHouse — live, beside your agents"
  />
);

// ── Scene 2 — new connection form (secrets → Keychain) ───────────────────────
const NewConnectionScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow nav={navFor('connections')} title="Otto — New Connection">
        <div style={{ display: 'flex', height: '100%', boxSizing: 'border-box' }}>
          {/* form column */}
          <div style={{ width: 600, padding: '26px 30px', boxSizing: 'border-box', borderRight: `1px solid ${T.border}` }}>
            <PaneTitle icon="plug">New Connection</PaneTitle>

            <Appear delay={8} y={12}>
              <div style={{ marginBottom: 20 }}>
                <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 500, color: T.textDim, display: 'block', marginBottom: 7 }}>
                  Type
                </span>
                <Segmented options={['SSH', 'MySQL', 'Redis', 'Mongo', 'ClickHouse']} active={1} />
              </div>
            </Appear>

            <Appear delay={16} y={12}>
              <div style={{ display: 'flex', gap: 12, marginBottom: 13 }}>
                <Field label="Host" value="prod-db.internal" mono icon="globe" style={{ flex: 2 }} />
                <Field label="Port" value="3306" mono style={{ flex: 1 }} />
              </div>
            </Appear>

            <Appear delay={22} y={12}>
              <Field label="User" value="otto_ro" mono icon="user" style={{ marginBottom: 13 }} />
            </Appear>

            <Appear delay={28} y={12}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
                <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 500, color: T.textDim }}>Password</span>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    minHeight: 32,
                    padding: '0 11px',
                    borderRadius: 5,
                    background: T.surface2,
                    border: `1px solid ${T.border}`,
                  }}
                >
                  <Icon name="key" size={14} color={brand.violet} />
                  <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 14, color: T.text, letterSpacing: 1 }}>
                    •••• · stored in Keychain
                  </span>
                  <Chip color={brand.violet}>
                    <Icon name="key" size={11} color={brand.violet} /> Keychain
                  </Chip>
                </div>
              </div>
            </Appear>

            <Appear delay={36} y={12}>
              <div style={{ display: 'flex', gap: 10, marginTop: 24 }}>
                <Button variant="default" icon="zap">Test connection</Button>
                <Button variant="primary" icon="check">Save</Button>
              </div>
            </Appear>
          </div>

          {/* helper column */}
          <div style={{ flex: 1, padding: '26px 30px', boxSizing: 'border-box' }}>
            <PaneTitle icon="key">Secrets</PaneTitle>
            <Appear delay={20} y={14}>
              <div
                style={{
                  borderRadius: 10,
                  border: `1px solid ${alpha(brand.violet, 0.4)}`,
                  background: alpha(brand.violet, 0.08),
                  padding: 18,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 10 }}>
                  <Icon name="key" size={16} color={brand.violet} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>
                    macOS Keychain
                  </span>
                </div>
                <div style={{ fontFamily: fonts.ui, fontSize: 13, lineHeight: 1.6, color: T.textDim }}>
                  Your password never appears in the UI. The database only stores an
                  opaque key reference — the secret itself lives in the Keychain.
                </div>
              </div>
            </Appear>
            <Appear delay={30} y={14}>
              <div style={{ marginTop: 16, fontFamily: fonts.mono, fontSize: 12.5, color: T.textDim, lineHeight: 1.8 }}>
                <div>db.connection.secret_ref</div>
                <div style={{ color: T.text }}>→ keychain://otto/conn/prod-db</div>
              </div>
            </Appear>
            <Appear delay={40} y={14}>
              <div style={{ marginTop: 22, display: 'flex', alignItems: 'center', gap: 8 }}>
                <Icon name="link" size={14} color={brand.cyan} />
                <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>
                  SSH supports a bastion / ProxyJump host
                </span>
              </div>
            </Appear>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Add a connection — secrets go to the Keychain"
      sub="The database only stores an opaque key reference"
    />
  </>
);

// ── Scene 3 — test connection probe ──────────────────────────────────────────
const TestScene: React.FC = () => {
  const frame = useCurrentFrame();
  // Spinner phase resolves into a "connected" result around frame 70.
  const resolved = frame > 70;
  const ringVal = resolved ? 1 : track(frame, [6, 70], [0, 0.9]);
  const dots = '.'.repeat((Math.floor(frame / 8) % 3) + 1);
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow nav={navFor('connections')} title="Otto — Test Connection">
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', padding: 40, boxSizing: 'border-box' }}>
            <div
              style={{
                width: 560,
                borderRadius: 14,
                border: `1px solid ${T.border}`,
                background: T.surface,
                padding: 36,
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'center',
                gap: 22,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <Icon name="db" size={18} color="#00758f" />
                <span style={{ fontFamily: fonts.mono, fontSize: 15, fontWeight: 600, color: T.text }}>
                  otto_ro@prod-db.internal:3306
                </span>
              </div>

              <div style={{ position: 'relative', width: 130, height: 130, display: 'grid', placeItems: 'center' }}>
                <Ring
                  value={ringVal}
                  size={130}
                  color={resolved ? STATUS.working : brand.cyan}
                  label={resolved ? undefined : '·'}
                />
                {resolved && (
                  <Appear delay={0} scale={0.6} style={{ position: 'absolute' }}>
                    <span style={{ width: 52, height: 52, borderRadius: '50%', background: alpha(STATUS.working, 0.18), display: 'grid', placeItems: 'center' }}>
                      <Icon name="check" size={30} color={STATUS.working} />
                    </span>
                  </Appear>
                )}
              </div>

              {resolved ? (
                <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                  <StatusDot kind="working" size={11} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 18, fontWeight: 700, color: T.text }}>
                    Connected
                  </span>
                  <Chip tone="ok">24 ms</Chip>
                </div>
              ) : (
                <span style={{ fontFamily: fonts.ui, fontSize: 18, fontWeight: 600, color: T.textDim }}>
                  Testing{dots}
                </span>
              )}

              <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, textAlign: 'center' }}>
                Handshake · auth · server version — verified before save
              </div>
            </div>
          </div>

          {resolved && (
            <Toast
              text="Connected · 24ms"
              tone="ok"
              delay={0}
              style={{ position: 'absolute', top: 22, right: 24 }}
            />
          )}
        </OttoWindow>
      </Stage>
      <Caption step={2} title="Test before you save" sub="A diagnostic probe verifies the handshake end-to-end" />
    </>
  );
};

// ── Scene 4 — live PTY session over the connection ───────────────────────────
const LiveScene: React.FC = () => (
  <>
    <Stage scale={0.9}>
      <OttoWindow
        nav={navFor('connections')}
        tabs={[
          { label: 'fix auth tests', icon: 'terminal', dot: 'working' },
          { label: 'prod-db · mysql', icon: 'db', active: true, dot: 'idle' },
          { label: 'cache-01 · redis', icon: 'db' },
        ]}
        title="Otto — prod-db · mysql"
      >
        <div style={{ display: 'flex', gap: 14, padding: 16, height: '100%', boxSizing: 'border-box' }}>
          {/* mysql PTY */}
          <div style={{ flex: 1.4, display: 'flex', flexDirection: 'column', background: T.termBg, border: `1px solid ${T.border}`, borderRadius: 10, overflow: 'hidden' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '9px 12px', borderBottom: `1px solid ${T.border}`, background: alpha('#fff', 0.02) }}>
              <Icon name="db" size={13} color="#00758f" />
              <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 13, fontWeight: 600, color: T.text }}>
                prod-db · mysql
              </span>
              <Chip color="#00758f">MySQL</Chip>
            </div>
            <Terminal
              delay={14}
              step={11}
              pad={14}
              fontSize={14}
              style={{ flex: 1, background: 'transparent', borderRadius: 0 }}
              lines={[
                { text: 'mysql> SELECT count(*) FROM players;', tone: 'cmd' },
                { text: '+----------+' },
                { text: '| count(*) |' },
                { text: '+----------+' },
                { text: '|  482913  |', tone: 'accent' },
                { text: '+----------+' },
                { text: '1 row in set (0.01 sec)', tone: 'dim' },
                { text: 'mysql> ▏', tone: 'cmd' },
              ]}
            />
          </div>

          {/* redis PTY */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', background: T.termBg, border: `1px solid ${T.border}`, borderRadius: 10, overflow: 'hidden' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '9px 12px', borderBottom: `1px solid ${T.border}`, background: alpha('#fff', 0.02) }}>
              <Icon name="db" size={13} color="#ff5f57" />
              <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 13, fontWeight: 600, color: T.text }}>
                cache-01 · redis
              </span>
              <Chip color="#ff5f57">Redis</Chip>
            </div>
            <Terminal
              delay={60}
              step={11}
              pad={14}
              fontSize={14}
              style={{ flex: 1, background: 'transparent', borderRadius: 0 }}
              lines={[
                { text: '127.0.0.1:6379> GET session:482913', tone: 'cmd' },
                { text: '"active"', tone: 'ok' },
                { text: '127.0.0.1:6379> DBSIZE', tone: 'cmd' },
                { text: '(integer) 19204', tone: 'accent' },
                { text: '127.0.0.1:6379> ▏', tone: 'cmd' },
              ]}
            />
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={3}
      title="A real prompt you can type into"
      sub="Interactive sessions — recent & pinned in the sidebar"
    />
  </>
);

// ── Scene 5 — outro ──────────────────────────────────────────────────────────
const OutroScene: React.FC = () => (
  <WalkOutro
    title="Connections"
    tagline="Your whole stack, in reach."
    pills={[
      { label: 'SSH', color: brand.cyan, icon: 'key' },
      { label: 'MySQL', color: '#00758f', icon: 'db' },
      { label: 'Redis', color: '#ff5f57', icon: 'db' },
      { label: 'MongoDB', color: '#28c840', icon: 'db' },
      { label: 'Keychain', color: brand.violet, icon: 'key' },
    ]}
  />
);

const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 210, node: <NewConnectionScene />, name: 'New Connection' },
  { dur: 160, node: <TestScene />, name: 'Test' },
  { dur: 200, node: <LiveScene />, name: 'Live PTY' },
  { dur: 130, node: <OutroScene />, name: 'Outro' },
];

export const connectionsDuration = scenesDuration(SCENES);
export const Connections: React.FC = () => <Scenes scenes={SCENES} />;
