import React from 'react';
import { T, Theme, fonts, traffic, radius, alpha, status as STATUS } from '../theme';
import { Icon } from './Icon';
import { OttoGlyph } from './OttoLogo';

// ════════════════════════════════════════════════════════════════════════════
//  DESKTOP — macOS window with Otto's overlay titlebar + shell split
// ════════════════════════════════════════════════════════════════════════════

export const OttoWindow: React.FC<{
  nav?: React.ReactNode;
  right?: React.ReactNode;
  tabs?: { label: string; icon?: string; active?: boolean; dot?: keyof typeof STATUS }[];
  title?: string;
  t?: Theme;
  width?: number;
  height?: number;
  children: React.ReactNode;
  contentStyle?: React.CSSProperties;
  style?: React.CSSProperties;
}> = ({ nav, right, tabs, title = 'Otto', t = T, width = 1560, height = 884, children, contentStyle, style }) => (
  <div
    style={{
      width,
      height,
      borderRadius: 14,
      overflow: 'hidden',
      background: t.bg,
      border: `1px solid ${t.border}`,
      boxShadow: `0 50px 130px rgba(0,0,0,0.6), 0 0 0 1px ${alpha('#fff', 0.03)}`,
      display: 'flex',
      flexDirection: 'column',
      ...style,
    }}
  >
    {/* titlebar */}
    <div
      style={{
        height: 44,
        display: 'flex',
        alignItems: 'center',
        padding: '0 16px',
        gap: 9,
        background: t.bgSidebar,
        borderBottom: `1px solid ${t.border}`,
        flexShrink: 0,
        position: 'relative',
      }}
    >
      <Light color={traffic.close} />
      <Light color={traffic.min} />
      <Light color={traffic.max} />
      <div
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          textAlign: 'center',
          fontFamily: fonts.ui,
          fontSize: 13,
          fontWeight: 600,
          color: t.textDim,
          pointerEvents: 'none',
        }}
      >
        {title}
      </div>
    </div>
    {/* body */}
    <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
      {nav}
      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', background: t.bg }}>
        {tabs && <TabBar tabs={tabs} t={t} />}
        <div style={{ flex: 1, minHeight: 0, position: 'relative', overflow: 'hidden', ...contentStyle }}>{children}</div>
      </div>
      {right}
    </div>
  </div>
);

export const TabBar: React.FC<{
  tabs: { label: string; icon?: string; active?: boolean; dot?: keyof typeof STATUS }[];
  t?: Theme;
}> = ({ tabs, t = T }) => (
  <div
    style={{
      height: 38,
      display: 'flex',
      alignItems: 'center',
      gap: 4,
      padding: '0 8px',
      borderBottom: `1px solid ${t.border}`,
      background: t.bg,
      flexShrink: 0,
    }}
  >
    {tabs.map((tab, i) => (
      <div
        key={i}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 7,
          height: 28,
          padding: '0 12px',
          borderRadius: radius.s,
          background: tab.active ? t.surface : 'transparent',
          border: `1px solid ${tab.active ? t.border : 'transparent'}`,
          color: tab.active ? t.text : t.textDim,
          fontFamily: fonts.ui,
          fontSize: 12.5,
          fontWeight: tab.active ? 600 : 500,
        }}
      >
        {tab.dot && <span style={{ width: 7, height: 7, borderRadius: '50%', background: STATUS[tab.dot] }} />}
        {tab.icon && <Icon name={tab.icon} size={13} />}
        {tab.label}
      </div>
    ))}
  </div>
);

const Light: React.FC<{ color: string }> = ({ color }) => (
  <div style={{ width: 13, height: 13, borderRadius: '50%', background: color }} />
);

// A right-hand panel column (Git / files / notes / activity), like the real shell.
export const RightPanel: React.FC<{ t?: Theme; width?: number; title?: string; icon?: string; children: React.ReactNode }> = ({
  t = T,
  width = 320,
  title,
  icon,
  children,
}) => (
  <div
    style={{
      width,
      flexShrink: 0,
      background: t.bgSidebar,
      borderLeft: `1px solid ${t.border}`,
      display: 'flex',
      flexDirection: 'column',
      minHeight: 0,
    }}
  >
    {title && (
      <div
        style={{
          height: 38,
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '0 14px',
          borderBottom: `1px solid ${t.border}`,
          fontFamily: fonts.ui,
          fontSize: 12.5,
          fontWeight: 600,
          color: t.text,
          flexShrink: 0,
        }}
      >
        {icon && <Icon name={icon} size={14} color={t.textDim} />}
        {title}
      </div>
    )}
    <div style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>{children}</div>
  </div>
);

// ════════════════════════════════════════════════════════════════════════════
//  MOBILE — iPhone shell (top bar + content + bottom nav)
// ════════════════════════════════════════════════════════════════════════════

const PHONE_NAV = [
  { id: 'agents', icon: 'terminal', label: 'Agents' },
  { id: 'swarm', icon: 'grid', label: 'Swarm' },
  { id: 'connections', icon: 'plug', label: 'Connections' },
  { id: 'git', icon: 'branch', label: 'Git' },
  { id: 'more', icon: 'command', label: 'More' },
];

