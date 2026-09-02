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
    /** Feature guide under docs/features/ — the code-grounded, current reference. */
    doc: string;
  }

  /** A module that has a feature guide but no walkthrough video (yet). */
  interface GuideItem {
    title: string;
    desc: string;
    tags: string;
    doc: string;
  }

  // The feature guides are the authoritative, code-grounded explainers; videos
  // lag behind them (see marketing/videos/UPDATE_PLAN.md). Link every entry to
  // its guide so a stale caption never becomes the last word.
  const DOCS_BASE = 'https://github.com/itzikiusa/otto_os/blob/main/docs/features';

  function docUrl(doc: string): string {
    return `${DOCS_BASE}/${doc}`;
  }

  // The MP4s are NOT bundled with the app (they were ~135 MB and were baked
  // into both ottod's embed-ui and the Tauri bundle). They are hosted as assets
  // of the rolling GitHub release tagged `walkthroughs`
  // (packaging/publish-walkthroughs.sh re-encodes + uploads them). Override the
  // base URL at build time with VITE_WALKTHROUGHS_BASE (e.g. a mirror / CDN).
  const WALKTHROUGHS_BASE: string =
    import.meta.env.VITE_WALKTHROUGHS_BASE ??
    'https://github.com/itzikiusa/otto_os/releases/download/walkthroughs';

  function videoUrl(file: string): string {
    return `${WALKTHROUGHS_BASE.replace(/\/+$/, '')}/${file}`;
  }

  // One entry per rendered composition in marketing/videos/ (→ marketing/videos/out/,
  // published to the `walkthroughs` release). Order = recommended viewing order:
  // Intro → agents & delivery → automation → knowledge → infra & data → platform.
  // Keep the `file` set in sync with marketing/videos/src/Root.tsx + render-all.mjs.
  // `desc`/`tags` describe the feature as it ships TODAY (so search finds current
  // terminology) — where a video's captions lag, the note says so and the guide
  // link is the truth. Caption fixes live in marketing/videos/UPDATE_PLAN.md.
  const videos: VideoItem[] = [
    { file: 'Intro.mp4',          title: 'Welcome to Otto',           desc: 'Run many AI coding agents — and your whole workflow — in one native window.',                                                             tags: 'intro welcome overview onboarding first steps tour ade agentic development environment',                              doc: 'README.md' },
    { file: 'Sessions.mp4',       title: 'Agent Sessions',            desc: 'claude, codex, agy (Antigravity), custom providers & shell as live PTY sessions — tiled, split, broadcast, resumable, auto-trusted.',       tags: 'agent session terminal pty tiled split broadcast resume trust claude codex agy antigravity shell custom provider model picker names', doc: 'agent-sessions.md' },
    { file: 'MissionControl.mp4', title: 'Mission Control',           desc: 'One live work graph over sessions, swarms, goal loops, workflows, reviews, product stories, PRs & external triggers.',                       tags: 'mission control work graph nodes overview unified status workflow pr external trigger feed',                          doc: 'mission-control.md' },
    { file: 'Git.mp4',            title: 'Git & Pull Requests',       desc: 'Repo tabs, commit graph, WIP staging, the conflict resolver, worktrees, a Focus tab, and agent-drafted PRs that auto-push.',              tags: 'git pr pull request branch commit graph diff merge conflict resolver wip stage discard worktree focus stash draft github bitbucket gitlab', doc: 'git.md' },
    { file: 'Review.mp4',         title: 'AI Code Review',            desc: 'One reviewer per lens × provider over a PR or working tree; findings become tracked records — triage, fix with an agent, verify.',        tags: 'review code lens findings security correctness performance tests pr working tree triage verify waive regressed', doc: 'code-review.md' },
    { file: 'ProofPacks.mp4',     title: 'Proof Packs',               desc: 'No “done” without evidence — artifacts, derived status & risk, and completion gates on PRs, goal loops and workflows.',                    tags: 'proof pack evidence artifact status risk gate test pr badge score report',                                             doc: 'proof-packs.md' },
    { file: 'Product.mp4',        title: 'Product · Jira & Confluence', desc: 'Ticket or Confluence page → analyze, ask, rewrite, test cases, plan → hand to a swarm or a fresh agent session; publishes back to Jira & Confluence.', tags: 'product jira confluence ticket story spec plan analysis discovery rfc rewrite test cases learnings mockup publish inject', doc: 'product.md' },
    { file: 'Canvas.mp4',         title: 'Canvas',                    desc: 'File-backed Excalidraw, Mermaid & D2 scenes an agent edits while you chat.',                                                             tags: 'canvas excalidraw mermaid d2 diagram draw scene visual',                                                              doc: 'canvas.md' },
    { file: 'Swarm.mp4',          title: 'Agent Swarm',               desc: 'A company of role agents — recruiter, per-swarm coordinator, org tree, Kanban board, run graph, schedules & presets.',                    tags: 'swarm team agent coordinator recruiter org kanban board dag roles preset budget',                                      doc: 'agent-swarm.md' },
    { file: 'GoalLoops.mp4',      title: 'Goal Loops',                desc: 'Give a goal + budget; agents iterate Plan→Execute→Evaluate→Digest on an isolated goal-loop/<id> branch until criteria pass.',           tags: 'goal loop iterate plan execute evaluate digest budget criteria branch worktree',                                       doc: 'goal-loops.md' },
    { file: 'Workflows.mp4',      title: 'Workflows',                 desc: 'Chain agents, HTTP, DB, brokers, approvals & swarm tasks into a graph — manual, webhook, event, schedule & Slack-chat triggers; retry + run queue.', tags: 'workflow graph node trigger webhook approval automation pipeline schedule cron slack chat retry queue condition loop version', doc: 'workflows.md' },
    { file: 'ScheduledTasks.mp4', title: 'Scheduled Tasks',           desc: 'Recurring agent jobs (interval, daily, weekly, cron) → a Markdown report → Slack, Telegram, email or webhook; presets & otto.* MCP tools.', tags: 'schedule task recurring cron report daily weekly interval timezone deliver markdown preset',                          doc: 'scheduled-tasks.md' },
    { file: 'Channels.mp4',       title: 'Channels',                  desc: 'Bridge a Slack or Telegram thread to an agent — messages relayed both ways (files in on Slack, out on both) — plus Broadcast.',            tags: 'slack telegram channel bridge thread ticket relay broadcast socket mode botfather',                                    doc: 'channels-slack-telegram.md' },
    { file: 'Skills.mp4',         title: 'Skills & Self-Improvement', desc: 'A versioned skill library (Settings → Skills) that drives review lenses, product analysis & insights — and improves itself from your sessions.', tags: 'skill library install version self improvement reflect lens insights okf',                                        doc: 'skills-library.md' },
    { file: 'SkillsEval.mp4',     title: 'Skills Lab · Evaluator',    desc: 'Benchmark a skill: implement→validate→score→improve across providers, compare runs — now the Evaluator tab of the Skills Lab module.',   tags: 'skill eval evaluator benchmark score iterate provider compare report skills lab review editor',                       doc: 'skills-evaluator.md' },
    { file: 'Vault.mp4',          title: 'Vault',                     desc: 'Docs home: register a local (Obsidian) markdown folder — wikilinks & backlinks, tags, full-text search, a scalable graph, OKF validation. Video predates Vault v3 (no vector recall).', tags: 'vault docs knowledge note obsidian markdown backlink wikilink graph search fts tags okf quick switcher', doc: 'vault.md' },
    { file: 'Connections.mp4',    title: 'Connections · SSH & SFTP',  desc: 'SSH / MySQL / Postgres / Redis / Mongo / ClickHouse / Kafka connections, tunnels (-L / SOCKS5), and an SFTP browser — secrets in Keychain.', tags: 'ssh sftp connection tunnel socks bastion keychain mysql postgres redis mongo clickhouse kafka custom',          doc: 'connections-ssh-sftp.md' },
    { file: 'Database.mp4',       title: 'Database Explorer',         desc: 'TablePlus-class browser: schema tree, DB Assistant (NL→SQL), reviewed inline edits, index editor, mongosh scripts, ERD, dashboards, export.', tags: 'database mysql postgres redis mongodb clickhouse sql query schema nl assistant join index mongosh explain erd dashboard export detached', doc: 'database-explorer.md' },
    { file: 'Brokers.mp4',        title: 'Message Brokers',           desc: 'Kafka (incl. AWS MSK over SSH) — topics & configs, peek/produce, consumer-group lag, replay, lag alerts, schema registry.',                tags: 'kafka broker topic produce consumer group lag replay alert schema registry msk config',                             doc: 'message-brokers.md' },
    { file: 'Api.mp4',            title: 'API Client',                desc: 'A Postman-class workbench: HTTP/SSE/WS/gRPC/GraphQL, environments, cookie jar, Postman/OpenAPI/HAR import, automations — SSRF-guarded.', tags: 'api http rest grpc graphql websocket sse postman openapi har environment import automation ssrf',                    doc: 'api-client.md' },
    { file: 'Mcp.mp4',            title: 'MCP Control Plane',         desc: 'Govern MCP calls (allowlist → policy → approval → dry-run → fail-closed audit); expose Otto outward as 100+ otto.* tools behind a restricted token.', tags: 'mcp model context protocol tool governance approval audit server outbound gateway policy',                        doc: 'mcp-control-plane.md' },
    { file: 'Plugins.mp4',        title: 'Custom Plugins',            desc: 'Runtime sidecar plugins in any language — supervised, reverse-proxied, scoped by RBAC.',                                                  tags: 'plugin sidecar extend runtime iframe host api rbac install',                                                          doc: 'plugins.md' },
    { file: 'UsageInsights.mp4',  title: 'Usage, Cost & Insights',    desc: 'Real per-turn tokens & cost from transcripts, opt-in budgets, system metrics, and scheduled catch-up reports.',                           tags: 'usage cost token clickhouse budget insight report daily weekly cache metrics',                                        doc: 'usage-and-cost.md' },
    { file: 'TeamMobile.mp4',     title: 'Multi-user & Mobile',       desc: 'Per-feature RBAC grants + workspace roles, scoped expiring share links with email-OTP, and an installable PWA over a tunnel.',              tags: 'rbac user role grant share link otp mobile pwa tunnel remote tablet responsive sharing impersonation',              doc: 'rbac-multiuser-sharing.md' },
    { file: 'Platform.mp4',       title: 'Platform & Shortcuts',      desc: '⌘K palette, ⌘I Ask Otto, themes, RTL, a customizable sidebar, multi-window, and daily CLI auto-update.',                                  tags: 'platform shortcut command palette theme rtl sidebar settings auto-update multi window',                               doc: 'rtl-and-responsive.md' },
  ];

  // Modules that ship today but have no walkthrough yet — listed so the page
  // is an honest map of Otto, not just of what was filmed. Each links to its
  // feature guide; a later render pass turns these into videos.
  const guides: GuideItem[] = [
    { title: 'Run with Otto',      desc: 'One button: a Jira/Confluence/GitHub/Slack item → worktree → agent → proof pack → review → approval → PR.', tags: 'run with otto one button pipeline jira slack pr approval',         doc: 'run-with-otto.md' },
    { title: 'Browser',            desc: 'Reader & live tabs, DOM marks you send into a session or save to the Vault, site credentials for agents.',   tags: 'browser web reader live tab annotation mark lightpanda credentials', doc: 'browser.md' },
    { title: 'AWS console',        desc: 'S3, SQS, EC2, Athena & EKS through the aws CLI per saved account — secrets in Keychain.',                     tags: 'aws cloud s3 sqs ec2 athena eks account sso profile',                doc: 'aws-console.md' },
    { title: 'Kubernetes console', desc: 'k9s-class view over any kubeconfig context: workloads, logs, exec, Argo Rollouts & Argo CD actions.',        tags: 'kubernetes k8s kubectl pod logs exec argo rollouts argocd helm eks',  doc: 'kubernetes-console.md' },
    { title: 'Personal Agents',    desc: 'Named personas with a pinned provider + model, schedules, memory, chat-anytime and user-visible rooms.',     tags: 'personal agent persona schedule room memory model catalog',          doc: 'personal-agents.md' },
    { title: 'Snipping Tool',      desc: 'System-wide ⌘⌃⇧2 → region select → annotate; the image is on the clipboard at every step.',                  tags: 'snip screenshot capture annotate clipboard',                          doc: 'snipping-tool.md' },
    { title: 'Multi-window',       desc: 'File → New Window (⌘⇧N): independent workspace surfaces, restored on relaunch.',                            tags: 'multi window new window restore',                                     doc: 'multi-window.md' },
    { title: 'Daemon HTTP API',    desc: 'Drive Otto programmatically over ottod — tokens, REST map, WebSocket streams.',                              tags: 'api daemon rest http token websocket programmatic',                   doc: 'daemon-http-api.md' },
  ];

  let activeIndex = $state(0);
  let videoEl: HTMLVideoElement | null = $state(null);
  let searchQuery = $state('');
  let railEl: HTMLElement | null = $state(null);
  // Per-file load state: files that failed to load (offline / blocked / 404)
  // render the "unavailable" placeholder instead of a black box. Kept as a
  // set so switching away and back doesn't re-hit a known-dead URL until the
  // user explicitly retries.
  let failed = $state(new Set<string>());
  let loading = $state(false);

  function onVideoError(file: string): void {
    failed = new Set([...failed, file]);
    loading = false;
  }

  function retry(file: string): void {
    const next = new Set(failed);
    next.delete(file);
    failed = next;
  }

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

  const filteredGuides = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return guides;
    return guides.filter((g) =>
      g.title.toLowerCase().includes(q) ||
      g.desc.toLowerCase().includes(q) ||
      g.tags.includes(q),
    );
  });

  function select(i: number): void {
    activeIndex = i;
  }

  // Auto-play whenever a new video element is bound (after key block re-mounts).
  // play() rejects when the stream can't be fetched (offline) or autoplay is
  // blocked — swallow it; the `error` event drives the placeholder state.
  $effect(() => {
    if (videoEl) {
      loading = true;
      videoEl.play().catch(() => {});
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
    // Guides without a video are reachable from ⌘K too ("Guide: AWS console").
    const guideCmds = guides.map((g) => ({
      id: `walkthrough.guide.${g.doc}`,
      title: `Guide: ${g.title}`,
      group: 'Help',
      keywords: `${g.tags} docs guide feature`,
      run: () => {
        window.open(docUrl(g.doc), '_blank', 'noopener,noreferrer');
      },
    }));
    const unreg = registry.register('walkthroughs', [...cmds, ...guideCmds]);
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
        {#if filteredGuides.length === 0}
          <div class="rail-empty dim">No matches for "{searchQuery}"</div>
        {/if}
      {/each}

      {#if filteredGuides.length > 0}
        <div class="rail-section" aria-label="Not yet covered by a video">
          <div class="rail-section-title dim">Not yet covered — read the guide</div>
          {#each filteredGuides as g (g.doc)}
            <a
              class="rail-guide"
              href={docUrl(g.doc)}
              target="_blank"
              rel="noopener noreferrer"
              title={g.desc}
            >
              <span class="item-text">
                <span class="item-title">{g.title} <span class="ext" aria-hidden="true">↗</span></span>
                <span class="item-desc">{g.desc}</span>
              </span>
            </a>
          {/each}
        </div>
      {/if}
    </div>

    <!-- Main: player -->
    <div class="player-area">
      <div class="player-meta">
        <h2 class="player-title">{current.title}</h2>
        <p class="player-desc">
          {current.desc}
          <a class="doc-link" href={docUrl(current.doc)} target="_blank" rel="noopener noreferrer">Read the guide ↗</a>
        </p>
      </div>
      {#key current.file}
        {#if failed.has(current.file)}
          <div class="video-fallback" role="status">
            <span class="fallback-icon" aria-hidden="true">▶</span>
            <p class="fallback-title">Video unavailable offline</p>
            <p class="fallback-sub">
              The walkthroughs stream from GitHub and aren't bundled with Otto.
            </p>
            <div class="fallback-actions">
              <a class="fallback-link" href={videoUrl(current.file)} target="_blank" rel="noopener noreferrer">
                Open on GitHub ↗
              </a>
              <button class="fallback-retry" onclick={() => retry(current.file)}>Retry</button>
            </div>
          </div>
        {:else}
          <div class="video-frame" class:loading>
            <!-- preload="metadata" (never "auto"): only the moov atom + first
                 frame are fetched per selected video; nothing is fetched for
                 the other 24 entries in the rail. -->
            <!-- svelte-ignore a11y_media_has_caption -->
            <video
              bind:this={videoEl}
              class="video-el"
              controls
              preload="metadata"
              playsinline
              src={videoUrl(current.file)}
              onerror={() => onVideoError(current.file)}
              onloadeddata={() => (loading = false)}
              oncanplay={() => (loading = false)}
            ></video>
            {#if loading}
              <div class="video-loading dim" aria-hidden="true">Loading…</div>
            {/if}
          </div>
        {/if}
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

  .rail-section {
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .rail-section-title {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 0 10px 4px;
  }

  .rail-guide {
    display: flex;
    padding: 7px 10px;
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    text-decoration: none;
    transition: background 120ms ease-out;
  }

  .rail-guide:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }

  .rail-guide .ext {
    color: var(--text-dim);
    font-size: 10px;
  }

  .doc-link {
    margin-inline-start: 6px;
    color: var(--accent);
    text-decoration: none;
    white-space: nowrap;
  }

  .doc-link:hover {
    text-decoration: underline;
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

  .video-frame {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
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

  /* Poster-less placeholder while metadata/first frame streams in. */
  .video-loading {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: 12px;
    color: color-mix(in srgb, #fff 55%, transparent);
    pointer-events: none;
    border-radius: var(--radius-s, 6px);
  }

  /* Offline / blocked / 404 state — same footprint as the player. */
  .video-fallback {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    text-align: center;
    padding: 24px;
    border-radius: var(--radius-s, 6px);
    border: 1px dashed var(--border);
    background: var(--surface-2);
    color: var(--text);
  }

  .fallback-icon {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 16px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    margin-bottom: 6px;
  }

  .fallback-title {
    font-size: 13.5px;
    font-weight: 600;
    margin: 0;
  }

  .fallback-sub {
    font-size: 12px;
    color: var(--text-dim);
    margin: 0;
    max-width: 380px;
  }

  .fallback-actions {
    display: flex;
    gap: 10px;
    align-items: center;
    margin-top: 10px;
  }

  .fallback-link {
    font-size: 12.5px;
    color: var(--accent);
    text-decoration: none;
    padding: 5px 10px;
    border-radius: var(--radius-s, 5px);
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  }

  .fallback-link:hover {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }

  .fallback-retry {
    font-size: 12.5px;
    padding: 5px 10px;
    border-radius: var(--radius-s, 5px);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }

  .fallback-retry:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
</style>
