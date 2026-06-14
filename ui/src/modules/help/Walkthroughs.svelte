<script lang="ts">
  // Walkthroughs page — plays the 6 onboarding MP4s with a left-rail selector
  // and a large video player area. Self-contained, no props.

  interface VideoItem {
    file: string;
    title: string;
    desc: string;
  }

  const videos: VideoItem[] = [
    { file: 'Intro.mp4',       title: 'Welcome to Otto',    desc: 'Run many AI coding agents in one native window.' },
    { file: 'AgentMode.mp4',   title: 'Agent Mode',         desc: 'Many sessions at once — tiled, broadcast, resumable.' },
    { file: 'Shortcuts.mp4',   title: 'Shortcuts',          desc: '⌘K command palette, ⌘I Ask Otto, ⌘F find, and more.' },
    { file: 'Connections.mp4', title: 'Connections',        desc: 'Save SSH / MySQL / Redis / ClickHouse and open them in a click.' },
    { file: 'GitPr.mp4',       title: 'Git & Pull Requests', desc: 'Branches, commit graph, and AI review agents.' },
    { file: 'Settings.mp4',    title: 'Settings',           desc: 'Themes, providers, accounts, channels — make it yours.' },
  ];

  let activeIndex = $state(0);
  let videoEl: HTMLVideoElement | null = $state(null);

  function select(i: number): void {
    activeIndex = i;
    // {#key} re-creates the <video> element when current.file changes.
    // Auto-play once the new element is mounted via $effect.
  }

  // Auto-play whenever a new video element is bound (after key block re-mounts).
  $effect(() => {
    if (videoEl) {
      void videoEl.play();
    }
  });

  const current = $derived(videos[activeIndex]);
</script>

<div class="walkthroughs">
  <div class="page-header">
    <h1 class="page-title">Walkthroughs</h1>
    <p class="page-sub">Short tours of Otto's features.</p>
  </div>

  <div class="layout">
    <!-- Left rail: video list -->
    <nav class="video-rail" aria-label="Walkthrough list">
      {#each videos as v, i (v.file)}
        <button
          class="rail-item"
          class:active={activeIndex === i}
          onclick={() => select(i)}
          aria-current={activeIndex === i ? 'true' : undefined}
        >
          <span class="item-num">{i + 1}</span>
          <span class="item-text">
            <span class="item-title">{v.title}</span>
            <span class="item-desc">{v.desc}</span>
          </span>
        </button>
      {/each}
    </nav>

    <!-- Main: player -->
    <div class="player-area">
      <div class="player-meta">
        <h2 class="player-title">{current.title}</h2>
        <p class="player-desc">{current.desc}</p>
      </div>
      {#key current.file}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          bind:this={videoEl}
          class="video-el"
          controls
          preload="metadata"
          src="/walkthroughs/{current.file}"
        ></video>
      {/key}
    </div>
  </div>
</div>

<style>
  .walkthroughs {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding: 28px 32px 16px;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
  }

  .page-header {
    flex-shrink: 0;
    margin-bottom: 20px;
  }

  .page-title {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    margin: 0 0 4px;
    color: var(--text);
  }

  .page-sub {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
  }

  .layout {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 20px;
  }

  /* Left rail */
  .video-rail {
    flex-shrink: 0;
    width: 220px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
  }

  .rail-item {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 9px 10px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    cursor: pointer;
    text-align: left;
    transition: background 120ms ease-out;
    width: 100%;
  }

  .rail-item:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }

  .rail-item.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .item-num {
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    font-size: 10px;
    font-weight: 700;
    display: grid;
    place-items: center;
    color: var(--text-dim);
    margin-top: 1px;
    transition: background 120ms ease-out, color 120ms ease-out;
  }

  .rail-item.active .item-num {
    background: var(--accent);
    color: #fff;
  }

  .item-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .item-title {
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1.3;
    color: var(--text);
  }

  .rail-item.active .item-title {
    color: var(--accent);
  }

  .item-desc {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  /* Player area */
  .player-area {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow: hidden;
  }

  .player-meta {
    flex-shrink: 0;
  }

  .player-title {
    font-size: 15px;
    font-weight: 600;
    margin: 0 0 3px;
    color: var(--text);
    letter-spacing: -0.01em;
  }

  .player-desc {
    font-size: 12.5px;
    color: var(--text-dim);
    margin: 0;
  }

  .video-el {
    flex: 1;
    min-height: 0;
    width: 100%;
    border-radius: var(--radius-s, 6px);
    border: 1px solid var(--border);
    background: #000;
    display: block;
    object-fit: contain;
  }
</style>
