<script lang="ts">
  // The one tour film (Help page). Plays `lib/walkthroughs/film.json`'s file
  // with chapter markers + a chapter list; a chapter whose `section` names an
  // existing guide links to it ("Read the guide"), and guides seek back here
  // through `playAt()` ("Watch this part").
  //
  // HOSTING: the MP4/poster/captions are NOT bundled with the app. They live on
  // the rolling `walkthroughs` GitHub release (or VITE_WALKTHROUGHS_V2_BASE /
  // VITE_WALKTHROUGHS_BASE — a mirror, CDN or local screening server).
  //
  // GitHub serves release assets through a 302 to a short-lived signed URL, and
  // WebKit (the desktop webview) refuses a <video src> that redirects
  // (MEDIA_ERR_SRC_NOT_SUPPORTED — Chromium tolerates it, which is why this only
  // bites in the app). The daemon resolves the hop (`GET /walkthroughs/resolve`)
  // and the element gets the final URL; the raw URL is the fallback when the
  // daemon can't resolve it (offline, a remote browser without the route…).
  //
  // CAPTIONS: GitHub's asset host sends no CORS headers, so a remote <track> can
  // never load from it (and `crossorigin` on the <video> would break playback
  // outright). Captions therefore come from a VTT bundled next to film.json
  // (played from a same-origin blob: URL), or — for a non-GitHub base that is
  // expected to send CORS headers — straight from the host.
  //
  // States: no manifest (quiet "not in this build" card) · loading (poster +
  // "Loading…") · unavailable (inline message + Retry, never a raw error) ·
  // playing.
  import { onDestroy } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { ResolveWalkthroughResp } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { chapterAt, timeLabel, type FilmManifest } from './guide';
  import { BUNDLED_CAPTIONS } from './sections';

  interface Props {
    film: FilmManifest | null;
    /** Guide ids that exist (a chapter links to its guide only when it does). */
    guideIds: Set<string>;
    /** Open a guide (from a chapter's "Read the guide"). */
    onopenguide: (id: string) => void;
  }
  let { film, guideIds, onopenguide }: Props = $props();

  const customBase: string | undefined =
    import.meta.env.VITE_WALKTHROUGHS_V2_BASE?.trim() || import.meta.env.VITE_WALKTHROUGHS_BASE?.trim() || undefined;
  const BASE: string = (customBase ?? 'https://github.com/itzikiusa/otto_os/releases/download/walkthroughs').replace(
    /\/+$/,
    '',
  );
  const isGithub = BASE.startsWith('https://github.com/');

  function assetUrl(file: string): string {
    return `${BASE}/${file}`;
  }

  let videoEl: HTMLVideoElement | null = $state(null);
  let src = $state<string | null>(null);
  let status = $state<'loading' | 'ready' | 'failed'>('loading');
  let currentTime = $state(0);
  let mediaDuration = $state(0);
  let captionsOn = $state(true);
  let pendingSeek: number | null = null;
  let resolveNonce = 0;

  const duration = $derived(mediaDuration || film?.duration || 0);
  const chapters = $derived(film?.chapters ?? []);
  const activeChapter = $derived(chapterAt(chapters, currentTime));

  // Captions: bundled VTT → blob URL; else remote only from a CORS-capable host.
  const bundledVtt = $derived(film?.captions ? BUNDLED_CAPTIONS[film.captions] : undefined);
  let captionsBlob: string | null = null;
  const captionsSrc = $derived.by(() => {
    if (captionsBlob) URL.revokeObjectURL(captionsBlob);
    captionsBlob = null;
    if (!film?.captions) return null;
    if (bundledVtt) {
      captionsBlob = URL.createObjectURL(new Blob([bundledVtt], { type: 'text/vtt' }));
      return captionsBlob;
    }
    return isGithub ? null : assetUrl(film.captions);
  });
  // A remote track needs a CORS-mode media request (never for GitHub, see above).
  const corsMode = $derived(!!captionsSrc && !bundledVtt ? 'anonymous' : undefined);
  onDestroy(() => {
    if (captionsBlob) URL.revokeObjectURL(captionsBlob);
  });

  async function resolve(): Promise<void> {
    if (!film) return;
    const raw = assetUrl(film.file);
    const nonce = ++resolveNonce;
    let url = raw;
    if (isGithub) {
      try {
        url = (await api.get<ResolveWalkthroughResp>(`/walkthroughs/resolve?url=${encodeURIComponent(raw)}`)).url || raw;
      } catch {
        url = raw;
      }
    }
    if (nonce === resolveNonce) src = url;
  }

  $effect(() => {
    if (!film) return;
    status = 'loading';
    src = null;
    void resolve();
  });

  function retry(): void {
    // The signed URL expires (~1h) — re-resolve rather than re-hit a dead one.
    status = 'loading';
    src = null;
    void resolve();
  }

  function applyCaptions(): void {
    const t = videoEl?.textTracks?.[0];
    if (t) t.mode = captionsOn ? 'showing' : 'hidden';
  }

  function toggleCaptions(): void {
    captionsOn = !captionsOn;
    applyCaptions();
  }

  function onMetadata(): void {
    status = 'ready';
    mediaDuration = Number.isFinite(videoEl?.duration) ? (videoEl?.duration ?? 0) : 0;
    applyCaptions();
    if (videoEl && pendingSeek !== null) {
      videoEl.currentTime = pendingSeek;
      pendingSeek = null;
      void videoEl.play().catch(() => {});
    }
  }

  /** Seek to `seconds` and play (queued until the metadata is in). Used by the
   *  chapter list, the markers and the guides' "Watch this part". */
  export function playAt(seconds: number): void {
    currentTime = seconds;
    if (status === 'failed') {
      pendingSeek = seconds;
      retry();
      return;
    }
    if (videoEl && videoEl.readyState >= 1) {
      videoEl.currentTime = seconds;
      void videoEl.play().catch(() => {});
    } else {
      pendingSeek = seconds;
    }
  }
