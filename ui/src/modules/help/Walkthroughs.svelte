<script lang="ts">
  // Walkthroughs page — plays the onboarding MP4s with a left-rail selector,
  // a search box, and keyboard arrow-key navigation.
  // Videos are also registered as palette commands so they're reachable from ⌘K.
  import { registry } from '../../lib/commands.svelte';
  import { router } from '../../lib/router.svelte';

  interface VideoItem {
    file: string;
    title: string;
    desc: string;
    /** Extra keywords for the search/fuzzy index. */
    tags: string;
  }

  // One entry per rendered composition in marketing/videos/ (→ ui/public/walkthroughs/).
  // Order = recommended viewing order (Intro → features). Keep in sync with
  // marketing/videos/src/Root.tsx + render-all.mjs.
  const videos: VideoItem[] = [
    { file: 'Intro.mp4',          title: 'Welcome to Otto',         desc: 'Run many AI coding agents — and your whole workflow — in one native window.',                              tags: 'intro welcome overview onboarding first steps tour' },
    { file: 'Sessions.mp4',       title: 'Agent Sessions',          desc: 'claude, codex, agy & shell as live PTY sessions — tiled, broadcast, resumable, auto-trusted.',             tags: 'agent session terminal pty tiled broadcast resume trust claude codex shell' },
    { file: 'MissionControl.mp4', title: 'Mission Control',         desc: 'One unified work graph over every agent, swarm, goal loop, review and scheduled task.',                    tags: 'mission control work graph nodes overview unified status' },
    { file: 'Git.mp4',            title: 'Git & Pull Requests',     desc: 'Repo tabs, commit graph, conflict resolution, and agent-drafted PRs that auto-push.',                      tags: 'git pr pull request branch commit graph diff merge github bitbucket gitlab' },
    { file: 'Review.mp4',         title: 'AI Code Review',          desc: 'Fan out review agents per lens × provider; findings become a tracked workflow.',                          tags: 'review code lens findings security correctness performance pr working tree' },
    { file: 'ProofPacks.mp4',     title: 'Proof Packs',             desc: 'No “done” without evidence — artifacts, derived status & risk, and completion gates.',                     tags: 'proof pack evidence artifact status risk gate test pr badge' },
    { file: 'Product.mp4',        title: 'Product · Jira & Confluence', desc: 'Turn a ticket into a build-ready spec — analyze, ask, rewrite, plan, then inject into an agent.',        tags: 'product jira confluence ticket story spec plan analysis discovery rewrite' },
    { file: 'Canvas.mp4',         title: 'Canvas',                  desc: 'File-backed Excalidraw & Mermaid scenes an agent edits while you chat.',                                   tags: 'canvas excalidraw mermaid diagram draw scene visual' },
    { file: 'Swarm.mp4',          title: 'Agent Swarm',             desc: 'A company of role agents — recruiter, coordinator, org tree, Kanban & run graph.',                         tags: 'swarm team agent coordinator recruiter org kanban dag roles' },
    { file: 'GoalLoops.mp4',      title: 'Goal Loops',              desc: 'Give a goal + budget; agents iterate Plan→Execute→Evaluate→Digest until criteria pass.',                  tags: 'goal loop iterate plan execute evaluate digest budget criteria branch' },
    { file: 'Connections.mp4',    title: 'Connections · SSH & SFTP', desc: 'SSH/DB connections, tunnels (-L / SOCKS5), and an SFTP browser — secrets in Keychain.',                    tags: 'ssh sftp connection tunnel socks bastion keychain mysql redis mongo clickhouse' },
    { file: 'Database.mp4',       title: 'Database Explorer',       desc: 'TablePlus-class browser: schema tree, NL→SQL, inline edits, dashboards, export.',                          tags: 'database mysql redis mongodb clickhouse sql query schema nl join dashboard export' },
    { file: 'Brokers.mp4',        title: 'Message Brokers',         desc: 'Kafka (incl. AWS MSK over SSH) — topics, peek/produce, consumer-group lag, schema registry.',              tags: 'kafka broker topic produce consumer group lag schema registry msk aws' },
    { file: 'Channels.mp4',       title: 'Channels',                desc: 'Bridge a Slack or Telegram thread to an agent — messages & files both ways.',                              tags: 'slack telegram channel bridge thread ticket relay webhook' },
    { file: 'Workflows.mp4',      title: 'Workflows',               desc: 'Chain agents, HTTP, DB & approvals into a graph — manual, webhook & event triggers.',                     tags: 'workflow graph node trigger webhook approval automation pipeline' },
    { file: 'ScheduledTasks.mp4', title: 'Scheduled Tasks',         desc: 'Recurring agent jobs → a Markdown report → delivered to Slack, Telegram, email or webhook.',               tags: 'schedule task recurring cron report daily weekly interval deliver markdown' },
    { file: 'Mcp.mp4',            title: 'MCP Control Plane',       desc: 'Govern outbound MCP calls; expose Otto outward as otto.* tools behind a restricted token.',                tags: 'mcp model context protocol tool governance approval audit server outbound' },
    { file: 'Vault.mp4',          title: 'Vault',                   desc: 'Workspace knowledge store: [[backlinks]], hybrid keyword + semantic recall, a live graph.',                tags: 'vault memory knowledge note backlink graph search recall semantic' },
    { file: 'Skills.mp4',         title: 'Skills & Self-Improvement', desc: 'A versioned skill library that drives reviews & analysis — and improves itself from your sessions.',      tags: 'skill library install version self improvement reflect lens insights' },
    { file: 'SkillsEval.mp4',     title: 'Skills Evaluator',        desc: 'Benchmark a skill: implement→validate→score→improve across providers, compare runs.',                     tags: 'skill eval benchmark score iterate provider compare report' },
    { file: 'UsageInsights.mp4',  title: 'Usage, Cost & Insights',  desc: 'Real per-turn tokens & cost from transcripts, budgets, and scheduled catch-up reports.',                   tags: 'usage cost token clickhouse budget insight report daily weekly cache' },
    { file: 'Api.mp4',            title: 'API Client',              desc: 'A Postman-class workbench: HTTP/SSE/WS/gRPC, environments, import/export — SSRF-guarded.',                 tags: 'api http rest grpc websocket sse postman environment import ssrf' },
    { file: 'Plugins.mp4',        title: 'Custom Plugins',          desc: 'Runtime sidecar plugins in any language — supervised, reverse-proxied, scoped by RBAC.',                   tags: 'plugin sidecar extend runtime iframe host api rbac install' },
    { file: 'TeamMobile.mp4',     title: 'Multi-user & Mobile',     desc: 'Per-feature RBAC, scoped email-OTP share links, and an installable PWA over a tunnel.',                    tags: 'rbac user role share link otp mobile pwa tunnel remote tablet responsive sharing' },
    { file: 'Platform.mp4',       title: 'Platform & Shortcuts',    desc: '⌘K palette, themes, RTL, a customizable sidebar, and daily CLI auto-update.',                              tags: 'platform shortcut command palette theme rtl sidebar settings auto-update' },
  ];

  let activeIndex = $state(0);
  let videoEl: HTMLVideoElement | null = $state(null);
  let searchQuery = $state('');
  let railEl: HTMLElement | null = $state(null);

  // ---- filtered list ----
  const filteredVideos = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return videos.map((v, i) => ({ v, i }));
    return videos
      .map((v, i) => ({ v, i }))
      .filter(({ v }) =>
        v.title.toLowerCase().includes(q) ||
        v.desc.toLowerCase().includes(q) ||
        v.tags.includes(q),
      );
  });

  function select(i: number): void {
    activeIndex = i;
  }

  // Auto-play whenever a new video element is bound (after key block re-mounts).
  $effect(() => {
    if (videoEl) {
      void videoEl.play();
    }
  });

  // Arrow-key navigation within the rail (when it is focused or contains focus).
  // Arrow-key navigation: when the rail container or one of its children has
  // focus, ArrowDown/Up moves between visible entries.
  $effect(() => {
    function onRailKey(e: KeyboardEvent): void {
      // Only intercept when focus is inside the rail container.
      if (!railEl?.contains(document.activeElement)) return;
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const visible = filteredVideos.map((x) => x.i);
        const cur = visible.indexOf(activeIndex);
        if (e.key === 'ArrowDown') {
          const next = visible[Math.min(cur + 1, visible.length - 1)];
          if (next !== undefined) activeIndex = next;
        } else {
          const prev = visible[Math.max(cur - 1, 0)];
          if (prev !== undefined) activeIndex = prev;
        }
      }
    }
    window.addEventListener('keydown', onRailKey);
    return () => window.removeEventListener('keydown', onRailKey);
  });

  const current = $derived(videos[activeIndex]);

  // ---- register every video as a palette command ----
  $effect(() => {
    const cmds = videos.map((v, i) => ({
      id: `walkthrough.${v.file}`,
      title: `Walkthrough: ${v.title}`,
      group: 'Help',
      keywords: `${v.tags} video tour`,
      run: () => {
        activeIndex = i;
        router.go('walkthroughs');
      },
    }));
    const unreg = registry.register('walkthroughs', cmds);
    return unreg;
  });
