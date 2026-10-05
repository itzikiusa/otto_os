// Overview: DORA tiles (band + sparkline), guardrails summary, headline flow
// numbers — every ratio shown next to the capacity it was earned in.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, fmtNum, fmtX, info, badge, bandBadge, sparkline, section, notAvailable, isNum } = TP;

  const DORA = [
    {
      keys: ['deployment_frequency', 'deploy_frequency'],
      title: 'Deployment frequency',
      fmt: (m) => `${fmtNum(m.value, 2)}<small> ${esc(m.unit || 'per week')}</small>`,
      definition: 'How often code reaches production. A git tag counts as a deployment when its name contains "deployed", "hf" or "hotfix" (configurable in Settings).',
      formula: 'deploy tags in period ÷ weeks in period',
    },
    {
      keys: ['lead_time', 'lead_time_days', 'lead_time_for_changes'],
      title: 'Lead time for changes',
      fmt: (m) => fmtD(m.value),
      definition: 'Time from the first commit of a ticket to the first deployment that contains it (median).',
      formula: 'median(first deploy tag − first commit)',
    },
    {
      keys: ['change_failure_rate', 'cfr'],
      title: 'Change failure rate',
      fmt: (m) => fmtPct(m.value),
      definition: 'Share of deployments followed by a hotfix tag or a bug on the same code soon after delivery.',
      formula: 'failed deployments ÷ deployments',
    },
    {
      keys: ['mttr', 'mttr_days', 'time_to_restore'],
      title: 'Time to restore',
      fmt: (m) => fmtD(m.value),
      definition: 'How long it takes to recover from a failed change: failing deployment → next hotfix/fix deployment (median).',
      formula: 'median(restore deploy − failing deploy)',
    },
  ];

  const pick = (obj, keys) => {
    if (!obj) return null;
    for (const k of keys) if (obj[k] != null) return M(obj[k]);
    return null;
  };

  function tile({ title, valueHtml, band, series, context, def, quality }) {
    return `<div class="tile">
      <div class="label">${esc(title)} ${info({ title, ...def, quality })}</div>
      <div class="row"><span class="value">${valueHtml}</span>${series ? sparkline(series, title) : ''}</div>
      <div class="row">${band ? bandBadge(band) : '<span></span>'}${context ? `<span class="context">${context}</span>` : ''}</div>
    </div>`;
  }

  function doraTiles(o) {
    const d = o.dora;
    if (!d) return notAvailable('DORA metrics');
    return `<div class="tiles">${DORA.map((x) => {
      const m = pick(d, x.keys);
      if (!m || !isNum(m.value)) return tile({ title: x.title, valueHtml: '<span class="na">not available yet</span>', def: x, quality: m && m.quality });
      return tile({
        title: x.title,
        valueHtml: x.fmt(m),
        band: m.band,
        series: m.series,
        context: isNum(m.n) ? `n=${m.n}` : '',
        def: x,
        quality: m.quality,
      });
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
    const capCtx = capDays != null ? `over ${fmtNum(capDays, 0)} available person-days${isNum(cap.time_off_days) && cap.time_off_days ? ` (${fmtNum(cap.time_off_days, 0)} off)` : ''}` : 'capacity not available yet';
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
        quality: o.guardrails && o.guardrails.find((g) => /estimate/i.test(g.msg || '') && g.level !== 'ok'),
      }),
      tile({
        title: 'Pace vs estimate',
        valueHtml: fmtX(est.pace),
        context: isNum(est.pace_adjusted) ? `inflation-adjusted ${fmtX(est.pace_adjusted)} (prev ${fmtX(est.pace_ref)})` : 'actual days per estimated day',
        def: {
          definition: 'Elapsed working days per estimated ideal day. Above ×1 = slower than estimated. When estimates themselves drift between periods, trend the adjusted value.',
          formula: 'Σ actual ÷ Σ estimate (estimated tickets only)',
        },
      }),
      tile({
        title: 'Fix rate',
        valueHtml: fmtPct(sc.fix_rate),
        context: 'tickets needing fix commits after done',
        def: { definition: 'Share of done tickets that got fix commits within the fix window after delivery.', formula: 'tickets with fixes ÷ done tickets' },
      }),
    ];
    return `<div class="tiles">${tiles.join('')}</div>`;
  }

  TP.views.overview = {
    render(host, { o }) {
      if (o.capped) host.insertAdjacentHTML('beforeend', `<div class="banner" role="note">${TP.icon('warn')}<span>An earlier scan hit the issue cap, so history is incomplete. Run a full rescan from the ⋯ menu.</span></div>`);
      section(host, {
        title: 'DORA',
        sub: 'Delivery speed and stability, from deploy tags and git. Bands follow the DORA research tiers.',
        load: async () => doraTiles(o),
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
        sub: 'Ratios are never productivity on their own — each tile names the capacity it was earned in.',
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
