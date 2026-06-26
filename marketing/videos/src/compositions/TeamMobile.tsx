import React from 'react';
import { T, brand, fonts, alpha, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro, FloorGlow } from '../components/scene';
import { OttoWindow, PhoneFrame, TabletFrame } from '../components/Frame';
import { Navigator, NavSession } from '../components/Nav';
import {
  Appear,
  TitleCard,
  Caption,
  Chip,
  Button,
  Field,
  Segmented,
  Table,
  Icon,
  StatusDot,
  Toast,
} from '../components/kit';

// ── helpers ───────────────────────────────────────────────────────────────────

const roleColor = (role: string): string =>
  role === 'Admin' ? brand.purple
  : role === 'Edit' ? T.accent
  : role === 'View' ? status.working
  : T.textDim;

const RoleChip: React.FC<{ role: string }> = ({ role }) => (
  <Chip color={roleColor(role)} style={{ fontSize: 11 }}>{role}</Chip>
);

const UserCell: React.FC<{ initial: string; name: string; sub: string; color: string }> = ({
  initial, name, sub, color,
}) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
    <div style={{
      width: 24, height: 24, borderRadius: '50%',
      background: alpha(color, 0.26), color,
      display: 'grid', placeItems: 'center',
      fontFamily: fonts.ui, fontSize: 11, fontWeight: 700, flexShrink: 0,
    }}>{initial}</div>
    <div>
      <div style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600, color: T.text }}>{name}</div>
      <div style={{ fontFamily: fonts.ui, fontSize: 10.5, color: T.textDim }}>{sub}</div>
    </div>
  </div>
);

// ── shared data ───────────────────────────────────────────────────────────────

const activeSessions: NavSession[] = [
  { title: 'fix: payment gateway', provider: 'claude', status: 'working', tasks: [3, 6] },
  { title: 'review: auth module',  provider: 'codex',  status: 'idle',    tasks: [4, 4] },
];

// ── Scene 1 — Title (~75f) ────────────────────────────────────────────────────

const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Multi-user · Sharing · Mobile"
    title="Team & Mobile"
    subtitle="Per-feature RBAC · scoped share links · email-OTP · installable PWA — your whole team, safely, on any device."
  />
);

// ── Scene 2 — RBAC grant matrix (~165f) ──────────────────────────────────────

const rbacRows: (string | React.ReactNode)[][] = [
  [
    <UserCell key="alex" initial="A" name="Alex" sub="root · owner" color={brand.purple} />,
    <RoleChip key="g"  role="Admin" />,
    <RoleChip key="d"  role="Admin" />,
    <RoleChip key="sw" role="Admin" />,
    <RoleChip key="sc" role="Admin" />,
  ],
  [
    <UserCell key="sam" initial="S" name="Sam" sub="engineer" color={T.accent} />,
    <RoleChip key="g"  role="Edit" />,
    <RoleChip key="d"  role="Edit" />,
    <RoleChip key="sw" role="View" />,
    <RoleChip key="sc" role="None" />,
  ],
  [
    <UserCell key="jordan" initial="J" name="Jordan" sub="analyst" color={status.working} />,
    <RoleChip key="g"  role="View" />,
    <RoleChip key="d"  role="View" />,
    <RoleChip key="sw" role="None" />,
    <RoleChip key="sc" role="View" />,
  ],
];

const RbacScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={
          <Navigator
            active="settings"
            sessions={activeSessions}
            workingCount={1}
            activeSessionTitle="fix: payment gateway"
            user={{ name: 'Alex', sub: 'root · admin' }}
          />
        }
        title="Otto — Settings · Users"
      >
        <div style={{
          position: 'absolute', inset: 0,
          padding: 22, display: 'flex', flexDirection: 'column', gap: 18,
          boxSizing: 'border-box', overflow: 'hidden',
        }}>

          {/* page header */}
          <Appear delay={4}>
            <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between' }}>
              <div>
                <div style={{ fontFamily: fonts.ui, fontSize: 21, fontWeight: 700, color: T.text }}>
                  Users &amp; Permissions
                </div>
                <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginTop: 4 }}>
                  Per-feature roles — None &lt; View &lt; Edit &lt; Admin — with per-session data isolation
                </div>
              </div>
              <Button variant="primary" icon="plus">Invite user</Button>
            </div>
          </Appear>

          {/* grant matrix */}
          <Appear delay={12}>
            <Table
              columns={['User', 'Git', 'Database', 'Swarm', 'Scheduled Tasks']}
              rows={rbacRows}
              widths={['2fr', '1fr', '1fr', '1fr', '1.5fr']}
              delay={16}
              step={14}
              fontSize={12.5}
            />
          </Appear>

          {/* role legend */}
          <Appear delay={62}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 20 }}>
              <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>Role hierarchy:</span>
              {['None', 'View', 'Edit', 'Admin'].map((r) => (
                <div key={r} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <div style={{ width: 8, height: 8, borderRadius: '50%', background: roleColor(r) }} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>{r}</span>
                </div>
              ))}
            </div>
          </Appear>

          {/* impersonation banner */}
          <Appear delay={78}>
            <div style={{
              display: 'flex', gap: 12, padding: '12px 16px',
              background: alpha(brand.purple, 0.09),
              border: `1px solid ${alpha(brand.purple, 0.28)}`,
              borderRadius: 9,
            }}>
              <Icon name="eye" size={16} color={brand.purple} />
              <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>
                <span style={{ color: brand.purple, fontWeight: 600 }}>Admin impersonation:</span>
                {' '}act-as any user for debugging — every action is logged with the impersonator&apos;s identity.
              </span>
            </div>
          </Appear>

        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={1}
      title="Per-feature RBAC — None < View < Edit < Admin"
      sub="Grant matrix per user × feature · per-session isolation · audited admin impersonation"
    />
  </>
);

// ── Scene 3 — Share link + Email-OTP (~145f) ──────────────────────────────────

