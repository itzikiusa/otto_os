<script lang="ts">
  // Interactive Web Audio waveform player + scrubber for TTS clips.
  // Reads theme colours from CSS custom properties at draw time so light,
  // dark, Pro Dark, Warm and custom accent schemes stay token-consistent.
  import { onDestroy } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { formatDurationMs } from './ttsPresets';

  interface Props {
    src: string | null;
    title?: string;
    subtitle?: string;
    onexport?: () => void;
    ondownload?: () => void;
  }
  let { src, title = 'Audio clip', subtitle = '', onexport, ondownload }: Props = $props();

  let audioEl = $state<HTMLAudioElement | null>(null);
  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let playing = $state(false);
  let loop = $state(false);
  let currentSec = $state(0);
  let durationSec = $state(0);
  let peaks = $state<number[]>([]);
  let decoding = $state(false);
  let decodeError = $state<string | null>(null);
  let rafId = 0;

  const progress = $derived(durationSec > 0 ? Math.min(1, Math.max(0, currentSec / durationSec)) : 0);

  // Decode audio peaks whenever `src` changes.
  let decodeSeq = 0;
  $effect(() => {
    const url = src;
    const my = ++decodeSeq;
    playing = false;
    currentSec = 0;
    durationSec = 0;
    peaks = [];
    decodeError = null;
    if (!url) return;
    decoding = true;
    void decodePeaks(url, my);
  });

  async function decodePeaks(url: string, seq: number): Promise<void> {
    let ctx: AudioContext | null = null;
    try {
      const resp = await fetch(url);
      if (!resp.ok) throw new Error('Audio stream unavailable');
      const buf = await resp.arrayBuffer();
      if (seq !== decodeSeq) return;
      const AudioCtx = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      ctx = new AudioCtx();
      const decoded = await ctx.decodeAudioData(buf.slice(0));
      if (seq !== decodeSeq) return;
      durationSec = decoded.duration;
      const ch = decoded.getChannelData(0);
      const bars = 96;
      const step = Math.max(1, Math.floor(ch.length / bars));
      const out: number[] = [];
      let max = 0.01;
      for (let i = 0; i < bars; i++) {
        let peak = 0;
        const start = i * step;
        const end = Math.min(ch.length, start + step);
        for (let j = start; j < end; j++) {
          const v = Math.abs(ch[j]);
          if (v > peak) peak = v;
        }
        out.push(peak);
        if (peak > max) max = peak;
      }
      peaks = out.map((v) => Math.max(0.08, v / max));
    } catch {
      if (seq === decodeSeq) {
        // Fallback flat waveform if codec isn't decodable by WebAudio (still playable by <audio>).
        peaks = Array.from({ length: 64 }, (_, i) => 0.25 + 0.45 * Math.abs(Math.sin(i * 0.42)));
        decodeError = null;
      }
    } finally {
      if (ctx) void ctx.close().catch(() => {});
      if (seq === decodeSeq) decoding = false;
    }
  }

  function draw(): void {
    const canvas = canvasEl;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const w = Math.max(1, Math.floor(rect.width));
    const h = Math.max(1, Math.floor(rect.height));
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
    }
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const style = getComputedStyle(canvas);
    const playedColor = style.getPropertyValue('--accent-solid').trim() || style.color;
    const restColor = style.getPropertyValue('--border-strong').trim() || style.color;
    const bars = peaks.length || 64;
    const gap = 2;
    const barW = Math.max(2, (w - gap * (bars - 1)) / bars);
    const midY = h / 2;

    for (let i = 0; i < bars; i++) {
      const amp = peaks[i] ?? 0.2;
      const barH = Math.max(4, amp * (h - 10));
      const x = i * (barW + gap);
      const frac = bars > 1 ? i / (bars - 1) : 0;
      ctx.fillStyle = frac <= progress ? playedColor : restColor;
      ctx.beginPath();
      ctx.roundRect(x, midY - barH / 2, barW, barH, 2);
      ctx.fill();
    }
  }

  $effect(() => {
    void peaks;
    void progress;
    draw();
  });

  $effect(() => {
    const el = canvasEl;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => draw());
    ro.observe(el);
    return () => ro.disconnect();
  });

  function tickPlayhead(): void {
    if (audioEl) currentSec = audioEl.currentTime;
    draw();
    if (playing) rafId = requestAnimationFrame(tickPlayhead);
  }

  function togglePlay(): void {
    const a = audioEl;
    if (!a || !src) return;
    if (a.paused) {
      void a.play().catch(() => {});
    } else {
      a.pause();
    }
  }

  function seekToFrac(frac: number): void {
    const a = audioEl;
    if (!a || !durationSec) return;
    const clamped = Math.min(1, Math.max(0, frac));
    a.currentTime = clamped * durationSec;
    currentSec = a.currentTime;
    draw();
  }

  function onWaveClick(e: MouseEvent): void {
    const el = canvasEl;
    if (!el) return;
    const r = el.getBoundingClientRect();
    if (r.width <= 0) return;
    seekToFrac((e.clientX - r.left) / r.width);
  }

  function onWaveKey(e: KeyboardEvent): void {
    if (e.key === ' ' || e.key === 'Enter') {
      e.preventDefault();
      togglePlay();
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      seekToFrac(progress + 0.05);
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      seekToFrac(progress - 0.05);
    }
  }

  onDestroy(() => {
    decodeSeq++;
    cancelAnimationFrame(rafId);
  });
