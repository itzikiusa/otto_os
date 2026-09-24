// Crisp flow clips via the Chrome DevTools screencast (JPEG frames at device
// pixels + timestamps) instead of Playwright's low-bitrate recordVideo. Frames
// are re-timed onto a constant 30 fps timeline with ffmpeg.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const FFMPEG = process.env.FFMPEG ?? '/opt/homebrew/bin/ffmpeg';

/**
 * Start recording `page`. Returns `stop(outFile)` which encodes an H.264 mp4
 * (default 2560×1440 so the film can zoom a little without going soft).
 */
export async function record(page, { quality = 88, width = 2560, height = 1440 } = {}) {
  const cdp = await page.context().newCDPSession(page);
  const frames = [];
  cdp.on('Page.screencastFrame', async ({ data, metadata, sessionId }) => {
    frames.push({ buf: Buffer.from(data, 'base64'), ts: metadata.timestamp });
    try {
      await cdp.send('Page.screencastFrameAck', { sessionId });
    } catch {
      /* session closing */
    }
  });
  const vp = page.viewportSize();
  await cdp.send('Page.startScreencast', {
    format: 'jpeg',
    quality,
    maxWidth: vp.width * 2,
    maxHeight: vp.height * 2,
    everyNthFrame: 1,
  });
  const began = Date.now() / 1000;
  return {
    async stop(outFile, { tail = 0.6 } = {}) {
      await cdp.send('Page.stopScreencast');
      const ended = Date.now() / 1000;
      await cdp.detach().catch(() => {});
      if (!frames.length) throw new Error(`[rec] no frames captured for ${outFile}`);
      const dir = mkdtempSync(join(tmpdir(), 'otto-tour-rec-'));
      const lines = [];
      frames.forEach((f, i) => {
        const name = join(dir, `f${String(i).padStart(5, '0')}.jpg`);
        writeFileSync(name, f.buf);
        const next = i + 1 < frames.length ? frames[i + 1].ts : f.ts + Math.max(tail, ended - f.ts);
        lines.push(`file '${name}'`, `duration ${Math.max(0.001, next - f.ts).toFixed(4)}`);
      });
      // concat demuxer needs the last file repeated for its duration to count.
      lines.push(`file '${join(dir, `f${String(frames.length - 1).padStart(5, '0')}.jpg`)}'`);
      writeFileSync(join(dir, 'list.txt'), lines.join('\n'));
      execFileSync('nice', [
        '-n', '10', FFMPEG, '-y', '-v', 'error', '-f', 'concat', '-safe', '0', '-i', join(dir, 'list.txt'),
        '-vf', `fps=30,scale=${width}:${height}:flags=lanczos,format=yuv420p`,
        '-c:v', 'libx264', '-preset', 'medium', '-crf', '18', '-movflags', '+faststart', outFile,
      ]);
      rmSync(dir, { recursive: true, force: true });
      const secs = frames.length ? frames[frames.length - 1].ts - frames[0].ts : 0;
      console.log(`[rec] ${outFile.split('/').pop()}  ${frames.length} frames over ${secs.toFixed(1)}s (started ${(frames[0].ts - began).toFixed(2)}s late)`);
      return outFile;
    },
  };
}

/** A visible macOS-style pointer + click ripple (headless Chrome draws none). */
export const CURSOR_SCRIPT = `
(() => {
  if (window.__tourCursor) return;
  window.__tourCursor = true;
  const install = () => {
    const c = document.createElement('div');
    c.id = '__tour_cursor';
    c.innerHTML = '<svg width="26" height="30" viewBox="0 0 26 30" xmlns="http://www.w3.org/2000/svg"><path d="M3 2 L3 24 L8.6 18.8 L12.4 27.4 L16.2 25.8 L12.5 17.4 L20 17.4 Z" fill="#111" stroke="#fff" stroke-width="1.8" stroke-linejoin="round"/></svg>';
    Object.assign(c.style, { position: 'fixed', left: '0px', top: '0px', zIndex: 2147483647, pointerEvents: 'none', transform: 'translate(-3px,-2px)', filter: 'drop-shadow(0 2px 3px rgba(0,0,0,.35))', transition: 'opacity .2s', opacity: '0' });
    document.documentElement.appendChild(c);
    addEventListener('mousemove', (e) => { c.style.left = e.clientX + 'px'; c.style.top = e.clientY + 'px'; c.style.opacity = '1'; }, true);
    addEventListener('mousedown', (e) => {
      const r = document.createElement('div');
      Object.assign(r.style, { position: 'fixed', left: e.clientX - 18 + 'px', top: e.clientY - 18 + 'px', width: '36px', height: '36px', borderRadius: '50%', border: '2px solid rgba(10,132,255,.9)', background: 'rgba(10,132,255,.18)', zIndex: 2147483646, pointerEvents: 'none', transform: 'scale(.4)', opacity: '1', transition: 'transform .45s ease-out, opacity .45s ease-out' });
      document.documentElement.appendChild(r);
      requestAnimationFrame(() => { r.style.transform = 'scale(1.3)'; r.style.opacity = '0'; });
      setTimeout(() => r.remove(), 600);
    }, true);
  };
  if (document.readyState === 'loading') addEventListener('DOMContentLoaded', install); else install();
})();
`;
