import React from 'react';
import { theme } from '../theme';
import { staticFile, Img } from 'remotion';

/**
 * A macOS-style app window frame with Otto's overlay titlebar (traffic lights +
 * mark). Pass `sidebar` and `children` for the navigator + content split, or
 * just `children` for a full-bleed content area.
 */
export const OttoWindow: React.FC<{
  sidebar?: React.ReactNode;
  children: React.ReactNode;
  title?: string;
  style?: React.CSSProperties;
}> = ({ sidebar, children, title = 'Otto', style }) => {
  return (
    <div
      style={{
        width: 1560,
        height: 880,
        borderRadius: 16,
        overflow: 'hidden',
        background: theme.surface,
        border: `1px solid ${theme.border}`,
        boxShadow: '0 40px 120px rgba(0,0,0,0.55), 0 0 0 1px rgba(255,255,255,0.02)',
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
          padding: '0 18px',
          gap: 10,
          background: 'rgba(255,255,255,0.015)',
          borderBottom: `1px solid ${theme.border}`,
          flexShrink: 0,
        }}
      >
        <Light color="#ff5f57" />
        <Light color="#febc2e" />
        <Light color="#28c840" />
        <div style={{ width: 14 }} />
        <Img src={staticFile('otto-mark.png')} style={{ width: 20, height: 20, borderRadius: 5 }} />
        <span style={{ color: theme.text, fontFamily: theme.font, fontSize: 14, fontWeight: 600 }}>
          {title}
        </span>
      </div>
      {/* body */}
      <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
        {sidebar ? (
          <div
            style={{
              width: 248,
              background: theme.surface,
              borderRight: `1px solid ${theme.border}`,
              flexShrink: 0,
            }}
          >
            {sidebar}
          </div>
        ) : null}
        <div style={{ flex: 1, minWidth: 0, background: theme.bg, position: 'relative' }}>
          {children}
        </div>
      </div>
    </div>
  );
};

const Light: React.FC<{ color: string }> = ({ color }) => (
  <div style={{ width: 13, height: 13, borderRadius: '50%', background: color }} />
);
