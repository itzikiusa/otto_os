import React from 'react';
import { useCurrentFrame, useVideoConfig, spring } from 'remotion';
import { T, brand, fonts, alpha, providers, status } from '../theme';
import { Scenes, SceneDef, scenesDuration, Stage, WalkOutro } from '../components/scene';
import { OttoWindow, PhoneFrame, TabletFrame } from '../components/Frame';
import { Navigator } from '../components/Nav';
import {
  Appear,
  Caption,
  TitleCard,
  Terminal,
  TermLine,
  Diff,
  DiffLine,
  StatusDot,
  Chip,
  Button,
  Segmented,
  Avatar,
  Caret,
  Icon,
  Toast,
  Sparkline,
  track,
  useTyped,
} from '../components/kit';

// ─────────────────────────────────────────────────────────────────────────────
//  Role grading — the RBAC ladder None < View < Edit < Admin, color-coded.
// ─────────────────────────────────────────────────────────────────────────────
type Role = 'None' | 'View' | 'Edit' | 'Admin';

const ROLE_COLOR: Record<Role, string> = {
  None: T.textDim,
  View: '#0a84ff',
  Edit: '#28c840',
  Admin: brand.violet,
};

const RoleCell: React.FC<{ role: Role; delay: number }> = ({ role, delay }) => {
  const c = ROLE_COLOR[role];
  const muted = role === 'None';
  return (
    <Appear delay={delay} y={8} scale={0.92}>
      <div
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          minWidth: 78,
          height: 30,
          padding: '0 12px',
          borderRadius: 999,
          fontFamily: fonts.ui,
          fontSize: 14,
          fontWeight: 700,
          color: muted ? alpha(T.textDim, 0.85) : c,
          background: muted ? alpha(T.textDim, 0.1) : alpha(c, 0.16),
          border: `1px solid ${muted ? T.border : alpha(c, 0.45)}`,
        }}
      >
        {role}
      </div>
    </Appear>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 1 — Title card