const ShareScene: React.FC = () => (
  <>
    <Stage scale={0.88}>
      <OttoWindow
        nav={
          <Navigator
            active="agents"
            sessions={activeSessions}
            workingCount={1}
            activeSessionTitle="fix: payment gateway"
            user={{ name: 'Alex', sub: 'root · admin' }}
          />
        }
        tabs={[
          { label: 'fix: payment gateway', icon: 'terminal', active: true, dot: 'working' },
          { label: 'review: auth module',  icon: 'terminal' },
        ]}
        title="Otto — sinatra-api"
      >
        <div style={{ position: 'absolute', inset: 0, display: 'flex' }}>

          {/* dimmed terminal backdrop */}
          <div style={{
            flex: 1,
            background: T.termBg,
            padding: 18,
            opacity: 0.22,
            fontFamily: fonts.mono, fontSize: 13, color: T.textDim,
          }}>
            $ claude code --resume fix/payment-gw
          </div>

          {/* share side panel */}
          <div style={{
            width: 368, flexShrink: 0,
            background: T.bgSidebar,
            borderLeft: `1px solid ${T.border}`,
            display: 'flex', flexDirection: 'column',
          }}>

            {/* panel header */}
            <div style={{
              height: 44, flexShrink: 0,
              display: 'flex', alignItems: 'center', gap: 9,
              padding: '0 16px',
              borderBottom: `1px solid ${T.border}`,
            }}>
              <Icon name="share" size={15} color={T.text} />
              <span style={{
                flex: 1, fontFamily: fonts.ui, fontSize: 14,
                fontWeight: 700, color: T.text,
              }}>Share session</span>
              <Icon name="x" size={14} color={T.textDim} />
            </div>

            {/* panel body */}
            <div style={{
              padding: 16, display: 'flex', flexDirection: 'column',
              gap: 14, overflow: 'hidden',
            }}>

              <Appear delay={6}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 500, color: T.textDim }}>
                    Access level
                  </span>
                  <Segmented options={['Viewer', 'Editor']} active={0} />
                </div>
              </Appear>

              <Appear delay={14}>
                <Field
                  label="Share link"
                  value="https://otto.sh/s/k9x2mQ7p"
                  mono
                  icon="link"
                />
              </Appear>

              <Appear delay={22}>
                <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                  <div style={{
                    flex: 1, height: 30, padding: '0 12px',
                    background: T.surface, border: `1px solid ${T.border}`,
                    borderRadius: 6,
                    display: 'flex', alignItems: 'center', gap: 8,
                  }}>
                    <Icon name="clock" size={13} color={T.textDim} />
                    <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                      Expires in 24 h
                    </span>
                  </div>
                  <Button variant="ghost" icon="trash" size="s">Revoke</Button>
                </div>
              </Appear>

              {/* divider */}
              <Appear delay={32}>
                <div style={{ height: 1, background: T.border }} />
              </Appear>

              {/* OTP gate */}
              <Appear delay={38}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <Icon name="key" size={14} color={brand.cyan} />
                    <span style={{ fontFamily: fonts.ui, fontSize: 13, fontWeight: 600, color: T.text }}>
                      Email-OTP gate
                    </span>
                  </div>
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>
                    Visitor enters their email → receives a 6-digit code to enter
                  </span>
                </div>
              </Appear>

              <Appear delay={48}>
                <Field
                  label="Visitor email"
                  value="sam.chen@acme.io"
                  icon="user"
                />
              </Appear>

              {/* 6-digit OTP boxes */}
              <Appear delay={58}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <span style={{ fontFamily: fonts.ui, fontSize: 12, fontWeight: 500, color: T.textDim }}>
                    One-time code
                  </span>
                  <div style={{ display: 'flex', gap: 7 }}>
                    {(['4', '2', '7', '—', '—', '—'] as const).map((d, i) => (
                      <div
                        key={i}
                        style={{
                          flex: 1, height: 42, borderRadius: 7,
                          background: T.surface,
                          border: `1px solid ${i < 3 ? brand.cyan : T.border}`,
                          boxShadow: i < 3 ? `0 0 0 2.5px ${alpha(brand.cyan, 0.2)}` : 'none',
                          display: 'grid', placeItems: 'center',
                          fontFamily: fonts.mono,
                          fontSize: i < 3 ? 20 : 16,
                          fontWeight: 700,
                          color: i < 3 ? brand.cyan : T.textDim,
                        }}
                      >
                        {d}
                      </div>
                    ))}
                  </div>
                </div>
              </Appear>

              <Appear delay={74}>
                <Button variant="primary" icon="check">Grant access</Button>
              </Appear>

              <Toast
                text="Access granted · sam.chen@acme.io"
                tone="ok"
                delay={92}
              />

            </div>
          </div>
        </div>
      </OttoWindow>
    </Stage>
    <Caption
      step={2}
      title="Scoped, expiring, revocable share links"
      sub="Viewer or editor · 24-hour TTL · email-OTP gate keeps anonymous visitors out"
    />
  </>
);

// ── Scene 4 — Phone + Tablet (~175f) ──────────────────────────────────────────

const phoneRows: { title: string; provider: string; kind: keyof typeof status; tasks: string }[] = [
  { title: 'fix: payment gateway', provider: 'claude', kind: 'working',  tasks: '3/6' },
  { title: 'review: auth module',  provider: 'codex',  kind: 'idle',     tasks: '4/4' },
  { title: 'add rate-limit tests', provider: 'claude', kind: 'needsYou', tasks: '1/3' },
];

const PhoneContent: React.FC = () => (
  <div style={{ display: 'flex', flexDirection: 'column' }}>
    {phoneRows.map((s, i) => (
      <Appear key={s.title} delay={i * 9 + 5}>
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '11px 14px',
          borderBottom: `1px solid ${alpha(T.border, 0.55)}`,
          background: i === 0 ? alpha(T.accent, 0.07) : 'transparent',
        }}>
          <StatusDot kind={s.kind} size={8} pulse />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{
              fontFamily: fonts.ui, fontSize: 13,
              fontWeight: i === 0 ? 600 : 500, color: T.text,
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
            }}>{s.title}</div>
            <div style={{ fontFamily: fonts.ui, fontSize: 11, color: T.textDim, marginTop: 2 }}>
              {s.provider} · {s.tasks}
            </div>
          </div>
          {s.kind === 'needsYou' && (
            <span style={{
              padding: '2px 7px', borderRadius: 999,
              background: alpha(status.needsYou, 0.18),
              fontFamily: fonts.ui, fontSize: 10, fontWeight: 700,
              color: status.needsYou,
            }}>Needs you</span>
          )}
          <Icon name="chevronRight" size={12} color={T.textDim} />
        </div>
      </Appear>
    ))}
  </div>
);

