// Shared canvas export helpers — SVG→PNG download + copy-source-to-clipboard.
// Used by both MermaidCanvas and D2Canvas zoombars (their preview is always an
// inline <svg>, regardless of which renderer produced it).

/** Rasterize an on-screen <svg> to a PNG and trigger a browser download.
 *  Clones the node (so it renders at its natural size, unrelated to any pan/zoom
 *  transform applied to the live element) and draws it onto an offscreen canvas
 *  at `scale`× the SVG's viewBox size. Best-effort: silently no-ops on failure
 *  (canvas 2d context unavailable, blob creation failure, …) — there's no good
 *  place to surface an error from an image `onload` callback here; the caller's
 *  button stays clickable either way. */
export function svgToPngDownload(svgEl: SVGSVGElement, filename: string, scale = 2): void {
  const vb = svgEl.viewBox?.baseVal;
  const w = (vb && vb.width) || svgEl.getBoundingClientRect().width || 800;
  const h = (vb && vb.height) || svgEl.getBoundingClientRect().height || 600;

  const clone = svgEl.cloneNode(true) as SVGSVGElement;
  clone.setAttribute('width', String(w));
  clone.setAttribute('height', String(h));
  clone.style.maxWidth = '';
  clone.style.width = '';
  clone.style.height = '';
  const xml = new XMLSerializer().serializeToString(clone);
  const dataUrl = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(xml)}`;

  const img = new Image();
  img.onload = () => {
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(w * scale));
    canvas.height = Math.max(1, Math.round(h * scale));
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    // A white backing so the PNG isn't transparent (diagrams read on either
    // theme, but a downloaded image should look right dropped into a doc/slide).
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    canvas.toBlob((blob) => {
      if (!blob) return;
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = filename;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    }, 'image/png');
  };
  img.src = dataUrl;
}
