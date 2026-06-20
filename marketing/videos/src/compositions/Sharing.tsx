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

// ─── Sharing — Multi-user RBAC + session sharing + remote/mobile — ~34s ──────
// Scenes: RBAC user list → create share link (scoped, expiring, OTP-gated) →
// guest OTP gate → mobile/PWA + Cloudflare tunnel overview.
// ─────────────────────────────────────────────────────────────────────────────

const TITLE_DUR  = 75;
const S1_DUR     = 195;  // RBAC user list
const S2_DUR     = 210;  // share link form
const S3_DUR     = 150;  // OTP gate (guest view)
const S4_DUR     = 120;  // remote / mobile overview
const OUTRO_DUR  = 90;

const S1_START   = TITLE_DUR;
const S2_START   = S1_START + S1_DUR;
const S3_START   = S2_START + S2_DUR;
const S4_START   = S3_START + S3_DUR;
const OUTRO_START = S4_START + S4_DUR;

// ─── Helpers ─────────────────────────────────────────────────────────────────

function typewriter(text: string, frame: number, cps = 20): string {
  const chars = Math.floor((frame / 30) * cps);
  return text.slice(0, Math.min(chars, text.length));
}

const HR: React.FC = () => (
  <div style={{ height: 1, background: theme.border }} />
);

// ─── Role level colors ────────────────────────────────────────────────────────
const LEVEL_COLOR: Record<string, string> = {
  None:  theme.textDim,
  View:  '#63e6be',
  Edit:  theme.accent,
  Admin: '#bf7aff',
};

const LevelBadge: React.FC<{ level: string }> = ({ level }) => {
  const c = LEVEL_COLOR[level] ?? theme.textDim;
  return <span style={{ fontFamily: theme.mono, fontSize: 11, fontWeight: 700, color: c, background: `${c}22`, border: `1px solid ${c}44`, borderRadius: 6, padding: '2px 8px', letterSpacing: 0.4 }}>{level}</span>;
};

// ─── Scene 1 – RBAC user list ─────────────────────────────────────────────────
const USERS = [
  { name: 'itzik',   email: 'itzik@example.com',   role: 'Owner',  grants: { Agents: 'Admin', Git: 'Admin', Connections: 'Admin', Insights: 'Admin' } },
  { name: 'alex',    email: 'alex@example.com',     role: 'Member', grants: { Agents: 'Edit',  Git: 'Edit',  Connections: 'View',  Insights: 'View' } },
  { name: 'taylor',  email: 'taylor@example.com',   role: 'Member', grants: { Agents: 'View',  Git: 'None',  Connections: 'None',  Insights: 'View' } },
];

const Scene1RBAC: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      <Appear delay={4}>
        <div style={{ padding: '22px 28px 0', display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between' }}>
          <div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 26, fontWeight: 800 }}>Users</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 15, marginTop: 4 }}>
              Per-feature access control — <span style={{ fontFamily: theme.mono, color: theme.textDim, fontSize: 13 }}>None &lt; View &lt; Edit &lt; Admin</span>
            </div>
          </div>
          <div style={{ padding: '9px 20px', borderRadius: 10, background: theme.accent, color: '#fff', fontFamily: theme.font, fontSize: 14, fontWeight: 700, boxShadow: `0 6px 20px ${theme.accent}44` }}>
            + Invite user
          </div>
        </div>
      </Appear>

      <div style={{ padding: '20px 28px', flex: 1, display: 'flex', flexDirection: 'column', gap: 16, overflow: 'hidden' }}>
        {USERS.map((user, i) => {
          const s = spring({ frame: frame - (i * 14 + 24), fps, config: { damping: 200 } });
          const isOwner = user.role === 'Owner';
          return (
            <div key={user.name} style={{ opacity: s, transform: `translateX(${interpolate(s, [0, 1], [14, 0])}px)`, background: theme.surface2, borderRadius: 14, border: `1px solid ${isOwner ? '#bf7aff44' : theme.border}`, padding: '18px 22px' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 14 }}>
                <div style={{ width: 40, height: 40, borderRadius: 12, background: isOwner ? '#bf7aff22' : `${theme.accent}22`, border: `1px solid ${isOwner ? '#bf7aff44' : `${theme.accent}44`}`, display: 'grid', placeItems: 'center', fontFamily: theme.font, fontSize: 18, fontWeight: 700, color: isOwner ? '#bf7aff' : theme.accent, flexShrink: 0 }}>
                  {user.name[0].toUpperCase()}
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 2 }}>
                    <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 15, fontWeight: 700 }}>{user.name}</span>
                    <LevelBadge level={user.role} />
                  </div>
                  <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>{user.email}</div>
                </div>
                {!isOwner && (
                  <div style={{ padding: '6px 14px', borderRadius: 8, border: `1px solid ${theme.border}`, color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>Edit grants</div>
                )}
              </div>
              {/* feature grants */}
              <HR />
              <div style={{ display: 'flex', gap: 12, marginTop: 12, flexWrap: 'wrap' }}>
                {Object.entries(user.grants).map(([feature, level]) => (
                  <div key={feature} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <span style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 12 }}>{feature}</span>
                    <LevelBadge level={level} />
                  </div>
                ))}
              </div>
            </div>
          );
        })}
      </div>

      <Caption step={1} title="Multi-user RBAC" sub="Per-feature grants: None < View < Edit < Admin" delay={55} />
    </div>
  );
};