// ════════════════════════════════════════════════════════════════════════════
const TitleScene: React.FC = () => (
  <TitleCard
    kicker="Team · Remote · Mobile"
    title="Your whole team — anywhere, on any device"
    subtitle="Per-feature roles, secure share links, and a real mobile app"
  />
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 2 — RBAC: per-feature roles matrix, API tokens, impersonation banner
// ════════════════════════════════════════════════════════════════════════════

interface UserRow {
  name: string;
  tag: string;
  tagTone: 'accent' | 'ok' | 'default';
  color: string;
  roles: Role[]; // [Agents, Git, Database, Brokers]
}

const FEATURES = ['Agents', 'Git', 'Database', 'Brokers'];

const USERS: UserRow[] = [
  { name: 'Alex', tag: 'Owner', tagTone: 'accent', color: brand.violet, roles: ['Admin', 'Admin', 'Admin', 'Admin'] },
  { name: 'Sam', tag: 'Member', tagTone: 'ok', color: '#0a84ff', roles: ['Edit', 'Edit', 'View', 'Edit'] },
  { name: 'Jordan', tag: 'Viewer', tagTone: 'default', color: '#febc2e', roles: ['View', 'View', 'None', 'View'] },
];

const MATRIX_GRID = '200px repeat(4, 1fr)';

const RbacScene: React.FC = () => {
  const frame = useCurrentFrame();
  const bannerY = track(frame, [96, 116], [16, 0]);
  const bannerOp = track(frame, [96, 116], [0, 1]);
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="settings" workingCount={2} />}
          tabs={[{ label: 'Team & Roles', icon: 'user', active: true }, { label: 'API Tokens', icon: 'key' }]}
          title="Otto — Settings · Access"
        >
          <div style={{ display: 'flex', height: '100%', boxSizing: 'border-box' }}>
            {/* ── left: the users × features role matrix ── */}
            <div style={{ flex: 1, minWidth: 0, padding: 22, display: 'flex', flexDirection: 'column', gap: 16 }}>
              <Appear delay={4} y={10}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                  <span style={{ fontFamily: fonts.ui, fontSize: 22, fontWeight: 750 as never, color: T.text }}>
                    Per-feature access
                  </span>
                  <Chip tone="accent">RBAC</Chip>
                  <span style={{ flex: 1 }} />
                  <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>
                    None&nbsp;&lt;&nbsp;View&nbsp;&lt;&nbsp;Edit&nbsp;&lt;&nbsp;Admin
                  </span>
                </div>
              </Appear>

              {/* matrix card */}
              <Appear delay={8} y={16}>
                <div style={{ borderRadius: 12, overflow: 'hidden', border: `1px solid ${T.border}`, background: T.surface }}>
                  {/* header row */}
                  <div
                    style={{
                      display: 'grid',
                      gridTemplateColumns: MATRIX_GRID,
                      alignItems: 'center',
                      padding: '0 18px',
                      height: 42,
                      background: T.surface2,
                      borderBottom: `1px solid ${T.border}`,
                    }}
                  >
                    <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600, letterSpacing: 0.4, textTransform: 'uppercase', color: T.textDim }}>
                      User
                    </span>
                    {FEATURES.map((f) => (
                      <span key={f} style={{ textAlign: 'center', fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600, letterSpacing: 0.4, textTransform: 'uppercase', color: T.textDim }}>
                        {f}
                      </span>
                    ))}
                  </div>
                  {/* user rows */}
                  {USERS.map((u, ri) => (
                    <div
                      key={u.name}
                      style={{
                        display: 'grid',
                        gridTemplateColumns: MATRIX_GRID,
                        alignItems: 'center',
                        padding: '0 18px',
                        height: 64,
                        borderBottom: ri < USERS.length - 1 ? `1px solid ${alpha(T.border, 0.6)}` : 'none',
                      }}
                    >
                      <Appear delay={12 + ri * 6} x={-12} y={0}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 11 }}>
                          <Avatar name={u.name} color={u.color} size={34} />
                          <div style={{ minWidth: 0 }}>
                            <div style={{ fontFamily: fonts.ui, fontSize: 15.5, fontWeight: 650 as never, color: T.text }}>{u.name}</div>
                            <Chip tone={u.tagTone} color={u.tagTone === 'default' ? '#febc2e' : undefined} style={{ height: 18, fontSize: 11, marginTop: 2 }}>
                              {u.tag}
                            </Chip>
                          </div>
                        </div>
                      </Appear>
                      {u.roles.map((role, ci) => (
                        <div key={ci} style={{ display: 'grid', placeItems: 'center' }}>
                          <RoleCell role={role} delay={20 + ri * 7 + ci * 4} />
                        </div>
                      ))}
                    </div>
                  ))}
                </div>
              </Appear>

              {/* impersonation banner — root acting as Sam, audited */}
              <div
                style={{
                  opacity: bannerOp,
                  transform: `translateY(${bannerY}px)`,
                  display: 'flex',
                  alignItems: 'center',
                  gap: 13,
                  padding: '13px 16px',
                  borderRadius: 12,
                  background: alpha('#febc2e', 0.1),
                  border: `1px solid ${alpha('#febc2e', 0.5)}`,
                }}
              >
                <span style={{ width: 34, height: 34, borderRadius: 9, display: 'grid', placeItems: 'center', background: alpha('#febc2e', 0.2), color: '#febc2e', flexShrink: 0 }}>
                  <Icon name="eye" size={18} color="#febc2e" />
                </span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontFamily: fonts.ui, fontSize: 14.5, fontWeight: 650 as never, color: T.text }}>
                    Impersonating <b>Sam</b> — viewing exactly what they see
                  </div>
                  <div style={{ fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim, marginTop: 1 }}>
                    Every action is logged to the audit trail · root · for support only
                  </div>
                </div>
                <Chip color="#febc2e">audited</Chip>
                <Button variant="default" size="s">Exit</Button>
              </div>
            </div>

            {/* ── right: API tokens mini-list + admin overview ── */}
            <div style={{ width: 320, flexShrink: 0, borderLeft: `1px solid ${T.border}`, background: T.bgSidebar, padding: 18, display: 'flex', flexDirection: 'column', gap: 12 }}>
              <Appear delay={30} y={10}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <Icon name="key" size={16} color={T.textDim} />
                  <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>API tokens</span>
                  <Button variant="primary" size="s" icon="plus">New</Button>
                </div>
              </Appear>
              {[
                { name: 'ci-deploy', scope: 'Git · Edit', tail: '••a91f', tone: status.working },
                { name: 'metrics-bot', scope: 'Usage · View', tail: '••7c20', tone: status.working },
                { name: 'nightly-swarm', scope: 'Agents · Edit', tail: '••e4d8', tone: status.needsYou },
              ].map((tk, i) => (
                <Appear key={tk.name} delay={34 + i * 6} y={12}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 11, padding: '11px 13px', borderRadius: 10, background: T.surface, border: `1px solid ${T.border}` }}>
                    <span style={{ width: 8, height: 8, borderRadius: '50%', background: tk.tone, flexShrink: 0 }} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontFamily: fonts.mono, fontSize: 13.5, fontWeight: 600, color: T.text }}>{tk.name}</div>
                      <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim, marginTop: 1 }}>{tk.scope}</div>
                    </div>
                    <span style={{ fontFamily: fonts.mono, fontSize: 12, color: alpha(T.textDim, 0.9) }}>{tk.tail}</span>
                  </div>
                </Appear>
              ))}
              <Appear delay={56} y={12}>
                <div style={{ marginTop: 'auto', padding: '12px 13px', borderRadius: 10, background: alpha('#0a84ff', 0.08), border: `1px solid ${alpha('#0a84ff', 0.3)}` }}>
                  <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim, marginBottom: 4 }}>Admin overview</div>
                  <div style={{ display: 'flex', alignItems: 'baseline', gap: 7 }}>
                    <span style={{ fontFamily: fonts.ui, fontSize: 26, fontWeight: 750 as never, color: T.text }}>3</span>
                    <span style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim }}>users · 5 live sessions</span>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 7, marginTop: 8, fontFamily: fonts.ui, fontSize: 12, color: T.textDim }}>
                    <Icon name="x" size={12} color={status.exited} />
                    Terminate any session · isolated per user
                  </div>
                </div>
              </Appear>
            </div>
          </div>
        </OttoWindow>
      </Stage>
      <Caption step={1} title="Per-feature roles, isolation & audited impersonation" sub="None < View < Edit < Admin · API tokens for CI" />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 3 — Share link (scoped, expiring, revocable) + email-OTP gate
