// User wallpaper for the ambient backdrop (Settings → Appearance). The photo
// never leaves the device: it is decoded here, downscaled, blurred, and
// luminance-clamped into BOTH scheme bands (lib/ambient.ts `clampPixels`), and
// the two small JPEG data URLs are what gets stored (ui.setAmbientPhoto). The
// clamp is what lets chrome glass keep AA contrast over any photo.

import { clampPixels, type AmbientScheme } from './ambient';

/** Long edge of the stored image. It is shown blurred and full-bleed, so a
 *  small raster is indistinguishable from a big one — and fits localStorage. */
const EDGE = 720;
const BLUR_PX = 16;
/** Refuse absurd inputs before decoding them. */
export const MAX_WALLPAPER_BYTES = 25 * 1024 * 1024;

export async function processWallpaper(file: Blob): Promise<{ light: string; dark: string }> {
  if (!file.type.startsWith('image/')) throw new Error('Choose an image file (PNG, JPEG, HEIC, WebP…).');
  if (file.size > MAX_WALLPAPER_BYTES) throw new Error('That image is over 25 MB — pick a smaller one.');
  const bitmap = await createImageBitmap(file);
  try {
    const scale = Math.min(1, EDGE / Math.max(bitmap.width, bitmap.height));
    const w = Math.max(1, Math.round(bitmap.width * scale));
    const h = Math.max(1, Math.round(bitmap.height * scale));
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    if (!ctx) throw new Error('This browser cannot process images.');
    // Overdraw past the edges so the blur doesn't fade the border to clear.
    ctx.filter = `blur(${BLUR_PX}px)`;
    const pad = BLUR_PX * 2;
    ctx.drawImage(bitmap, -pad, -pad, w + pad * 2, h + pad * 2);
    ctx.filter = 'none';
    const src = ctx.getImageData(0, 0, w, h);
    const variant = (scheme: AmbientScheme): string => {
      const px = new ImageData(new Uint8ClampedArray(src.data), w, h);
      clampPixels(px.data, scheme);
      ctx.putImageData(px, 0, 0);
      return canvas.toDataURL('image/jpeg', 0.82);
    };
    return { light: variant('light'), dark: variant('dark') };
  } finally {
    bitmap.close();
  }
}
