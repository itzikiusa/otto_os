///<reference types="svelte" />
;// The one tour film (Help page). Plays `lib/walkthroughs/film.json`'s file
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
import { announceShown } from '../../lib/announce.svelte';
import { api } from '../../lib/api/client';
import type { ResolveWalkthroughResp } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
import { chapterAt, timeLabel, type FilmManifest } from './guide';
import { BUNDLED_CAPTIONS } from './sections';
;

  interface Props {
    film: FilmManifest | null;
    /** Guide ids that exist (a chapter links to its guide only when it does). */
    guideIds: Set<string>;
    /** Open a guide (from a chapter's "Read the guide"). */
    onopenguide: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
  let filmEl: HTMLElement | undefined = $state();
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

  // Native video controls (including fullscreen) also change this track.
  // Keep the external CC button truthful without resetting a saved preference
  // while a replacement video is still loading after Retry.
  $effect(() => {
    const video = videoEl;
    const tracks = video?.textTracks;
    if (!video || !tracks) return;
    const syncCaptions = () => {
      if (video.readyState >= 1 && tracks[0]) captionsOn = tracks[0].mode === 'showing';
    };
    tracks.addEventListener('change', syncCaptions);
    return () => tracks.removeEventListener('change', syncCaptions);
  });

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
      revealPlayer();
      void videoEl.play().catch(() => {});
    }
  }

  /** Seek to `seconds` and play (queued until the metadata is in). Used by the
   *  chapter list, the markers and the guides' "Watch this part". */
   function playAt(seconds: number): void {
    revealPlayer();
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

  function revealPlayer(): void {
    // Only explicit playback requests reveal the film; timeupdate must leave
    // readers where they scrolled. Instant also respects reduced motion.
    filmEl?.querySelector('.film-frame')?.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'instant' });
  }
;
async () => {

 { const $$_section0 = svelteHTML.createElement("section", {      "class":`film`,"aria-label":`Tour film`,...__sveltets_2_empty({"data-testid":`tour-film`}),});filmEl = $$_section0;
  if(!film){
     { svelteHTML.createElement("div", {   "class":`film-missing`,...__sveltets_2_empty({"data-testid":`tour-film-missing`}),});
       { svelteHTML.createElement("span", {   "class":`film-missing-icon`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":14,}});} }
       { svelteHTML.createElement("span", {});               }
     }
  }else{
    if(status === 'failed'){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: 'The tour film can’t load right now. Check your connection, then retry.', assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`film-frame fallback`,...__sveltets_2_empty({"data-testid":`tour-film-unavailable`}),});
         { svelteHTML.createElement("span", {   "class":`fallback-icon`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":16,}});} }
         { svelteHTML.createElement("p", { "class":`fallback-title`,});       }
         { svelteHTML.createElement("p", { "class":`fallback-sub`,});               }
         { svelteHTML.createElement("div", { "class":`fallback-actions`,});
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":retry,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}  }
           { svelteHTML.createElement("a", {       "class":`btn small ghost`,"href":assetUrl(film.file),"target":`_blank`,"rel":`noopener noreferrer`,});
                { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}
           }
         }
       }}
    }else{
       { svelteHTML.createElement("div", {  "class":`film-frame`,});status === 'loading';
        
        
         { const $$_video2 = svelteHTML.createElement("video", {                        "class":`film-video`,"controls":true,"preload":`metadata`,"playsinline":true,"crossorigin":corsMode,"poster":film.poster ? assetUrl(film.poster) : undefined,"src":src ?? undefined,"onloadedmetadata":onMetadata,"oncanplay":() => (status = 'ready'),"ontimeupdate":() => (currentTime = videoEl?.currentTime ?? 0),"onerror":() => (status = 'failed'),...__sveltets_2_empty({"data-testid":`tour-film-video`}),});videoEl = $$_video2;
          if(captionsSrc){
              { svelteHTML.createElement("track", {        "kind":`captions`,"srclang":`en`,"label":`English`,"src":captionsSrc,"default":true,});}
          }
         }
        if(status === 'loading'){
           { svelteHTML.createElement("div", {     "class":`film-loading`,"role":`img`,"aria-label":`Loading the tour`,}); { svelteHTML.createElement("span", {    "class":`spinner`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, " "); } }
        }
       }
    }

     { svelteHTML.createElement("div", { "class":`film-meta`,});
       { svelteHTML.createElement("div", { "class":`film-caption`,});
         { svelteHTML.createElement("strong", {});   }
        if(duration){ { svelteHTML.createElement("span", { "class":`dim`,});timeLabel(duration);chapters.length ? ` · ${chapters.length} chapters` : ''; }}
       }
      if(captionsSrc && status !== 'failed'){
         { svelteHTML.createElement("button", {         "class":`btn small ghost cc`,"aria-pressed":captionsOn,"onclick":toggleCaptions,"title":captionsOn ? 'Hide captions' : 'Show captions',});
           captionsOn ? 'on' : 'off';
         }
      }
     }

    if(chapters.length && duration){
      
       { svelteHTML.createElement("div", {   "class":`markers`,"aria-hidden":`true`,});
           for(let c of __sveltets_2_ensureArray(chapters)){c.id;
           { svelteHTML.createElement("button", {            "class":`marker`,"tabindex":-1,"title":`${timeLabel(c.start)} ${c.title}`,"onclick":() => playAt(c.start),});activeChapter?.id === c.id;__sveltets_2_ensureType(String, Number, `${(c.start / duration) * 100} `);__sveltets_2_ensureType(String, Number, `${(Math.max(c.duration, 0) / duration) * 100} `); }
        }
         { svelteHTML.createElement("span", {  "class":`playhead`,});__sveltets_2_ensureType(String, Number, `${Math.min(currentTime / duration, 1) * 100} `); }
       }
    }

    if(chapters.length){
       { svelteHTML.createElement("ol", {   "class":`chapters`,"aria-label":`Chapters`,});
           for(let c of __sveltets_2_ensureArray(chapters)){c.id;
           { svelteHTML.createElement("li", {  "class":`chapter`,});activeChapter?.id === c.id;
             { svelteHTML.createElement("button", {         "class":`chapter-play`,"aria-current":activeChapter?.id === c.id ? 'step' : undefined,"onclick":() => playAt(c.start),"title":`Play from ${timeLabel(c.start)}`,});
               { svelteHTML.createElement("span", { "class":`chapter-time`,});timeLabel(c.start); }
               { svelteHTML.createElement("span", { "class":`chapter-title`,});c.title; }
             }
            if(guideIds.has(c.section)){
               { svelteHTML.createElement("button", {   "class":`chapter-guide`,"onclick":() => onopenguide(c.section),});   }
            }
           }
        }
       }
    }
  }
 }


};
return { props: {} as any as Props, exports: {} as any as { playAt: typeof playAt }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TourFilm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TourFilm__SvelteComponent_ = ReturnType<typeof TourFilm__SvelteComponent_>;
/*Ωignore_endΩ*/export default TourFilm__SvelteComponent_;