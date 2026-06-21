import React from 'react';
import { brand } from '../theme';

// The real Otto mark — the angular "lightning Z" from ui/public/favicon.svg
// (viewBox 0 0 48 46). We render the authentic outer path with the brand
// purple→violet gradient + cyan glints, so it's crisp at any size (no PNG).
const MARK_PATH =
  'M25.946 44.938c-.664.845-2.021.375-2.021-.698V33.937a2.26 2.26 0 0 0-2.262-2.262H10.287' +
  'c-.92 0-1.456-1.04-.92-1.788l7.48-10.471c1.07-1.497 0-3.578-1.842-3.578H1.237' +
  'c-.92 0-1.456-1.04-.92-1.788L10.013.474c.214-.297.556-.474.92-.474h28.894' +
  'c.92 0 1.456 1.04.92 1.788l-7.48 10.471c-1.07 1.498 0 3.579 1.842 3.579h11.377' +
  'c.943 0 1.473 1.088.89 1.83L25.947 44.94z';

/** The bare Otto glyph (purple→cyan gradient, soft glow). */
export const OttoGlyph: React.FC<{ size?: number; glow?: boolean; idSuffix?: string }> = ({
  size = 64,
  glow = true,
  idSuffix = 'g',
}) => {
  const w = size;
  const h = (size * 46) / 48;
  const gid = `otto-grad-${idSuffix}`;
  return (
    <svg
      width={w}
      height={h}
      viewBox="0 0 48 46"
      fill="none"
      style={{ display: 'block', filter: glow ? `drop-shadow(0 6px 26px ${brand.glow}aa)` : undefined }}
    >
      <defs>
        <linearGradient id={gid} x1="2" y1="2" x2="44" y2="44" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#a06bff" />
          <stop offset="0.5" stopColor={brand.purple} />
          <stop offset="1" stopColor={brand.violet} />
        </linearGradient>
        <radialGradient id={`${gid}-glint`} cx="0.78" cy="0.7" r="0.5">
          <stop offset="0" stopColor={brand.cyan} stopOpacity="0.9" />
          <stop offset="1" stopColor={brand.cyan} stopOpacity="0" />
        </radialGradient>
      </defs>
      <path d={MARK_PATH} fill={`url(#${gid})`} />
      {/* cyan glint, clipped to the mark */}
      <clipPath id={`${gid}-clip`}>
        <path d={MARK_PATH} />
      </clipPath>
      <g clipPath={`url(#${gid}-clip)`}>
        <ellipse cx="38" cy="30" rx="16" ry="22" fill={`url(#${gid}-glint)`} />
        <path d={MARK_PATH} fill="none" stroke="#ffffff" strokeOpacity="0.18" strokeWidth="0.8" />
      </g>
    </svg>
  );
};

/**
 * The Otto mark on a rounded "squircle" app-icon tile (macOS dock look) — for
 * hero / title / outro shots. Glassy purple plate with the glyph centered.
 */
export const OttoIcon: React.FC<{ size?: number; glowPx?: number; idSuffix?: string }> = ({
  size = 140,
  glowPx = 80,
  idSuffix = 'app',
}) => (
  <div
    style={{
      width: size,
      height: size,
      borderRadius: size * 0.235,
      display: 'grid',
      placeItems: 'center',
      background: 'linear-gradient(150deg, #2a1860 0%, #14101f 92%)',
      boxShadow:
        `inset 0 1px 0 rgba(255,255,255,0.18), inset 0 0 0 1px rgba(255,255,255,0.06),` +
        `0 24px ${glowPx}px ${brand.glow}66, 0 8px 24px rgba(0,0,0,0.55)`,
      position: 'relative',
      overflow: 'hidden',
    }}
  >
    {/* top sheen */}
    <div
      style={{
        position: 'absolute',
        inset: 0,
        background:
          'radial-gradient(120% 80% at 50% -20%, rgba(160,107,255,0.45) 0%, rgba(160,107,255,0) 60%)',
      }}
    />
    <div style={{ position: 'relative', transform: `translateY(${size * 0.01}px)` }}>
      <OttoGlyph size={size * 0.56} glow={false} idSuffix={idSuffix} />
    </div>
  </div>
);
