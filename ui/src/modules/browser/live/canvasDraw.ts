// Aspect-fit (letterboxed) ImageBitmap draw onto a DPR-scaled canvas.
export function drawContainBitmap(
  canvas: HTMLCanvasElement | null,
  bitmap: ImageBitmap | null,
  box: { width: number; height: number },
): void {
  if (!canvas || !bitmap || box.width <= 0 || box.height <= 0) return;
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(box.width * dpr));
  const h = Math.max(1, Math.round(box.height * dpr));
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  ctx.clearRect(0, 0, w, h);
  const scale = Math.min(w / bitmap.width, h / bitmap.height);
  const dw = bitmap.width * scale;
  const dh = bitmap.height * scale;
  ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(bitmap, (w - dw) / 2, (h - dh) / 2, dw, dh);
}