const TabletContent: React.FC = () => (
  <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
    <Appear delay={10}>
      <div style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>
        fix: payment gateway
      </div>
    </Appear>
    <Appear delay={16}>
      <div style={{
        background: T.termBg, borderRadius: 8, padding: '12px 14px',
        fontFamily: fonts.mono, fontSize: 12, lineHeight: 1.75, color: T.textDim,
      }}>
        <div style={{ color: brand.cyan }}>{'$ claude code --resume fix/payment-gw'}</div>
        <div>{'  → reading: payment/gateway.go, webhook.go'}</div>
        <div style={{ color: status.working }}>{'  ✓ signature validation patched (3 files)'}</div>
        <div>{'  re-running 48 tests…'}</div>
        <div style={{ color: status.working }}>{'  ✓ PASS — 48/48  (2.1s)'}</div>
      </div>
    </Appear>
    <Appear delay={36}>
      <div style={{
        display: 'flex', gap: 8, alignItems: 'center',
        padding: '10px 14px',
        background: alpha(status.working, 0.08),
        border: `1px solid ${alpha(status.working, 0.22)}`,
        borderRadius: 8,
      }}>
        <Icon name="check" size={14} color={status.working} />
        <span style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.text, flex: 1 }}>
          PR draft ready · 3 files changed, +42 −11
        </span>
        <Button variant="primary" size="s" icon="pr">Open PR</Button>
      </div>
    </Appear>
  </div>
);

const MobileScene: React.FC = () => (
  <>
    <Stage scale={0.8} enter="up">
      <div style={{ display: 'flex', gap: 36, alignItems: 'center' }}>

        <PhoneFrame
          title="Agents"
          active="agents"
          workingBadge={2}
          height={640}
        >
          <PhoneContent />
        </PhoneFrame>

        <TabletFrame
          nav={
            <Navigator
              active="agents"
              sessions={activeSessions}
              workingCount={1}
              activeSessionTitle="fix: payment gateway"
              width={200}
              user={{ name: 'Alex', sub: 'root' }}
            />
          }
          title="Otto"
          height={640}
        >
          <TabletContent />
        </TabletFrame>

      </div>
    </Stage>

    <Caption
      step={3}
      title="Phone & iPad — the full Otto shell, responsive"
      sub="Installable PWA over a Cloudflare tunnel · collapsible nav · light/dark · daemon stays loopback-only by default"
    />

    <FloorGlow color={brand.cyan} w={860} />
  </>
);

// ── Composition ───────────────────────────────────────────────────────────────

const SCENES: SceneDef[] = [
  { dur: 75,  node: <TitleScene />,  name: 'Title'  },
  { dur: 165, node: <RbacScene />,   name: 'RBAC'   },
  { dur: 145, node: <ShareScene />,  name: 'Share'  },
  { dur: 175, node: <MobileScene />, name: 'Mobile' },
  {
    dur: 120,
    name: 'Outro',
    node: (
      <WalkOutro
        title="Multi-user, Sharing & Mobile"
        tagline="Your whole team — on any device, safely"
        pills={[
          { label: 'Per-feature RBAC',   icon: 'key'   },
          { label: 'Scoped share links', icon: 'share' },
          { label: 'Email-OTP gate',     icon: 'user'  },
          { label: 'PWA + tunnel',       icon: 'globe' },
        ]}
      />
    ),
  },
];

export const teamMobileDuration = scenesDuration(SCENES);
export const TeamMobile: React.FC = () => <Scenes scenes={SCENES} />;
