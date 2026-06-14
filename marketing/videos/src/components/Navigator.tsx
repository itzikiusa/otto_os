import React from 'react';
import { theme } from '../theme';
import { staticFile, Img } from 'remotion';

const Item: React.FC<{ icon: string; label: string; active?: boolean; badge?: string }> = ({
  icon,
  label,
  active,
  badge,
}) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      height: 38,
      padding: '0 12px',
      margin: '0 8px',
      borderRadius: 9,
      background: active ? 'rgba(61,91,255,0.14)' : 'transparent',
      color: active ? theme.text : theme.textDim,
      fontFamily: theme.font,
      fontSize: 15,
      fontWeight: active ? 600 : 500,
    }}
  >
    <span style={{ fontSize: 16, width: 18, textAlign: 'center' }}>{icon}</span>
    <span style={{ flex: 1 }}>{label}</span>
    {badge && (
      <span
        style={{
          fontSize: 11,
          fontWeight: 700,
          color: theme.bg,
          background: theme.working,
          borderRadius: 8,
          padding: '1px 7px',
        }}
      >
        {badge}
      </span>
    )}
  </div>
);

const Section: React.FC<{ label: string }> = ({ label }) => (
  <div
    style={{
      color: theme.textDim,
      fontFamily: theme.font,
      fontSize: 11,
      fontWeight: 600,
      letterSpacing: 1,
      textTransform: 'uppercase',
      padding: '14px 20px 6px',
    }}
  >
    {label}
  </div>
);

export const Navigator: React.FC<{
  active?: 'agents' | 'connections' | 'git';
  sessions?: { title: string; provider: string; status?: 'working' | 'idle' }[];
}> = ({ active = 'agents', sessions = [] }) => (
  <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 9, padding: '14px 18px' }}>
      <Img src={staticFile('otto-mark.png')} style={{ width: 22, height: 22, borderRadius: 6 }} />
      <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 16, fontWeight: 700 }}>Otto</span>
    </div>
    <Item icon="▸_" label="Agents" active={active === 'agents'} badge={sessions.some((s) => s.status === 'working') ? '•' : undefined} />
    {sessions.length > 0 && (
      <div style={{ paddingLeft: 14 }}>
        {sessions.map((s, i) => (
          <div
            key={i}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              height: 30,
              padding: '0 14px',
              color: theme.textDim,
              fontFamily: theme.font,
              fontSize: 13,
            }}
          >
            <span
              style={{
                width: 8,
                height: 8,
                borderRadius: '50%',
                background: s.status === 'working' ? theme.working : theme.idle,
              }}
            />
            <span style={{ flex: 1, color: theme.text }}>{s.title}</span>
            <span style={{ fontSize: 11 }}>{s.provider}</span>
          </div>
        ))}
      </div>
    )}
    <Item icon="🔌" label="Connections" active={active === 'connections'} />
    <Item icon="⎇" label="Git" active={active === 'git'} />
    <Section label="Workspaces" />
    <Item icon="📁" label="sinatra-users-go" />
    <Item icon="📁" label="admission" />
    <div style={{ flex: 1 }} />
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 9,
        padding: '12px 18px',
        borderTop: `1px solid ${theme.border}`,
        color: theme.textDim,
        fontFamily: theme.font,
        fontSize: 13,
      }}
    >
      <span style={{ fontSize: 15 }}>⚙</span> Settings
    </div>
  </div>
);