// ─── Scene 2 – Share link form ────────────────────────────────────────────────
const Scene2ShareLink: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const modalS = spring({ frame, fps, config: { damping: 180 } });
  const emailVal = frame >= 40 ? typewriter('reviewer@client.com', frame - 40, 16) : '';
  const EXPIRY_START = 80;

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
      <div style={{ opacity: modalS, transform: `scale(${interpolate(modalS, [0, 1], [0.9, 1])})`, width: 600, background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: 18, boxShadow: '0 40px 100px rgba(0,0,0,0.7)', padding: '30px 34px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 24 }}>
          <div style={{ width: 44, height: 44, borderRadius: 12, background: `${theme.accent}22`, border: `1px solid ${theme.accent}44`, display: 'grid', placeItems: 'center', fontSize: 22, flexShrink: 0 }}>🔗</div>
          <div>
            <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 20, fontWeight: 800 }}>Share session</div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13, marginTop: 2 }}>
              Create a scoped, expiring link — OTP-gated to a specific recipient
            </div>
          </div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
          {/* session */}
          <div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 0.8, textTransform: 'uppercase', marginBottom: 6 }}>Session</div>
            <div style={{ background: theme.surface2, borderRadius: 8, padding: '10px 14px', fontFamily: theme.mono, fontSize: 14, color: theme.text, border: `1px solid ${theme.border}` }}>feat/rbac-multiuser · claude</div>
          </div>

          {/* recipient email */}
          <div>
            <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 0.8, textTransform: 'uppercase', marginBottom: 6 }}>Recipient email (OTP locked)</div>
            <div style={{ background: theme.surface2, borderRadius: 8, padding: '10px 14px', fontFamily: theme.mono, fontSize: 14, color: emailVal ? theme.text : theme.textDim, border: `1px solid ${emailVal ? theme.accent : theme.border}`, boxShadow: emailVal ? `0 0 0 3px ${theme.accent}22` : 'none', minHeight: 44 }}>
              {emailVal || <span style={{ opacity: 0.5 }}>reviewer@example.com</span>}
            </div>
          </div>

          {/* expiry picker */}
          {frame >= EXPIRY_START && (() => {
            const expS = spring({ frame: frame - EXPIRY_START, fps, config: { damping: 180 } });
            return (
              <div style={{ opacity: expS, transform: `translateY(${interpolate(expS, [0, 1], [10, 0])}px)` }}>
                <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 11, fontWeight: 700, letterSpacing: 0.8, textTransform: 'uppercase', marginBottom: 8 }}>Expires</div>
                <div style={{ display: 'flex', gap: 8 }}>
                  {['1h', '4h', '12h', '24h'].map((t) => {
                    const active = t === '4h';
                    return (
                      <div key={t} style={{ flex: 1, textAlign: 'center', padding: '8px 0', borderRadius: 8, background: active ? `${theme.accent}18` : 'transparent', border: `1px solid ${active ? theme.accent : theme.border}`, color: active ? theme.accent : theme.textDim, fontFamily: theme.mono, fontSize: 13, fontWeight: active ? 700 : 400 }}>{t}</div>
                    );
                  })}
                </div>
              </div>
            );
          })()}

          {/* scope */}
          {frame >= EXPIRY_START + 30 && (() => {
            const scopeS = spring({ frame: frame - (EXPIRY_START + 30), fps, config: { damping: 180 } });
            return (
              <div style={{ opacity: scopeS, transform: `translateY(${interpolate(scopeS, [0, 1], [10, 0])}px)`, background: `${theme.accent2}11`, border: `1px solid ${theme.accent2}33`, borderRadius: 10, padding: '12px 16px', display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ fontSize: 16 }}>🔒</span>
                <div style={{ color: theme.accent2, fontFamily: theme.font, fontSize: 13 }}>Scoped to this session only · read access · no write permissions</div>
              </div>
            );
          })()}
        </div>

        <div style={{ display: 'flex', gap: 12, marginTop: 26, justifyContent: 'flex-end' }}>
          <div style={{ padding: '10px 22px', border: `1px solid ${theme.border}`, borderRadius: 10, color: theme.textDim, fontFamily: theme.font, fontSize: 15, fontWeight: 600 }}>Cancel</div>
          <div style={{ padding: '10px 28px', background: theme.accent, borderRadius: 10, color: '#fff', fontFamily: theme.font, fontSize: 15, fontWeight: 700, boxShadow: `0 6px 20px ${theme.accent}44` }}>Create link</div>
        </div>
      </div>

      <Caption step={2} title="Scoped share links" sub="Expiring, OTP-gated — recipient-locked for security" delay={65} />
    </div>
  );
};