</script>

<section class="film" aria-label="Tour film" data-testid="tour-film">
  {#if !film}
    <div class="film-missing" role="status" data-testid="tour-film-missing">
      <span class="film-missing-icon" aria-hidden="true"><Icon name="play" size={14} /></span>
      <span>The tour film isn't part of this build. The guides below cover everything it shows.</span>
    </div>
  {:else}
    {#if status === 'failed'}
      <div class="film-frame fallback" role="status" data-testid="tour-film-unavailable">
        <span class="fallback-icon" aria-hidden="true"><Icon name="play" size={18} /></span>
        <p class="fallback-title">The tour film can't load right now</p>
        <p class="fallback-sub">It streams from the internet. Check your connection, then try again. The guides work offline.</p>
        <div class="fallback-actions">
          <button class="btn small" onclick={retry}><Icon name="refresh" size={12} /> Retry</button>
          <a class="btn small ghost" href={assetUrl(film.file)} target="_blank" rel="noopener noreferrer">
            Open in browser <Icon name="external" size={12} />
          </a>
        </div>
      </div>
    {:else}
      <div class="film-frame" class:loading={status === 'loading'}>
        <!-- preload="metadata": only the moov atom + first frame are fetched
             until the viewer presses play. Narrated, so it never autoplays.
             The captions <track> is conditional (see CAPTIONS above). -->
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          bind:this={videoEl}
          class="film-video"
          controls
          preload="metadata"
          playsinline
          crossorigin={corsMode}
          poster={film.poster ? assetUrl(film.poster) : undefined}
          src={src ?? undefined}
          onloadedmetadata={onMetadata}
          oncanplay={() => (status = 'ready')}
          ontimeupdate={() => (currentTime = videoEl?.currentTime ?? 0)}
          onerror={() => (status = 'failed')}
          data-testid="tour-film-video"
        >
          {#if captionsSrc}
            <track kind="captions" srclang="en" label="English" src={captionsSrc} default />
          {/if}
        </video>
        {#if status === 'loading'}
          <div class="film-loading" aria-hidden="true">Loading the tour…</div>
        {/if}
      </div>
    {/if}

    <div class="film-meta">
      <div class="film-caption">
        <strong>Tour of Otto</strong>
        {#if duration}<span class="dim">{timeLabel(duration)}{chapters.length ? ` · ${chapters.length} chapters` : ''}</span>{/if}
      </div>
      {#if captionsSrc && status !== 'failed'}
        <button
          class="btn small ghost cc"
          aria-pressed={captionsOn}
          onclick={toggleCaptions}
          title={captionsOn ? 'Hide captions' : 'Show captions'}
        >
          CC {captionsOn ? 'on' : 'off'}
        </button>
      {/if}
    </div>

    {#if chapters.length && duration}
      <!-- Scrubber markers: one segment per chapter, proportional to its length. -->
      <div class="markers" aria-hidden="true">
        {#each chapters as c (c.id)}
          <button
            class="marker"
            class:active={activeChapter?.id === c.id}
            style:inset-inline-start="{(c.start / duration) * 100}%"
            style:width="{(Math.max(c.duration, 0) / duration) * 100}%"
            tabindex="-1"
            title="{timeLabel(c.start)} {c.title}"
            onclick={() => playAt(c.start)}
          ></button>
        {/each}
        <span class="playhead" style:inset-inline-start="{Math.min(currentTime / duration, 1) * 100}%"></span>
      </div>
    {/if}

    {#if chapters.length}
      <ol class="chapters" aria-label="Chapters">
        {#each chapters as c (c.id)}
          <li class="chapter" class:active={activeChapter?.id === c.id}>
            <button
              class="chapter-play"
              aria-current={activeChapter?.id === c.id ? 'step' : undefined}
              onclick={() => playAt(c.start)}
              title="Play from {timeLabel(c.start)}"
            >
              <span class="chapter-time">{timeLabel(c.start)}</span>
              <span class="chapter-title">{c.title}</span>
            </button>
            {#if guideIds.has(c.section)}
              <button class="chapter-guide" onclick={() => onopenguide(c.section)}>Read the guide</button>
            {/if}
          </li>
        {/each}
      </ol>
    {/if}
  {/if}
</section>

<style>
  .film {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 880px;
  }
  .film-frame {
    position: relative;
    aspect-ratio: 16 / 9;
    max-height: 52vh;
    width: 100%;
    border-radius: var(--radius-m);
    overflow: hidden;
    border: 1px solid var(--border);
    /* The letterbox behind the film is black in every theme, like any player. */
    background: black;
  }
  .film-video {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .film-loading {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: var(--fs-s);
    color: color-mix(in srgb, white 70%, transparent);
    pointer-events: none;
  }
  .film-frame.fallback {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 24px;
    text-align: center;
    background: var(--surface-2);
    border-style: dashed;
    color: var(--text);
  }
  .fallback-icon {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: var(--text-dim);
    background: var(--hover);
    margin-bottom: 4px;
  }
  .fallback-title {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .fallback-sub {
    margin: 0;
    max-width: 420px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .fallback-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }
  .film-missing {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .film-missing-icon {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: var(--hover);
    flex-shrink: 0;
  }
  .film-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: var(--fs-s);
  }
  .film-caption {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .cc[aria-pressed='true'] {
    color: var(--accent-text);
  }
  .markers {
    position: relative;
    height: 8px;
    border-radius: 4px;
    background: var(--surface-2);
  }
  .marker {
    position: absolute;
    inset-block: 0;
    padding: 0;
    border: 0;
    border-inline-end: 2px solid var(--bg);
    background: var(--hover);
    cursor: pointer;
  }
  .marker:hover {
    background: var(--border-strong);
  }
  .marker.active {
    background: var(--accent-soft);
  }
  .playhead {
    position: absolute;
    inset-block: -2px;
    width: 2px;
    background: var(--accent);
    pointer-events: none;
  }
  .chapters {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 4px;
  }
  .chapter {
    display: flex;
    align-items: center;
    border-radius: var(--radius-s);
    min-width: 0;
  }
  .chapter.active {
    background: var(--accent-soft);
  }
  .chapter-play {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 6px 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .chapter-play:hover {
    background: var(--hover);
  }
  .chapter-time {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    font-size: var(--fs-xs);
  }
  .chapter-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chapter.active .chapter-title {
    color: var(--accent-text);
  }
  .chapter-guide {
    flex-shrink: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    padding: 6px 8px;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .chapter-guide:hover {
    background: var(--hover);
  }
  .dim {
    color: var(--text-dim);
  }
</style>
