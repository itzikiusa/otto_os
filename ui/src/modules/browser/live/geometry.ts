// Remote live browser — pure geometry. No imports (the unit tests load this
// file straight into node), no DOM access: every function takes plain numbers.
//
// Three coordinate spaces meet here:
//   • CLIENT  — the viewer's CSS px (`PointerEvent.clientX/Y`, the panel's
//               `getBoundingClientRect()`).
//   • IMAGE   — pixels of the last screencast JPEG (`naturalWidth/Height`).
//               Chromium may send it at device pixels (css × dpr) or
//               downscaled, so it is never assumed to equal the viewport.
//   • PAGE    — the remote page's viewport in CSS px, which is what the input
//               messages carry (CDP `Input.dispatch*Event` x/y). The frame's
//               own metadata (`device_width/height`) says how big that is.
//
// The frame is drawn CONTAINED in the panel (letterboxed while a resize is
// in flight and the remote hasn't caught up), so a click on a bar outside the
// picture maps to nothing rather than to a clamped edge pixel.

export interface Size {
  width: number;
  height: number;
}

export interface Box extends Size {
  left: number;
  top: number;
}

/** Where a `content`-sized picture lands when contained (aspect-fit,
 *  centred) inside a `box`-sized area. `scale` is box px per content px. */
export interface Fit extends Size {
  x: number;
  y: number;
  scale: number;
}

/** The subset of a frame's metadata the mapping needs (mirrors CDP
 *  `Page.ScreencastFrameMetadata`, snake-cased by the daemon). */
export interface FrameGeometry {
  /** The remote viewport width in CSS px. */
  device_width: number;
  /** The remote viewport height in CSS px. */
  device_height: number;
  /** Top offset of the page content inside the image, in CSS px (0 on
   *  headless Chromium; non-zero only with a mobile top-controls bar). */
  offset_top?: number;
}

export interface Point {
  x: number;
  y: number;
}

/** Aspect-fit `content` inside `box`, centred. A degenerate size yields an
 *  empty fit at the box origin (nothing drawn, nothing clickable). */
export function fitContain(content: Size, box: Size): Fit {
  if (content.width <= 0 || content.height <= 0 || box.width <= 0 || box.height <= 0) {
    return { x: 0, y: 0, width: 0, height: 0, scale: 0 };
  }
  const scale = Math.min(box.width / content.width, box.height / content.height);
  const width = content.width * scale;
  const height = content.height * scale;
  return { x: (box.width - width) / 2, y: (box.height - height) / 2, width, height, scale };
}

/**
 * Map a client-space point to the remote page viewport (CSS px).
 *
 * `box` is the element the frame is drawn in (client space), `image` the
 * decoded frame's pixel size, `frame` its metadata. Returns null when the
 * point falls on the letterbox bars or nothing has been drawn yet — the
 * caller then simply doesn't send the event.
 */
export function clientToPage(client: Point, box: Box, image: Size, frame: FrameGeometry): Point | null {
  const fit = fitContain(image, box);
  if (fit.scale === 0 || frame.device_width <= 0 || frame.device_height <= 0) return null;
  const ix = (client.x - box.left - fit.x) / fit.scale; // image px
  const iy = (client.y - box.top - fit.y) / fit.scale;
  if (ix < 0 || iy < 0 || ix > image.width || iy > image.height) return null;
  const cssPerImagePx = frame.device_width / image.width;
  const x = ix * cssPerImagePx;
  const y = iy * cssPerImagePx - (frame.offset_top ?? 0);
  if (y < 0 || y > frame.device_height) return null;
  // Round to 1/100 px — CDP accepts fractional coordinates, but a stable
  // representation keeps the wire (and the tests) free of float noise.
  return { x: Math.round(x * 100) / 100, y: Math.round(y * 100) / 100 };
}

/** The inverse of `clientToPage`, for drawing things the server reports in
 *  page coordinates (the agent's ghost cursor, a highlighted target) over the
 *  frame. Returns coordinates relative to the box's top-left. */
export function pageToBox(page: Point, box: Size, image: Size, frame: FrameGeometry): Point | null {
  const fit = fitContain(image, box);
  if (fit.scale === 0 || frame.device_width <= 0) return null;
  const imagePxPerCss = image.width / frame.device_width;
  const ix = page.x * imagePxPerCss;
  const iy = (page.y + (frame.offset_top ?? 0)) * imagePxPerCss;
  return { x: fit.x + ix * fit.scale, y: fit.y + iy * fit.scale };
}

export interface ViewportRequest extends Size {
  /** Device scale factor to render at (so text stays crisp on Retina). */
  device_scale_factor: number;
}

export interface ViewportLimits {
  min: Size;
  max: Size;
  /** Cap on encoded pixels per frame (width × height × dsf²): a 5K display
   *  at 2× would otherwise ask Chromium for ~30 MP JPEGs. */
  maxPixels: number;
}

export const DEFAULT_VIEWPORT_LIMITS: ViewportLimits = {
  min: { width: 240, height: 160 },
  max: { width: 3840, height: 2400 },
  maxPixels: 3840 * 2400,
};

/**
 * The viewport to ask the remote for so it fills a `panel`-sized area 1:1 at
 * the viewer's `dpr`. Width/height are integer CSS px clamped to the limits;
 * the scale factor is quantised to 0.25 steps (so a fractional browser zoom
 * doesn't trigger a resize storm) and lowered until the pixel budget holds.
 */
export function viewportFor(panel: Size, dpr: number, limits: ViewportLimits = DEFAULT_VIEWPORT_LIMITS): ViewportRequest {
  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
  const width = Math.round(clamp(panel.width, limits.min.width, limits.max.width));
  const height = Math.round(clamp(panel.height, limits.min.height, limits.max.height));
  let dsf = clamp(Math.round((Number.isFinite(dpr) && dpr > 0 ? dpr : 1) * 4) / 4, 1, 3);
  while (dsf > 1 && width * height * dsf * dsf > limits.maxPixels) dsf -= 0.25;
  return { width, height, device_scale_factor: dsf };
}

/** True when two viewport requests differ enough to be worth a round-trip
 *  (a ±1 px wobble from sub-pixel layout is not). */
export function viewportChanged(a: ViewportRequest | null, b: ViewportRequest): boolean {
  if (!a) return true;
  return (
    Math.abs(a.width - b.width) > 1 ||
    Math.abs(a.height - b.height) > 1 ||
    a.device_scale_factor !== b.device_scale_factor
  );
}