// ─── Scene 3 – Guest OTP gate ─────────────────────────────────────────────────
const Scene3OTP: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const gateS = spring({ frame, fps, config: { damping: 180 } });
  const OTP_START = 70;
  const showSuccess = frame >= 120;
  const successS = spring({ frame: frame - 120, fps, config: { damping: 180 } });

  const OTP_DIGITS = ['3', '8', '1', '9', '5', '2'];

  return (
    <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', background: theme.bgGradient }}>
      {!showSuccess ? (
        <div style={{ opacity: gateS, transform: `scale(${interpolate(gateS, [0, 1], [0.9, 1])})`, width: 480, background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: 18, boxShadow: '0 40px 100px rgba(0,0,0,0.8)', padding: '40px 44px', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 20 }}>
          <div style={{ fontSize: 44, marginBottom: 4 }}>🔑</div>
          <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 22, fontWeight: 800, textAlign: 'center' }}>Verify your email</div>
          <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14, textAlign: 'center', lineHeight: 1.6 }}>
            A one-time code was sent to <span style={{ color: theme.text, fontWeight: 600 }}>reviewer@client.com</span>
          </div>
          {/* OTP boxes */}
          <div style={{ display: 'flex', gap: 10, marginTop: 8 }}>
            {OTP_DIGITS.map((d, i) => {
              const show = frame >= OTP_START + i * 8;
              const digitS = spring({ frame: frame - (OTP_START + i * 8), fps, config: { damping: 200 } });
              return (
                <div key={i} style={{ opacity: show ? digitS : 0, transform: `scale(${show ? interpolate(digitS, [0, 1], [0.6, 1]) : 0})`, width: 52, height: 64, borderRadius: 12, background: theme.surface2, border: `2px solid ${theme.accent}`, display: 'grid', placeItems: 'center', color: theme.text, fontFamily: theme.mono, fontSize: 28, fontWeight: 800, boxShadow: `0 0 0 3px ${theme.accent}22` }}>
                  {show ? d : ''}
                </div>
              );
            })}
          </div>
          <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 13 }}>Code expires in 5:00</div>
        </div>
      ) : (
        <div style={{ opacity: successS, transform: `scale(${interpolate(successS, [0, 1], [0.85, 1])})`, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 16 }}>
          <div style={{ width: 80, height: 80, borderRadius: '50%', background: `${theme.accent2}22`, border: `2px solid ${theme.accent2}`, display: 'grid', placeItems: 'center', fontSize: 38 }}>✓</div>
          <div style={{ color: theme.accent2, fontFamily: theme.font, fontSize: 28, fontWeight: 800 }}>Verified</div>
          <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 16 }}>Opening session…</div>
        </div>
      )}

      <Caption step={3} title="Email-OTP gate" sub="Guest verifies identity before accessing the shared session" delay={60} />
    </div>
  );
};