export const PhoneFrame: React.FC<{
  t?: Theme;
  title?: string;
  active?: string; // bottom-nav id
  time?: string;
  height?: number;
  children: React.ReactNode;
  workingBadge?: number;
  showPanelBtn?: boolean;
}> = ({ t = T, title = 'Agents', active = 'agents', time = '9:41', height = 880, children, workingBadge, showPanelBtn }) => {
  const w = Math.round(height * 0.486); // ~iPhone aspect incl. bezel
  const topbar = 44;
  const bottomnav = 60;
  return (
    <div
      style={{
        width: w,
        height,
        borderRadius: 56,
        padding: 13,
        background: 'linear-gradient(160deg,#2b2b30,#0c0c10)',
        boxShadow: `0 40px 110px rgba(0,0,0,0.6), inset 0 0 0 2px ${alpha('#fff', 0.06)}`,
        flexShrink: 0,
      }}
    >
      <div
        style={{
          width: '100%',
          height: '100%',
          borderRadius: 44,
          overflow: 'hidden',
          background: t.bg,
          display: 'flex',
          flexDirection: 'column',
          position: 'relative',
        }}
      >
        {/* status bar + dynamic island */}
        <div
          style={{
            height: 40,
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: '0 26px',
            background: t.bgSidebar,
            fontFamily: fonts.ui,
            fontSize: 14,
            fontWeight: 600,
            color: t.text,
          }}
        >
          <span>{time}</span>
          <div
            style={{
              position: 'absolute',
              left: '50%',
              top: 9,
              transform: 'translateX(-50%)',
              width: 92,
              height: 24,
              borderRadius: 14,
              background: '#000',
            }}
          />
          <span style={{ display: 'flex', gap: 5, alignItems: 'center', color: t.text }}>
            <span style={{ fontSize: 12 }}>5G</span>
            <span style={{ width: 22, height: 11, borderRadius: 3, border: `1px solid ${t.textDim}`, position: 'relative' }}>
              <span style={{ position: 'absolute', inset: 1.5, width: '70%', background: t.text, borderRadius: 1 }} />
            </span>
          </span>
        </div>
        {/* top bar */}
        <div
          style={{
            height: topbar,
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '0 12px',
            borderBottom: `1px solid ${t.border}`,
            background: t.bgSidebar,
          }}
        >
          <div style={{ width: 30, height: 30, borderRadius: 8, background: t.surface2, display: 'grid', placeItems: 'center', color: t.textDim }}>
            <Icon name="sidebar" size={16} />
          </div>
          <div style={{ flex: 1, textAlign: 'center', fontFamily: fonts.ui, fontSize: 16, fontWeight: 700, color: t.text }}>
            {title}
          </div>
          <div style={{ width: 30, height: 30, borderRadius: 8, background: t.surface2, display: 'grid', placeItems: 'center', color: t.textDim }}>
            <Icon name={showPanelBtn ? 'panel' : 'plus'} size={16} />
          </div>
        </div>
        {/* content */}
        <div style={{ flex: 1, minHeight: 0, position: 'relative', overflow: 'hidden' }}>{children}</div>
        {/* bottom nav */}
        <div
          style={{
            height: bottomnav,
            flexShrink: 0,
            display: 'flex',
            borderTop: `1px solid ${t.border}`,
            background: t.bgSidebar,
            paddingBottom: 6,
          }}
        >
          {PHONE_NAV.map((m) => {
            const on = m.id === active;
            return (
              <div key={m.id} style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 3, color: on ? t.accent : t.textDim }}>
                <div style={{ position: 'relative' }}>
                  <Icon name={m.icon} size={21} />
                  {m.id === 'agents' && workingBadge ? (
                    <span style={{ position: 'absolute', top: -5, right: -9, minWidth: 15, height: 15, padding: '0 3px', borderRadius: 999, background: STATUS.working, color: '#fff', fontFamily: fonts.ui, fontSize: 9, fontWeight: 700, display: 'grid', placeItems: 'center' }}>
                      {workingBadge}
                    </span>
                  ) : null}
                </div>
                <span style={{ fontFamily: fonts.ui, fontSize: 10.5, fontWeight: 500 }}>{m.label}</span>
              </div>
            );
          })}
        </div>
        {/* home indicator */}
        <div style={{ position: 'absolute', bottom: 7, left: '50%', transform: 'translateX(-50%)', width: 128, height: 5, borderRadius: 999, background: alpha(t.text, 0.5) }} />
      </div>
    </div>
  );
};

// ════════════════════════════════════════════════════════════════════════════
//  TABLET — iPad shell (persistent narrow navigator + content)
// ════════════════════════════════════════════════════════════════════════════

export const TabletFrame: React.FC<{
  t?: Theme;
  nav?: React.ReactNode;
  title?: string;
  height?: number;
  time?: string;
  children: React.ReactNode;
}> = ({ t = T, nav, title = 'Otto', height = 880, time = '9:41', children }) => {
  const w = Math.round(height * 1.33);
  return (
    <div
      style={{
        width: w,
        height,
        borderRadius: 38,
        padding: 16,
        background: 'linear-gradient(160deg,#2b2b30,#0c0c10)',
        boxShadow: `0 44px 120px rgba(0,0,0,0.6), inset 0 0 0 2px ${alpha('#fff', 0.06)}`,
        flexShrink: 0,
      }}
    >
      <div style={{ width: '100%', height: '100%', borderRadius: 24, overflow: 'hidden', background: t.bg, display: 'flex', flexDirection: 'column' }}>
        <div
          style={{
            height: 32,
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: '0 18px',
            background: t.bgSidebar,
            fontFamily: fonts.ui,
            fontSize: 12.5,
            fontWeight: 600,
            color: t.text,
          }}
        >
          <span style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
            <OttoGlyph size={13} glow={false} />
            {title}
          </span>
          <span>{time}</span>
        </div>
        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          {nav}
          <div style={{ flex: 1, minWidth: 0, position: 'relative', overflow: 'hidden', background: t.bg }}>{children}</div>
        </div>
      </div>
    </div>
  );
};
