import React from 'react';

/** Otto's app icon, redrawn as vector (the shipped PNGs are too small to scale). */
export const Logo: React.FC<{ size: number; glow?: number }> = ({ size, glow = 0 }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 100 100"
    style={{ filter: glow ? `drop-shadow(0 0 ${glow}px rgba(61,90,254,0.75))` : undefined, overflow: 'visible' }}
  >
    <defs>
      <linearGradient id="og" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stopColor="#4b6bff" />
        <stop offset="1" stopColor="#3350f0" />
      </linearGradient>
    </defs>
    <rect x="4" y="4" width="92" height="92" rx="22" fill="url(#og)" />
    <rect x="4" y="4" width="92" height="92" rx="22" fill="none" stroke="rgba(255,255,255,0.18)" strokeWidth="0.8" />
    <circle cx="50" cy="50" r="29" fill="#fff" />
    <circle cx="50" cy="50" r="19" fill="#0c1428" />
    <path d="M42.5 43.5 L49 50 L42.5 56.5" fill="none" stroke="#fff" strokeWidth="3.4" strokeLinecap="round" strokeLinejoin="round" />
    <line x1="51.5" y1="58" x2="59" y2="58" stroke="#fff" strokeWidth="3.4" strokeLinecap="round" />
    <path d="M17.5 43 L11 50 L17.5 57" fill="none" stroke="#fff" strokeWidth="3.6" strokeLinecap="round" strokeLinejoin="round" />
    <path d="M82.5 43 L89 50 L82.5 57" fill="none" stroke="#fff" strokeWidth="3.6" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);