// ─── Scene 4 – Remote / mobile overview ──────────────────────────────────────
const Scene4Remote: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const tiles = [
    { icon: '☁️', label: 'Cloudflare Tunnel',   desc: 'Expose Otto securely — no port forwarding, no VPN',            color: '#f6821f' },
    { icon: '📱', label: 'PWA / Mobile',          desc: 'Install Otto as an app on your phone — full touch terminal',  color: theme.accent },
    { icon: '🔐', label: 'Scoped share links',    desc: 'Read-only or write — expiring, OTP-gated, revocable',        color: '#bf7aff' },
    { icon: '👥', label: 'Multi-user RBAC',       desc: 'Granular per-feature grants — own, admin, edit, view',        color: theme.accent2 },
  ];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}>
      <Appear delay={4}>
        <div style={{ padding: '22px 28px 0' }}>
          <div style={{ color: theme.text, fontFamily: theme.font, fontSize: 26, fontWeight: 800 }}>Remote & Mobile Access</div>
          <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 15, marginTop: 4 }}>Work from anywhere — securely share with your team</div>
        </div>
      </Appear>
      <div style={{ flex: 1, padding: '24px 28px', display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 18, overflow: 'hidden' }}>
        {tiles.map((tile, i) => {
          const s = spring({ frame: frame - (i * 12 + 18), fps, config: { damping: 180 } });
          return (
            <div key={tile.label} style={{ opacity: s, transform: `scale(${interpolate(s, [0, 1], [0.88, 1])})`, background: theme.surface2, borderRadius: 16, border: `1px solid ${theme.border}`, padding: '26px 28px', display: 'flex', flexDirection: 'column', gap: 14 }}>
              <div style={{ fontSize: 34 }}>{tile.icon}</div>
              <div>
                <div style={{ color: tile.color, fontFamily: theme.font, fontSize: 18, fontWeight: 800, marginBottom: 6 }}>{tile.label}</div>
                <div style={{ color: theme.textDim, fontFamily: theme.font, fontSize: 14, lineHeight: 1.55 }}>{tile.desc}</div>
              </div>
            </div>
          );
        })}
      </div>

      <Caption step={4} title="Remote access + PWA" sub="Cloudflare tunnel + install-as-app — Otto in your pocket" delay={50} />
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
      <div style={{ opacity: t1, transform: `scale(${interpolate(t1, [0, 1], [0.5, 1])})`, fontSize: 80 }}>🔗</div>
      <div style={{ opacity: t2, transform: `translateY(${interpolate(t2, [0, 1], [24, 0])}px)`, color: theme.text, fontFamily: theme.font, fontSize: 64, fontWeight: 800, textAlign: 'center' }}>
        Collaborate. Securely.
      </div>
      <div style={{ opacity: t3, transform: `translateY(${interpolate(t3, [0, 1], [16, 0])}px)`, color: theme.textDim, fontFamily: theme.font, fontSize: 24, textAlign: 'center' }}>
        RBAC · Share links · OTP gate · Cloudflare tunnel · PWA
      </div>
    </div>
  );
};

// ─── Root composition ─────────────────────────────────────────────────────────
export const Sharing: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: theme.bgGradient, fontFamily: theme.font }}>

      <Sequence durationInFrames={TITLE_DUR}>
        <TitleCard kicker="OTTO ADE" title="Sharing" subtitle="Multi-user RBAC + secure remote access" />
      </Sequence>

      {/* RBAC scene inside window */}
      <Sequence from={S1_START} durationInFrames={S1_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Users & Sharing">
            <Scene1RBAC />
          </OttoWindow>
        </AbsoluteFill>
      </Sequence>

      {/* share link form (modal over window) */}
      <Sequence from={S2_START} durationInFrames={S2_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Users & Sharing">
            <div style={{ position: 'absolute', inset: 0 }} />
          </OttoWindow>
          <Scene2ShareLink />
        </AbsoluteFill>
      </Sequence>

      {/* OTP gate (guest view — full screen, no window chrome) */}
      <Sequence from={S3_START} durationInFrames={S3_DUR}>
        <Scene3OTP />
      </Sequence>

      {/* Remote / mobile tiles inside window */}
      <Sequence from={S4_START} durationInFrames={S4_DUR}>
        <AbsoluteFill style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <OttoWindow title="Otto — Remote Access">
            <Scene4Remote />
          </OttoWindow>
        </AbsoluteFill>
      </Sequence>

      <Sequence from={OUTRO_START} durationInFrames={OUTRO_DUR}>
        <Outro />
      </Sequence>

    </AbsoluteFill>
  );
};