// ════════════════════════════════════════════════════════════════════════════

const SHARE_LINK = 'otto.acme.dev/s/9fK2-aL7q';
const OTP = ['4', '8', '2', '9', '1', '3'];

const OtpBox: React.FC<{ digit: string; filled: boolean; delay: number; caret: boolean }> = ({ digit, filled, delay, caret }) => {
  const frame = useCurrentFrame();
  const pop = track(frame, [delay, delay + 7], [0.7, 1]);
  return (
    <div
      style={{
        width: 50,
        height: 62,
        borderRadius: 12,
        display: 'grid',
        placeItems: 'center',
        background: T.surface2,
        border: `1.5px solid ${filled ? T.accent : caret ? T.accent : T.border}`,
        boxShadow: filled || caret ? `0 0 0 3px ${alpha(T.accent, 0.2)}` : 'none',
        fontFamily: fonts.mono,
        fontSize: 30,
        fontWeight: 700,
        color: T.text,
        transform: filled ? `scale(${pop})` : 'scale(1)',
      }}
    >
      {filled ? digit : caret ? <Caret color={T.accent} h={30} /> : ''}
    </div>
  );
};

const ShareScene: React.FC = () => {
  const frame = useCurrentFrame();
  // How many OTP digits are filled over time (start ~frame 70).
  const filledCount = Math.min(OTP.length, Math.max(0, Math.floor((frame - 70) / 7)));
  const verified = frame > 130;
  return (
    <>
      <Stage scale={0.9}>
        <OttoWindow
          nav={<Navigator active="agents" workingCount={2} />}
          title="Otto — Share session"
        >
          <div style={{ display: 'flex', gap: 26, height: '100%', padding: 30, boxSizing: 'border-box', alignItems: 'stretch' }}>
            {/* ── left: Create share link panel ── */}
            <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
              <Appear delay={4} y={12}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 11, marginBottom: 18 }}>
                  <span style={{ width: 40, height: 40, borderRadius: 11, display: 'grid', placeItems: 'center', background: alpha(brand.cyan, 0.16), color: brand.cyan, border: `1px solid ${alpha(brand.cyan, 0.4)}` }}>
                    <Icon name="share" size={20} color={brand.cyan} />
                  </span>
                  <div>
                    <div style={{ fontFamily: fonts.ui, fontSize: 21, fontWeight: 750 as never, color: T.text }}>Create share link</div>
                    <div style={{ fontFamily: fonts.ui, fontSize: 13, color: T.textDim, marginTop: 1 }}>Scoped · expiring · revocable anytime</div>
                  </div>
                </div>
              </Appear>

              <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <Appear delay={12} y={12}>
                  <div style={{ display: 'flex', gap: 16 }}>
                    <div style={{ flex: 1 }}>
                      <FieldLabel>Scope</FieldLabel>
                      <SelectRow icon="terminal" value="fix auth tests" />
                    </div>
                    <div style={{ width: 150 }}>
                      <FieldLabel>Role</FieldLabel>
                      <SelectRow icon="eye" value="View only" />
                    </div>
                  </div>
                </Appear>

                <Appear delay={18} y={12}>
                  <div>
                    <FieldLabel>Expires in</FieldLabel>
                    <div style={{ marginTop: 6 }}>
                      <Segmented options={['1 hour', '12 hours', '24 hours']} active={1} />
                    </div>
                  </div>
                </Appear>

                <Appear delay={24} y={12}>
                  <div>
                    <FieldLabel>Send to</FieldLabel>
                    <SelectRow icon="user" value="sam@acme.com" mono />
                  </div>
                </Appear>

                {/* generated link + revoke */}
                <Appear delay={32} y={14}>
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 11,
                      padding: '13px 14px',
                      borderRadius: 12,
                      background: alpha(brand.cyan, 0.08),
                      border: `1px solid ${alpha(brand.cyan, 0.4)}`,
                    }}
                  >
                    <Icon name="link" size={16} color={brand.cyan} />
                    <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 15, color: T.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {SHARE_LINK}
                    </span>
                    <Chip tone="ok">12h left</Chip>
                    <Button variant="danger" size="s" icon="x">Revoke</Button>
                  </div>
                </Appear>
              </div>
            </div>

            {/* ── right: email-OTP gate card ── */}
            <Appear delay={40} y={16} style={{ width: 380, flexShrink: 0, display: 'flex' }}>
              <div
                style={{
                  width: '100%',
                  display: 'flex',
                  flexDirection: 'column',
                  borderRadius: 16,
                  background: T.surface,
                  border: `1px solid ${T.border}`,
                  boxShadow: '0 30px 80px rgba(0,0,0,0.5)',
                  padding: 26,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 4 }}>
                  <span style={{ width: 34, height: 34, borderRadius: 9, display: 'grid', placeItems: 'center', background: alpha('#febc2e', 0.18), color: '#febc2e' }}>
                    <Icon name="key" size={17} color="#febc2e" />
                  </span>
                  <span style={{ fontFamily: fonts.ui, fontSize: 18, fontWeight: 750 as never, color: T.text }}>Verify it's you</span>
                </div>
                <div style={{ fontFamily: fonts.ui, fontSize: 13.5, color: T.textDim, marginBottom: 20, lineHeight: 1.5 }}>
                  We sent a 6-digit code to{' '}
                  <span style={{ color: T.text, fontWeight: 600 }}>sam@acme.com</span>. Single-use · expires in 10 min.
                </div>
                <div style={{ display: 'flex', gap: 9, marginBottom: 22, justifyContent: 'center' }}>
                  {OTP.map((d, i) => (
                    <OtpBox key={i} digit={d} filled={i < filledCount} caret={i === filledCount && !verified} delay={70 + i * 7} />
                  ))}
                </div>
                <Button
                  variant="primary"
                  icon="check"
                  style={{
                    height: 44,
                    justifyContent: 'center',
                    fontSize: 15,
                    background: verified ? status.working : T.accent,
                    boxShadow: `0 8px 22px ${alpha(verified ? status.working : T.accent, 0.45)}`,
                  }}
                >
                  {verified ? 'Verified — opening session' : 'Verify & open'}
                </Button>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6, marginTop: 16, fontFamily: fonts.ui, fontSize: 12.5, color: T.textDim }}>
                  <Icon name="refresh" size={13} color={T.textDim} />
                  Extend re-issues a fresh code
                </div>
              </div>
            </Appear>
          </div>
        </OttoWindow>
      </Stage>
      <Caption step={2} title="Share a session — scoped, expiring, OTP-gated" sub="Cloudflare Tunnel + PWA · revoke anytime" />
    </>
  );
};