</script>

<div class="wp" data-testid="tts-waveform-player">
  <div class="wp-head">
    <div class="wp-meta">
      <strong class="wp-title">{title}</strong>
      {#if subtitle}<span class="wp-sub">{subtitle}</span>{/if}
    </div>
    <div class="wp-actions">
      {#if ondownload}
        <button type="button" class="btn small" disabled={!src} onclick={ondownload}>
          <Icon name="download" size={12} /> Download
        </button>
      {/if}
      {#if onexport}
        <button type="button" class="btn small" disabled={!src} onclick={onexport} data-testid="tts-export-ws">
          <Icon name="folder" size={12} /> Export to Workspace…
        </button>
      {/if}
    </div>
  </div>

  <div class="wp-body">
    <button
      type="button"
      class="icon-btn play-btn"
      disabled={!src}
      onclick={togglePlay}
      aria-label={playing ? 'Pause audio' : 'Play audio'}
      title={playing ? 'Pause (Space)' : 'Play (Space)'}
      data-testid="tts-play-toggle"
    >
      <Icon name={playing ? 'pause' : 'play'} size={14} />
    </button>

    <div
      class="wave-wrap"
      role="slider"
      tabindex="0"
      aria-label="Audio scrubber"
      aria-valuemin={0}
      aria-valuemax={Math.round(durationSec * 1000)}
      aria-valuenow={Math.round(currentSec * 1000)}
      aria-valuetext={`${formatDurationMs(currentSec * 1000)} of ${formatDurationMs(durationSec * 1000)}`}
      onclick={onWaveClick}
      onkeydown={onWaveKey}
    >
      <canvas class="wave" bind:this={canvasEl}></canvas>
      {#if decoding}<span class="wave-note">Decoding…</span>{/if}
      {#if decodeError}<span class="wave-note err">{decodeError}</span>{/if}
    </div>

    <span class="time" aria-label="Playback time">
      {formatDurationMs(currentSec * 1000)} / {formatDurationMs(durationSec * 1000)}
    </span>

    <button
      type="button"
      class="icon-btn"
      class:on={loop}
      aria-pressed={loop}
      aria-label="Toggle audio loop"
      title="Loop playback"
      onclick={() => (loop = !loop)}
    >
      <Icon name="refresh" size={16} />
    </button>
  </div>

  <audio
    bind:this={audioEl}
    src={src ?? undefined}
    {loop}
    onloadedmetadata={() => {
      if (audioEl && Number.isFinite(audioEl.duration)) durationSec = audioEl.duration;
    }}
    onplay={() => {
      playing = true;
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(tickPlayhead);
    }}
    onpause={() => {
      playing = false;
      cancelAnimationFrame(rafId);
      if (audioEl) currentSec = audioEl.currentTime;
      draw();
    }}
    onended={() => {
      playing = false;
      cancelAnimationFrame(rafId);
      currentSec = 0;
      draw();
    }}
  ></audio>
</div>

<style>
  .wp {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .wp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }
  .wp-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
  }
  .wp-title {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wp-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .wp-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .wp-body {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .play-btn {
    background: var(--accent-solid);
    color: var(--accent-contrast);
    border-radius: 999px;
    inline-size: 32px;
    block-size: 32px;
  }
  .play-btn:disabled {
    opacity: var(--disabled-opacity);
  }
  .wave-wrap {
    position: relative;
    flex: 1;
    min-inline-size: 0;
    block-size: 48px;
    border-radius: var(--radius-s);
    background: var(--bg);
    border: 1px solid var(--border);
    padding: 4px 6px;
    cursor: pointer;
  }
  .wave-wrap:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .wave {
    display: block;
    inline-size: 100%;
    block-size: 100%;
  }
  .wave-note {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    pointer-events: none;
  }
  .wave-note.err {
    color: var(--danger);
  }
  .time {
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  @media (pointer: coarse) {
    .play-btn,
    .wp :global(.icon-btn) {
      min-inline-size: 40px;
      min-block-size: 40px;
    }
    .wp :global(.btn) {
      min-block-size: 40px;
    }
  }
</style>