</script>

<div class="walkthroughs">
  <div class="page-header">
    <h1 class="page-title">Walkthroughs</h1>
    <p class="page-sub">Short tours of Otto's features. Search or use ⌘K → "Walkthrough:".</p>
  </div>

  <div class="layout">
    <!-- Left rail: search + video list.
         Arrow-key navigation is handled globally (window keydown) while the
         rail has focus, so no handler is needed directly on this element. -->
    <div
      class="video-rail"
      aria-label="Walkthrough list"
      bind:this={railEl}
    >
      <div class="rail-search">
        <input
          class="search-input"
          type="search"
          placeholder="Search…"
          bind:value={searchQuery}
          aria-label="Filter walkthroughs"
        />
      </div>

      {#each filteredVideos as { v, i } (v.file)}
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
      {:else}
        <div class="rail-empty dim">No matches for "{searchQuery}"</div>
      {/each}
    </div>

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
    width: 230px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
    min-height: 0;
  }

  .rail-search {
    padding: 0 0 6px;
    flex-shrink: 0;
  }

  .search-input {
    width: 100%;
    box-sizing: border-box;
    padding: 5px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 5px);
    background: var(--surface-2);
    color: var(--text);
    font-size: 12.5px;
    outline: none;
  }

  .search-input:focus {
    border-color: var(--accent);
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
    text-align: start;
    transition: background 120ms ease-out;
    width: 100%;
  }

  .rail-item:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }

  .rail-item.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }

  .rail-empty {
    padding: 12px 10px;
    font-size: 12px;
    text-align: center;
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
