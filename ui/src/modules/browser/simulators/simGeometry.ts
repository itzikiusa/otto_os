// Pure coordinate-mapping helpers for the embedded SimulatorView.
// Unit-tested directly under `ui/unit/`.

import type { SimDevice } from '../../../lib/api/types';
import { fitContain, type Box, type Point, type Size } from '../live/geometry.ts';

export type SimPoint = Point;
export type SimSize = Size;
export type SimBox = Box;

/** Map a client-space pointer point onto normalized [0..1] simulator screen
 *  coordinates, returning null when the click falls on the letterbox bars. */
export function clientToSimNormalized(
  client: SimPoint,
  box: SimBox,
  image: SimSize,
): SimPoint | null {
  const fit = fitContain(image, box);
  if (fit.scale === 0) return null;
  const rx = (client.x - box.left - fit.x) / fit.width;
  const ry = (client.y - box.top - fit.y) / fit.height;
  if (rx < 0 || ry < 0 || rx > 1 || ry > 1) return null;
  return {
    x: Math.round(rx * 10_000) / 10_000,
    y: Math.round(ry * 10_000) / 10_000,
  };
}

/** Human subtitle for a simulator/emulator row. */
export function simDeviceSubtitle(d: SimDevice): string {
  const parts: string[] = [];
  if (d.os_version) parts.push(d.os_version);
  else parts.push(d.platform === 'ios' ? 'iOS' : 'Android');
  if (d.form_factor && d.form_factor !== 'other') {
    parts.push(d.form_factor[0].toUpperCase() + d.form_factor.slice(1));
  }
  if (d.screen && d.screen.width > 0 && d.screen.height > 0) {
    parts.push(`${d.screen.width}×${d.screen.height}`);
  }
  if (d.is_physical) parts.push('USB');
  return parts.join(' · ');
}
