// Overview: DORA tiles (band + sparkline), guardrails summary, headline flow
// numbers — every ratio shown next to the capacity it was earned in.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, fmtNum, fmtX, info, badge, bandBadge, sparkline, section, notAvailable, isNum } = TP;

  // ---- shared helpers (also used by the other views at render time) -------
  // guardFor / guardBadge / tile live in components.js (TP.*).
  const { guardFor, guardBadge, fmtShare, overGuard, mergeGuard, capContext } = TP;

  /** Below 600px a chart's data table is the primary reading — open it by default. */
  const narrow = () => {
    try {
      return typeof window.matchMedia === 'function' && window.matchMedia('(max-width: 599px)').matches;
    } catch {
      return false;
    }
  };
  const openTablesOnNarrow = (html) => (narrow() ? String(html).replace(/<details><summary>Show as table/g, '<details open><summary>Show as table') : html);

  Object.assign(TP, { narrow, openTablesOnNarrow });

  const DEFAULT_PATTERNS = ['deployed', 'hf', 'hotfix'];
  const fmtHours = (h) => (!isNum(h) ? '—' : h < 48 ? `${fmtNum(h, 1)}h` : `${fmtNum(h / 24, 1)}d`);
  const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;

  const DORA = [
    {
      keys: ['deployment_frequency', 'deploy_frequency'],
      gids: ['deploymentFrequency', 'dora.deploy_frequency'],
      title: 'Deployment frequency',
      fmt: (m) => `${fmtNum(m.value, 2)}<small> ${esc(m.unit || 'per week')}</small>`,
      ctx: (m) => (isNum(m.total) ? plural(m.total, 'deploy') + ' in period' : ''),
      definition: 'How often code reaches production. A git tag counts as a deployment when its name contains "deployed", "hf" or "hotfix" (configurable in Settings). Same-day tags in one repo count once.',
      formula: 'deploy events in period ÷ weeks in period',
    },
    {
      keys: ['lead_time', 'lead_time_days', 'lead_time_for_changes'],
      gids: ['leadTime', 'dora.lead_time'],
      title: 'Lead time for changes',
      fmt: (m) => fmtD(m.value),
      ctx: (m) => (isNum(m.n) ? `median of ${plural(m.n, 'deployed ticket')}` : ''),
      definition: 'Time from the first commit of a ticket to the first deployment that contains it (median, business days).',
      formula: 'median(first deploy tag − first commit)',
    },
    {
      keys: ['change_failure_rate', 'cfr'],
      gids: ['changeFailureRate', 'dora.change_failure_rate'],
      title: 'Change failure rate',
      fmt: (m) => TP.shareHtml(m.value),
      ctx: (m) => (isNum(m.failed) && isNum(m.total) ? `${m.failed} of ${plural(m.total, 'deploy')} failed` : ''),
      definition: 'Share of deployments followed by a hotfix tag or a bug on the same code within the failure window.',
      formula: 'failed deployments ÷ deployments',
      direction: 'Lower is better.',
      share: true,
    },
    {
      keys: ['mttr', 'mttr_days', 'time_to_restore'],
      gids: ['mttr', 'dora.mttr'],
      title: 'Time to restore',
      fmt: (m) => (m.unit === 'hours' ? fmtHours(m.value) : fmtD(m.value)),
      ctx: (m) => (isNum(m.n) ? `median of ${plural(m.n, 'incident')}` : ''),
      definition: 'How long it takes to recover from a failed change: failure signal → next hotfix/fix deployment (median, wall-clock).',
      formula: 'median(restore deploy − failure signal)',
    },
  ];

  const pick = (obj, keys) => {
    if (!obj) return null;
    for (const k of keys) if (obj[k] != null) return M(obj[k]);
    return null;
  };

  /** Overview tile → TP.tile with the metric's definition as its ⓘ. */
  const tile = ({ def, ...rest }) => TP.tile({ ...rest, def: def ? { definition: def.definition, formula: def.formula, direction: def.direction } : null });
  /** Ticket list behind a metric, when the payload carries one. */
  const drillOf = (m, valueLabel) => {
    const list = m && (m.tickets || m.items || m.evidence);
    return Array.isArray(list) && list.length ? { tickets: list.map((t) => (typeof t === 'string' ? { key: t } : { key: t.key || t.ref || t.tag, summary: t.summary || t.title, value: t.value ?? t.days ?? t.hours, note: t.note || t.reason || t.kind })), valueLabel } : null;
  };

  /** Deploy frequency with no matching tags: say why and where to fix it. Never a band. */
  function deployNotAvailable(d, m, app) {
    const cfg = (app && app.config) || {};
    const patterns = (m && m.patterns) || d.patterns || cfg.deploy_tag_patterns || DEFAULT_PATTERNS;
    const reposRaw = (m && m.repos) || d.repos || Object.keys((m && m.by_repo) || {});
    const repos = (Array.isArray(reposRaw) ? reposRaw : []).map((r) => (typeof r === 'string' ? r : r && (r.name || r.path))).filter(Boolean);
    const branch = (m && (m.branch || m.target_ref)) || d.branch || d.target_ref || null;
    const reason = (m && m.reason) || d.reason || 'No git tag in this period matched a deployment pattern.';
    return `<div class="tile">
      <div class="label">Deployment frequency ${info({ title: 'Deployment frequency', definition: DORA[0].definition, formula: DORA[0].formula, quality: { level: 'bad', msg: reason } })}</div>
      <div class="row"><span class="value na">Not available</span></div>
      <p class="context">${esc(reason)}</p>
      <dl class="small dim">
        <dt>Tag contains</dt><dd class="mono">${esc([].concat(patterns).join(', '))}</dd>
        <dt>Repos</dt><dd>${repos.length ? esc(repos.slice(0, 6).join(', ')) + (repos.length > 6 ? ` +${repos.length - 6}` : '') : 'none scanned'}</dd>
        <dt>Branch</dt><dd class="mono">${esc(branch || 'origin/develop (default)')}</dd>
      </dl>
      <button type="button" class="compact" data-goto="settings" data-focus="cfg-tags">Check deploy tags in Settings</button>
    </div>`;
  }

  function doraTiles(o, app) {
    const d = o.dora;
    if (!d) return notAvailable('DORA metrics');
    return `<div class="tiles">${DORA.map((x, i) => {
      const m = pick(d, x.keys);
      const guard = mergeGuard(guardFor(o, x.gids, m), x.share && m ? overGuard(m.value, x.title) : null);
      if (i === 0 && (d.status === 'not_available' || (m && m.status === 'not_available'))) return deployNotAvailable(d, m, app);
      if (!m || !isNum(m.value)) {
        return tile({ title: x.title, valueHtml: '<span class="na">not available yet</span>', def: x, quality: m && m.quality, guard, context: m && m.reason ? esc(m.reason) : '' });
      }
      return tile({ title: x.title, valueHtml: x.fmt(m), band: m.band, series: m.series, context: x.ctx(m), def: x, quality: m.quality, guard, drill: drillOf(m, x.drillLabel) });
    }).join('')}</div>`;
  }

  function guardrailsHtml(o) {
    const g = Array.isArray(o.guardrails) ? o.guardrails : [];
    if (!g.length) return '<p class="dim">No guardrail checks reported for this scope yet.</p>';
    const bad = g.filter((x) => x.level !== 'ok');
    const head = bad.length
      ? `<div class="banner ${bad.some((x) => x.level === 'bad') ? 'danger' : ''}" role="note">${TP.icon('warn')}<span><b>${bad.length} input check${bad.length > 1 ? 's' : ''} failing.</b> Metrics marked with these inputs may not reflect reality — read the ⓘ on each number before comparing people or periods.</span></div>`
      : `<div class="banner info" role="note">${TP.icon('check')}<span>All input checks pass for this scope.</span></div>`;
    const tone = (l) => (l === 'bad' ? 'danger' : l === 'warn' ? 'warning' : 'success');
    const label = (l) => (l === 'bad' ? 'weak' : l === 'warn' ? 'check' : 'ok');
    return `${head}<ul class="guardrails">${g
      .map((x) => `<li>${badge(tone(x.level), label(x.level))}<span>${esc(x.msg)}${x.metric ? ` <span class="dim">· affects ${esc(x.metric)}</span>` : ''}</span></li>`)
      .join('')}</ul>`;
  }

  /** Estimates inflated more than 10% vs the reference period → the adjusted pace is the headline. */
  const inflated = (est) => isNum(est.est_inflation) && est.est_inflation > 1.1 && isNum(est.pace_adjusted);

  // ---- second-tier delivery signals (lib/flow, lib/prs, lib/dora) -------------
  const fmtHrs = (h) => (!isNum(h) ? '—' : h < 48 ? `${fmtNum(h, 1)}h` : `${fmtNum(h / 24, 1)}d`);
  const nCtx = (m, what) => (m && isNum(m.n) ? `n=${m.n} ${what}` : '');
  /** [{title, src(o) → raw, fmt, share, ctx, definition, formula, direction, gids, cap}] */
  const SIGNALS = [
    {
      title: 'Flow efficiency',
      src: (o) => o.flow && (o.flow.flow_efficiency || o.flow.flowEfficiency),
      share: true,
      ctx: (m) => nCtx(m, 'tickets'),
      definition: 'Share of a ticket’s cycle time spent actively worked (dev, in review) rather than waiting (review wait, QA wait, deploy wait).',
      formula: 'Σ active phase days ÷ Σ cycle days',
      direction: 'Higher is better; 15–40% is typical.',
      gids: ['flowEfficiency', 'cycleTimeByPhase'],
    },
    {
      title: 'Focus',
      src: (o) => o.flow && (o.flow.focus_share || o.flow.focusShare || o.flow.focus),
      share: true,
      ctx: (m) => (m && isNum(m.focused_days) && isNum(m.n) ? `${m.focused_days} of ${m.n} person-days on one ticket` : nCtx(m, 'person-days')),
      definition: 'Share of person-days with commits on a single ticket. Low focus means context switching between tickets.',
      formula: 'person-days with 1 distinct key ÷ person-days with commits',
      direction: 'Higher is better.',
      gids: ['focusShare', 'contextSwitching'],
    },
    {
      title: 'Escape rate',
      src: (o) => o.flow && (o.flow.escape_rate || o.flow.escapeRate),
      share: true,
      ctx: (m) => (m && isNum(m.escaped) && isNum(m.n) ? `${m.escaped} of ${m.n} delivered tickets got a bug after delivery` : ''),
      definition: 'Share of delivered tickets that a later bug was traced back to (Jira links, titles or git blame).',
      formula: 'tickets with an escaped bug ÷ delivered tickets',
      direction: 'Lower is better.',
      gids: ['escapeRate', 'rework'],
      drillLabel: 'Bugs',
    },
    {
      title: 'Sprint plan accuracy',
      src: (o) => {
        const sp = o.flow && (o.flow.sprint_planning || o.flow.sprintPlanning);
        return sp && sp.value && typeof sp.value === 'object' ? { ...sp, value: sp.value.accuracy, creep: sp.value.scope_creep } : sp;
      },
      share: true,
      ctx: (m) => (m && isNum(m.creep) ? `scope creep ${TP.shareHtml(m.creep)} added after sprint start` : nCtx(m, 'sprints')),
      definition: 'Share of tickets planned at sprint start that were done by sprint end. Scope creep = tickets added after the start ÷ planned.',
      formula: 'planned & done ÷ planned at start',
      direction: 'Higher is better; read with scope creep.',
      gids: ['sprintPlanning'],
    },
    {
      title: 'Review load spread',
      src: (o) => {
        const rl = o.pr_flow && (o.pr_flow.review_load || o.pr_flow.reviewLoad);
        return rl ? { ...rl, value: rl.top_share ?? rl.value } : null;
      },
      share: true,
      ctx: (m) => (m && isNum(m.reviewers_n) ? `top reviewer’s share of ${m.n} reviews, across ${m.reviewers_n} reviewers` : ''),
      definition: 'How concentrated code review is: the share of all reviews done by the busiest reviewer. High = one person is a review bottleneck.',
      formula: 'max(reviews by one person) ÷ all reviews',
      direction: 'Lower is better (review work is spread).',
      gids: ['reviewLoad', 'prReview'],
    },
    {
      title: 'People in PRs',
      src: (o) => {
        const pp = o.pr_flow && (o.pr_flow.pr_people || o.pr_flow.people || o.pr_flow.by_person);
        if (!pp) return null;
        const rows = Object.values(pp);
        return { value: rows.filter((r) => r && r.authored > 0).length, reviewers: rows.filter((r) => r && r.reviewed_given > 0).length, n: rows.length };
      },
      fmt: (m) => fmtNum(m.value, 0),
      ctx: (m) => (m && isNum(m.reviewers) ? `${m.value} authored · ${m.reviewers} reviewed — see People for per-capacity-day rates` : ''),
      definition: 'Distinct people who authored merged PRs and who reviewed or commented on others’ PRs. Per-person counts are divided by capacity days on the People tab.',
      formula: 'count(distinct authors) · count(distinct reviewers)',
      gids: ['prPeople', 'prReview'],
    },
    {
      title: 'Hotfix rate',
      src: (o) => o.dora && (o.dora.hotfix_rate || o.dora.hotfixRate),
      share: true,
      ctx: (m) => (m && isNum(m.hotfixes) && isNum(m.total) ? `${m.hotfixes} of ${m.total} deploys were hotfixes` : ''),
      definition: 'Share of deployments whose tag marks a hotfix ("hf" / "hotfix"). Hotfixes count as deployments too.',
      formula: 'hotfix deploys ÷ all deploys',
      direction: 'Lower is better.',
      gids: ['hotfixRate', 'deploymentFrequency'],
    },
    {
      title: 'Batch size',
      src: (o) => o.dora && (o.dora.batch_size || o.dora.batchSize),
      fmt: (m) => `${fmtNum(m.value, 1)}<small> tickets / deploy</small>`,
      ctx: (m) => nCtx(m, 'deploys'),
      definition: 'Tickets shipped per deployment (median). Smaller batches mean less risk per release and faster feedback.',
      formula: 'median(tickets first deployed by each deploy)',
      direction: 'Lower is better.',
      gids: ['batchSize', 'deploymentFrequency'],
    },
    {
      title: 'Time to detect',
      src: (o) => o.dora && (o.dora.time_to_detect || o.dora.detect),
      fmt: (m) => fmtHrs(m.value),
      ctx: (m) => nCtx(m, 'incidents'),
      definition: 'Failed deploy → the first failure signal (bug filed or hotfix started), median wall-clock hours.',
      formula: 'median(failure signal − deploy)',
      direction: 'Lower is better.',
      gids: ['timeToDetect', 'mttr'],
    },
    {
      title: 'Time to restore',
      src: (o) => o.dora && (o.dora.time_to_restore || o.dora.mttr),
      fmt: (m) => fmtHrs(m.value),
      ctx: (m) => nCtx(m, 'incidents'),
      definition: 'Failure signal → the hotfix / fix deployment that restored service, median wall-clock hours.',
      formula: 'median(restore deploy − failure signal)',
      direction: 'Lower is better.',
      gids: ['mttr'],
      drillLabel: 'Hours',
    },
    {
      title: 'Deploys per capacity day',
      src: (o) => o.dora && (o.dora.deploys_per_capacity_day || o.dora.deploy_per_capacity || (o.dora.deploy_frequency && o.dora.deploy_frequency.per_capacity_day)),
      fmt: (m) => fmtNum(m.value, 2),
      cap: true,
      ctx: () => 'capacity-normalised: holidays and time off do not read as fewer deploys',
      definition: 'Deployments divided by the team’s capacity person-days, so a holiday-heavy period is comparable with a full one.',
      formula: 'deploys ÷ capacity person-days',
      direction: 'Higher is better.',
      gids: ['deploymentFrequency'],
    },
  ];

  function signalTiles(o) {
    const cap = o.capacity && (o.capacity.team || o.capacity);
    const tiles = SIGNALS.map((x) => {
      const m = M(x.src(o));
      const def = { definition: x.definition, formula: x.formula, direction: x.direction };
      const guard = mergeGuard(guardFor(o, x.gids, m), x.share && m ? overGuard(m.value, x.title) : null);
      if (!m || !isNum(m.value)) return tile({ title: x.title, valueHtml: '<span class="na">not available yet</span>', def, guard, context: m && m.reason ? esc(m.reason) : '' });
      return tile({
        title: x.title,
        valueHtml: x.fmt ? x.fmt(m) : x.share ? TP.shareHtml(m.value) : fmtNum(m.value),
        context: x.ctx(m),
        capCtx: x.cap ? capContext(cap, 'capacity person-days') : '',
        def,
        guard,
        drill: drillOf(m, x.drillLabel),
      });
    });
    return `<div class="tiles">${tiles.join('')}</div>`;
  }

  function flowTiles(o) {
    const sc = o.scope || {};
    const cap = o.capacity && (o.capacity.team || o.capacity);
    const capDays = cap && isNum(cap.capacity_days) ? cap.capacity_days : null;
    const capCtx = capDays != null
      ? `over ${fmtNum(capDays, 0)} capacity person-days${isNum(cap.business_days) ? ` (${fmtNum(cap.business_days, 0)} working − ${fmtNum(cap.time_off_days || 0, 0)} off)` : ''}`
      : 'capacity not available yet — not comparable across periods';
    const est = o.est_basis || {};
    const tiles = [
      tile({
        title: 'Completed tickets',
        valueHtml: `${o.completed ?? 0}${o.excluded ? `<small> ${o.excluded} excluded</small>` : ''}`,
        context: `${o.open ?? 0} still open`,
        def: { definition: 'Tickets finished in the period. Excluded = outliers or stale tickets kept out of medians.', formula: 'count(done in period)' },
      }),
      tile({
        title: 'Median cycle time',
        valueHtml: fmtD(sc.median_cycle_days),
        context: 'first work → done, business days',
        guard: guardFor(o, ['cycleTimeByPhase']),
        def: { definition: 'Typical elapsed working time per ticket, from first real work (commit or In Progress) to done. See Flow for the phase split.', formula: 'median(done − start), business days' },
      }),
      tile({
        title: 'Delivered scope',
        valueHtml: isNum(sc.weighted_throughput_wk) ? `${fmtNum(sc.weighted_throughput_wk)}<small> est-d / week</small>` : '—',
        context: capCtx,
        def: {
          definition: 'Estimated ideal days of work delivered per week — fairer than ticket counts. Rework tickets are not counted as new scope. Read together with capacity: fewer available days means less delivered scope, not lower productivity.',
          formula: 'Σ estimate(done tickets, credited by commit share) ÷ weeks',
        },
        guard: guardFor(o, ['throughputPerWeek']),
      }),
      tile({
        title: inflated(est) ? 'Pace vs estimate (inflation-adjusted)' : 'Pace vs estimate',
        valueHtml: fmtX(inflated(est) ? est.pace_adjusted : est.pace),
        context: inflated(est)
          ? `estimates grew ${fmtX(est.est_inflation)} vs the previous period, so the adjusted pace is the headline · raw ${fmtX(est.pace)} · prev ${fmtX(est.pace_ref)}`
          : isNum(est.pace_adjusted) ? `inflation-adjusted ${fmtX(est.pace_adjusted)} (prev ${fmtX(est.pace_ref)})` : 'actual dev-days ÷ estimated days',
        guard: guardFor(o, ['estimateAccuracy']),
        def: {
          definition: 'Elapsed working days per estimated ideal day. Above ×1 = slower than estimated. When estimates themselves drift between periods, trend the adjusted value.',
          formula: 'Σ actual ÷ Σ estimate (estimated tickets only)',
        },
      }),
      tile({
        title: 'Fix rate',
        valueHtml: TP.shareHtml(sc.fix_rate),
        context: 'tickets needing fix commits after done',
        guard: mergeGuard(guardFor(o, ['rework']), overGuard(sc.fix_rate, 'Fix rate')),
        def: { definition: 'Share of done tickets that got fix commits within the fix window after delivery.', formula: 'tickets with fixes ÷ done tickets' },
      }),
    ];
    return `<div class="tiles">${tiles.join('')}</div>`;
  }

  TP.views.overview = {
    SIGNALS,
    signalTiles,
    doraTiles,
    guardrailsHtml,
    flowTiles,
    render(outer, { o, app }) {
      // Own wrapper per render so delegated listeners never pile up on #view.
      const host = document.createElement('div');
      outer.appendChild(host);
      host.addEventListener('click', (e) => {
        const b = e.target.closest && e.target.closest('[data-goto]');
        if (!b || !app) return;
        app.settingsFocus = b.dataset.focus || null;
        app.goTab(b.dataset.goto);
      });
      if (o.capped) host.insertAdjacentHTML('beforeend', `<div class="banner" role="note">${TP.icon('warn')}<span>An earlier scan hit the issue cap, so history is incomplete. Run a full rescan from the ⋯ menu.</span></div>`);
      section(host, {
        title: 'DORA',
        sub: 'Delivery speed and stability, from deploy tags and git. Bands follow the DORA research tiers.',
        load: async () => doraTiles(o, app),
      });
      section(host, {
        title: 'Input checks',
        infoDef: {
          title: 'Input checks',
          definition: 'Guardrails test whether the data behind each metric is strong enough to trust: coverage of estimates, git signal, capacity entries, sample sizes.',
          formula: 'per-metric coverage / sample-size thresholds',
        },
        sub: 'When an input is weak, the metric still shows, but treat it as indicative only.',
        load: async () => guardrailsHtml(o),
      });
      section(host, {
        title: 'Flow at a glance',
        sub: 'Ratios are never a productivity score on their own — each tile names the capacity it was earned in.',
        load: async () => flowTiles(o),
        after: () => {},
      });
      section(host, {
        title: 'More delivery signals',
        sub: 'Flow efficiency, focus, quality escapes, sprint planning, review load and deploy shape. Each ⓘ says which direction is better.',
        load: async () => signalTiles(o),
      });
      host.insertAdjacentHTML(
        'beforeend',
        `<details class="notes"><summary>How to read this page</summary><ul>
          <li>Every number has an ⓘ with its definition, formula and how good its inputs are.</li>
          <li>“Not available yet” means the scan hasn’t computed it — it is never a zero.</li>
          <li>Compare people only on the People tab, where each ratio sits next to that person’s available days (holidays and time off removed).</li>
        </ul></details>`,
      );
    },
  };
})();