const FieldLabel: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <span style={{ fontFamily: fonts.ui, fontSize: 12.5, fontWeight: 600, color: T.textDim, letterSpacing: 0.2 }}>{children}</span>
);

const SelectRow: React.FC<{ icon: string; value: string; mono?: boolean }> = ({ icon, value, mono }) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      height: 42,
      marginTop: 6,
      padding: '0 13px',
      borderRadius: 10,
      background: T.surface2,
      border: `1px solid ${T.border}`,
    }}
  >
    <Icon name={icon} size={15} color={T.textDim} />
    <span style={{ flex: 1, fontFamily: mono ? fonts.mono : fonts.ui, fontSize: 14.5, color: T.text }}>{value}</span>
    <Icon name="chevronDown" size={14} color={T.textDim} />
  </div>
);

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 4 — MOBILE · phone (the PWA in your pocket)
// ════════════════════════════════════════════════════════════════════════════

const PhoneScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const typed = useTyped('add a regression test for the exp claim', 70, 22);
  const focus = spring({ frame: frame - 56, fps, config: { damping: 200 } });
  const typing = typed.length > 0 && typed.length < 38;
  return (
    <>
      <Stage scale={0.94}>
        <div style={{ position: 'relative' }}>
          <PhoneFrame title="claude · fix auth" active="agents" workingBadge={3}>
            <div style={{ display: 'flex', flexDirection: 'column', height: '100%', boxSizing: 'border-box' }}>
              {/* session header */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 9, padding: '11px 14px', borderBottom: `1px solid ${T.border}`, background: T.bgSidebar }}>
                <StatusDot kind="working" size={9} />
                <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 14, fontWeight: 600, color: T.text }}>fix auth tests</span>
                <Chip color={providers.claude}>claude</Chip>
              </div>
              {/* live terminal body */}
              <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
                <Terminal
                  lines={[
                    { text: '$ go test ./auth/...', tone: 'cmd' },
                    { text: '  reading jwt.go…', tone: 'dim' },
                    { text: '  ✗ exp claim not validated', tone: 'err' },
                    { text: '  patch → middleware/jwt.go', tone: 'text' },
                    { text: '  ✓ 142 passed (3.4s)', tone: 'ok' },
                    { text: '  awaiting next instruction…', tone: 'dim' },
                  ] as TermLine[]}
                  delay={10}
                  step={9}
                  pad={14}
                  fontSize={13.5}
                  style={{ background: 'transparent', borderRadius: 0, height: '100%' }}
                />
              </div>
              {/* touch input bar */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 9, padding: '11px 12px', borderTop: `1px solid ${T.border}`, background: T.bgSidebar }}>
                <div
                  style={{
                    flex: 1,
                    display: 'flex',
                    alignItems: 'center',
                    minHeight: 40,
                    padding: '0 14px',
                    borderRadius: 13,
                    background: T.surface2,
                    border: `1px solid ${focus > 0.5 ? T.accent : T.border}`,
                    boxShadow: focus > 0.5 ? `0 0 0 3px ${alpha(T.accent, 0.22 * focus)}` : 'none',
                    fontFamily: fonts.ui,
                    fontSize: 14,
                    color: typed ? T.text : alpha(T.textDim, 0.8),
                  }}
                >
                  <span style={{ overflow: 'hidden', whiteSpace: 'nowrap' }}>{typed || 'Type to your agent…'}</span>
                  {typing && <Caret color={T.accent} h={16} />}
                </div>
                <div style={{ width: 42, height: 42, borderRadius: 13, flexShrink: 0, display: 'grid', placeItems: 'center', background: T.accent, color: '#fff', boxShadow: `0 6px 16px ${alpha(T.accent, 0.45)}` }}>
                  <Icon name="arrowUp" size={19} color="#fff" />
                </div>
              </div>
            </div>
          </PhoneFrame>

          {/* PWA "Add to Home Screen" hint floating beside the phone */}
          <Appear delay={26} y={0} x={26} style={{ position: 'absolute', top: 96, left: '100%', marginLeft: 36 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '13px 16px', borderRadius: 14, background: T.surface, border: `1px solid ${T.border}`, boxShadow: '0 24px 60px rgba(0,0,0,0.5)', width: 240 }}>
              <span style={{ width: 44, height: 44, borderRadius: 11, flexShrink: 0, display: 'grid', placeItems: 'center', background: 'linear-gradient(150deg,#2a1860,#14101f)', boxShadow: `0 6px 18px ${alpha(brand.glow, 0.5)}` }}>
                <Icon name="globe" size={20} color={brand.cyan} />
              </span>
              <div>
                <div style={{ fontFamily: fonts.ui, fontSize: 14, fontWeight: 700, color: T.text }}>Add Otto to Home Screen</div>
                <div style={{ fontFamily: fonts.ui, fontSize: 11.5, color: T.textDim, marginTop: 1 }}>Installable PWA · opens like a native app</div>
              </div>
            </div>
          </Appear>

          {/* per-device chip below the hint */}
          <Appear delay={40} y={10} style={{ position: 'absolute', top: 230, left: '100%', marginLeft: 36 }}>
            <Toast text="iPhone · this device’s sessions only" tone="ok" />
          </Appear>
        </div>
      </Stage>
      <Caption
        step={3}
        title="A real app in your pocket"
        sub="Installable PWA · touch terminal · per-device sessions · light/dark + RTL"
      />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 5 — MOBILE · tablet (iPad landscape, full responsive 2-pane layout)
