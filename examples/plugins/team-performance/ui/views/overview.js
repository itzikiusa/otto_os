// Overview: DORA tiles (band + sparkline), guardrails summary, headline flow
// numbers — every ratio shown next to the capacity it was earned in.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, fmtNum, fmtX, info, badge, bandBadge, sparkline, section, notAvailable, isNum } = TP;

  // ---- shared helpers (also used by the other views at render time) -------

  /**
   * Guardrail for a metric, looked up by CANONICAL metric id — never by
   * matching message text. Sources, in order: the per-metric badge map
   * (guardrail_badges: {metricId: {severity, codes, reasons}}), then banner
   * entries whose `metric` (or `id` = "code:metric") equals one of the ids.
   * → {level:'bad'|'warn', msg, codes} or null.
   */
  function guardFor(o, ids) {
    if (!o) return null;
    const want = [].concat(ids || []).filter(Boolean);
    if (!want.length) return null;
    const badges = o.guardrail_badges || {};
    for (const id of want) {
      const b = badges[id];
      if (b) return { level: b.severity === 'danger' ? 'bad' : 'warn', msg: (b.reasons || []).join(' '), codes: b.codes || [] };
    }
    const metricOf = (g) => g.metric || (typeof g.id === 'string' && g.id.includes(':') ? g.id.slice(g.id.indexOf(':') + 1) : null);
    const hits = (Array.isArray(o.guardrails) ? o.guardrails : []).filter((g) => g && g.level !== 'ok' && want.includes(metricOf(g)));
    if (!hits.length) return null;
    return { level: hits.some((g) => g.level === 'bad') ? 'bad' : 'warn', msg: hits.map((g) => g.msg).filter(Boolean).join(' '), codes: hits.map((g) => g.code || g.id) };
  }
  const guardBadge = (g) => (g ? badge(g.level === 'bad' ? 'danger' : 'warning', g.level === 'bad' ? 'weak inputs' : 'check inputs', g.msg) : '');

  /** Below 600px a chart's data table is the primary reading — open it by default. */
  const narrow = () => {
    try {
      return typeof window.matchMedia === 'function' && window.matchMedia('(max-width: 599px)').matches;
    } catch {
      return false;
    }
  };
  const openTablesOnNarrow = (html) => (narrow() ? String(html).replace(/<details><summary>Show as table/g, '<details open><summary>Show as table') : html);

  /** Share that may arrive as 0..1 or 0..100 → always 0..1 for fmtPct. */
  const share01 = (v) => (isNum(v) ? (v > 1 ? v / 100 : v) : null);

  Object.assign(TP, { guardFor, guardBadge, narrow, openTablesOnNarrow, share01 });

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
      fmt: (m) => fmtPct(share01(m.value)),
      ctx: (m) => (isNum(m.failed) && isNum(m.total) ? `${m.failed} of ${plural(m.total, 'deploy')} failed` : ''),
      definition: 'Share of deployments followed by a hotfix tag or a bug on the same code within the failure window.',
      formula: 'failed deployments ÷ deployments',
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

  function tile({ title, valueHtml, band, series, context, def, quality, guard }) {
    return `<div class="tile">
      <div class="label">${esc(title)} ${info({ title, ...def, quality: guard || quality })}</div>
      <div class="row"><span class="value">${valueHtml}</span>${series ? sparkline(series, title) : ''}</div>
      <div class="row">${band ? bandBadge(band) : guard ? guardBadge(guard) : '<span></span>'}${context ? `<span class="context">${context}</span>` : ''}</div>
      ${band && guard ? `<div class="row">${guardBadge(guard)}</div>` : ''}
    </div>`;
  }

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
      const guard = guardFor(o, x.gids);
      if (i === 0 && (d.status === 'not_available' || (m && m.status === 'not_available'))) return deployNotAvailable(d, m, app);
      if (!m || !isNum(m.value)) {
        return tile({ title: x.title, valueHtml: '<span class="na">not available yet</span>', def: x, quality: m && m.quality, guard, context: m && m.reason ? esc(m.reason) : '' });
      }
      return tile({ title: x.title, valueHtml: x.fmt(m), band: m.band, series: m.series, context: x.ctx(m), def: x, quality: m.quality, guard });
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
        title: 'Pace vs estimate',
        valueHtml: fmtX(est.pace),
        context: isNum(est.pace_adjusted) ? `inflation-adjusted ${fmtX(est.pace_adjusted)} (prev ${fmtX(est.pace_ref)})` : 'actual dev-days ÷ estimated days',
        guard: guardFor(o, ['estimateAccuracy']),
        def: {
          definition: 'Elapsed working days per estimated ideal day. Above ×1 = slower than estimated. When estimates themselves drift between periods, trend the adjusted value.',
          formula: 'Σ actual ÷ Σ estimate (estimated tickets only)',
        },
      }),
      tile({
        title: 'Fix rate',
        valueHtml: fmtPct(sc.fix_rate),
        context: 'tickets needing fix commits after done',
        guard: guardFor(o, ['rework']),
        def: { definition: 'Share of done tickets that got fix commits within the fix window after delivery.', formula: 'tickets with fixes ÷ done tickets' },
      }),
    ];
    return `<div class="tiles">${tiles.join('')}</div>`;
  }

  TP.views.overview = {
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