// ════════════════════════════════════════════════════════════════════════════

const DIFF_LINES: DiffLine[] = [
  { text: 'middleware/jwt.go', kind: 'hunk' },
  { text: 'func Validate(tok *Token) error {', kind: 'ctx' },
  { text: '  if tok.Sub == "" {', kind: 'del' },
  { text: '  if tok.Sub == "" || tok.Exp == 0 {', kind: 'add' },
  { text: '    return ErrUnauthorized', kind: 'ctx' },
  { text: '  }', kind: 'ctx' },
  { text: '  if time.Now().After(tok.Exp) {', kind: 'add' },
  { text: '    return ErrExpired', kind: 'add' },
  { text: '  }', kind: 'add' },
  { text: '  return nil', kind: 'ctx' },
];

const TabletNav = <Navigator active="git" width={200} workingCount={3} />;

const TabletScene: React.FC = () => {
  const frame = useCurrentFrame();
  const reviewOp = track(frame, [70, 88], [0, 1]);
  return (
    <>
      <Stage scale={0.92}>
        <TabletFrame nav={TabletNav} title="Otto — iPad" height={840}>
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
            {/* content header */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 11, padding: '12px 18px', borderBottom: `1px solid ${T.border}`, background: T.bgSidebar, flexShrink: 0 }}>
              <Icon name="branch" size={16} color={T.textDim} />
              <span style={{ fontFamily: fonts.ui, fontSize: 15, fontWeight: 700, color: T.text }}>sinatra-users-go</span>
              <Chip tone="accent">fix/jwt-exp</Chip>
              <span style={{ flex: 1 }} />
              <Chip tone="ok">+6</Chip>
              <Chip tone="bad">−1</Chip>
              <Button variant="primary" size="s" icon="pr">Open PR</Button>
            </div>

            {/* the responsive 2-pane content: file list + diff, independently scrollable */}
            <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
              {/* left mini pane — changed files (a collapsible section) */}
              <div style={{ width: 230, flexShrink: 0, borderRight: `1px solid ${T.border}`, background: T.bg, padding: '12px 10px', display: 'flex', flexDirection: 'column', gap: 4 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '4px 8px', fontFamily: fonts.ui, fontSize: 11.5, fontWeight: 600, letterSpacing: 0.4, textTransform: 'uppercase', color: T.textDim }}>
                  <Icon name="chevronDown" size={12} color={T.textDim} />
                  Changed files
                </div>
                {[
                  { f: 'middleware/jwt.go', add: '+6', del: '−1', active: true },
                  { f: 'auth/handler.go', add: '+2', del: '', active: false },
                  { f: 'auth/jwt_test.go', add: '+18', del: '', active: false },
                ].map((row, i) => (
                  <Appear key={row.f} delay={8 + i * 5} x={-10} y={0}>
                    <div
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                        height: 32,
                        padding: '0 9px',
                        borderRadius: 8,
                        background: row.active ? alpha(T.accent, 0.14) : 'transparent',
                        border: `1px solid ${row.active ? alpha(T.accent, 0.35) : 'transparent'}`,
                      }}
                    >
                      <Icon name="file" size={13} color={row.active ? T.accent : T.textDim} />
                      <span style={{ flex: 1, fontFamily: fonts.mono, fontSize: 12, color: row.active ? T.text : T.textDim, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {row.f}
                      </span>
                      <span style={{ fontFamily: fonts.mono, fontSize: 11, color: status.working }}>{row.add}</span>
                      {row.del && <span style={{ fontFamily: fonts.mono, fontSize: 11, color: status.exited }}>{row.del}</span>}
                    </div>
                  </Appear>
                ))}

                {/* a second collapsible section to prove independent panes */}
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '4px 8px', marginTop: 8, fontFamily: fonts.ui, fontSize: 11.5, fontWeight: 600, letterSpacing: 0.4, textTransform: 'uppercase', color: T.textDim }}>
                  <Icon name="chevronRight" size={12} color={T.textDim} />
                  Usage today
                </div>
                <Appear delay={30} y={10}>
                  <div style={{ padding: '8px 9px', borderRadius: 8, background: T.surface, border: `1px solid ${T.border}` }}>
                    <Sparkline data={[3, 5, 4, 7, 6, 9, 8, 12]} color={brand.cyan} width={188} height={40} />
                    <div style={{ fontFamily: fonts.ui, fontSize: 11, color: T.textDim, marginTop: 4 }}>$2.41 · 318k tokens</div>
                  </div>
                </Appear>
              </div>

              {/* right pane — the diff (the second independently-scrollable pane) */}
              <div style={{ flex: 1, minWidth: 0, padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
                <Appear delay={14} y={12}>
                  <Diff lines={DIFF_LINES} delay={18} step={4} fontSize={13.5} style={{ flexShrink: 0 }} />
                </Appear>
                {/* review verdict, fades in to keep the scene alive to the end */}
                <div
                  style={{
                    opacity: reviewOp,
                    display: 'flex',
                    alignItems: 'center',
                    gap: 11,
                    padding: '12px 14px',
                    borderRadius: 11,
                    background: alpha(status.working, 0.1),
                    border: `1px solid ${alpha(status.working, 0.4)}`,
                  }}
                >
                  <span style={{ width: 30, height: 30, borderRadius: 8, display: 'grid', placeItems: 'center', background: alpha(status.working, 0.2), color: status.working, flexShrink: 0 }}>
                    <Icon name="check" size={16} color={status.working} />
                  </span>
                  <span style={{ flex: 1, fontFamily: fonts.ui, fontSize: 13.5, color: T.text }}>
                    exp-claim now validated · tests pass — ready to merge
                  </span>
                  <Chip tone="ok">approved</Chip>
                </div>
              </div>
            </div>
          </div>
        </TabletFrame>
      </Stage>
      <Caption
        step={4}
        title="iPad, with the full layout"
        sub="Persistent navigator · collapsible, independently-scrollable panes"
      />
    </>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  SCENE 6 — WalkOutro
// ════════════════════════════════════════════════════════════════════════════
const OutroScene: React.FC = () => (
  <WalkOutro
    title="Team, Remote & Mobile"
    tagline="Otto goes where you go."
    pills={[
      { label: 'Per-feature RBAC', color: '#0a84ff', icon: 'user' },
      { label: 'Share links', color: brand.cyan, icon: 'share' },
      { label: 'Email-OTP', color: '#febc2e', icon: 'key' },
      { label: 'Cloudflare Tunnel', color: brand.violet, icon: 'globe' },
      { label: 'PWA + touch', color: '#28c840', icon: 'command' },
    ]}
  />
);

// ════════════════════════════════════════════════════════════════════════════
//  COMPOSITION
// ════════════════════════════════════════════════════════════════════════════
const SCENES: SceneDef[] = [
  { dur: 80, node: <TitleScene />, name: 'Title' },
  { dur: 220, node: <RbacScene />, name: 'RBAC' },
  { dur: 200, node: <ShareScene />, name: 'Share & OTP' },
  { dur: 240, node: <PhoneScene />, name: 'Phone' },
  { dur: 220, node: <TabletScene />, name: 'Tablet' },
  { dur: 130, node: <OutroScene />, name: 'Outro' },
];

export const teamMobileDuration: number = scenesDuration(SCENES);
export const TeamMobile: React.FC = () => <Scenes scenes={SCENES} />;
