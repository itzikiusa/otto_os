// Reports: saved report history, live "generating now" jobs, one New report
// modal (scope, period, masking, sections, share confirmation) and a viewer
// dialog with a sandboxed iframe (no scripts; popups only for Jira links).
'use strict';
(function () {
  const TP = window.TP;
  const { esc, api, post, section, modal, confirmer, toast, fmtDate, badge, icon } = TP;
  const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const SECTIONS = [
    ['dora', 'DORA'],
    ['flow', 'Flow and phases'],
    ['pr', 'Pull requests'],
    ['quality', 'Quality and rework'],
    ['investment', 'Investment'],
    ['people', 'People and capacity'],
    ['estimates', 'Estimates'],
    ['guardrails', 'Input checks'],
  ];

  let activeT = null;

  async function openReport(app, id) {
    let r;
    try {
      r = await api(`/report/html?account=${encodeURIComponent(app.account)}&id=${encodeURIComponent(id)}`);
    } catch (e) {
      toast(`Couldn’t open the report: ${e.message}`, 'danger');
      return;
    }
    const title = `${r.assignee_name || 'Team'} — ${r.label}`;
    const m = modal({
      wide: true,
      body: `<div style="display:flex;flex-wrap:wrap;gap:var(--sp-3);align-items:center;margin-block-end:var(--sp-4)">
          <h2 id="rv-title" style="margin-inline-end:auto">${esc(title)}</h2>
          ${r.masked ? badge('info', 'names masked') : badge('warning', 'names visible')}
          <button type="button" class="compact" id="rv-dl">${icon('download')}Download</button>
          <button type="button" class="compact icon" id="rv-close" aria-label="Close report" title="Close (Esc)">${icon('x')}</button>
        </div>
        <iframe title="${esc(title)}" sandbox="allow-popups"></iframe>`,
      labelledBy: 'rv-title',
    });
    m.el.style.gridTemplateRows = '1fr';
    const body = m.el.querySelector('.modal-body');
    body.style.display = 'grid';
    body.style.gridTemplateRows = 'auto 1fr';
    body.style.minBlockSize = '0';
    m.el.querySelector('iframe').srcdoc = r.html;
    m.el.querySelector('#rv-close').onclick = () => m.close();
    m.el.querySelector('#rv-dl').onclick = async () => {
      const ok = await confirmer.ask({
        title: 'Download this report?',
        message: `The file "${title}" will be saved to your computer. Anyone you forward it to can read every number in it${r.masked ? '; names are masked' : ', including people’s names'}. Ticket links point to your Jira and need Jira access to open.`,
        confirmLabel: 'Download',
      });
      if (!ok) return;
      const a = document.createElement('a');
      a.href = URL.createObjectURL(new Blob([r.html], { type: 'text/html' }));
      a.download = `${title}.html`.replace(/[\s/]+/g, '-');
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 2000);
    };
  }

  function pollJob(app, job, onDone) {
    const started = Date.now();
    let seen = false;
    const t = setInterval(async () => {
      let s;
      try {
        s = await api(`/report/status?job=${encodeURIComponent(job)}`);
      } catch {
        return;
      }
      if (s.state === 'running') seen = true;
      else if (s.state === 'done') {
        clearInterval(t);
        toast('Report ready.', 'success');
        onDone(s.report);
      } else if (s.state === 'error') {
        clearInterval(t);
        toast(`Report failed: ${s.error || 'unknown error'}`, 'danger');
        onDone(null);
      } else if (s.state === 'idle' && (seen || Date.now() - started > 8000)) {
        clearInterval(t);
        toast('Report generation was interrupted — the plugin restarted. Please generate it again.', 'danger');
        onDone(null);
      }
    }, 2000);
  }

  /** The single New report modal. preset: {scope, assignee}. */
  function newReport(app, preset = {}) {
    const people = Object.entries(app.people)
      .filter(([, p]) => !p.merged_into && p.included !== false)
      .sort((a, b) => a[1].name.localeCompare(b[1].name));
    const now = new Date();
    const pm = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - 1, 1));
    const years = Array.from({ length: 7 }, (_, k) => now.getUTCFullYear() - k);
    const scope = preset.scope || 'team';
    const m = modal({
      title: 'New report',
      body: `<div class="form-grid">
          <label class="field"><span>Scope</span><select id="nr-scope">
            <option value="team" ${scope === 'team' ? 'selected' : ''}>Team overview</option>
            <option value="combined" ${scope === 'combined' ? 'selected' : ''}>Team + every person</option>
            <option value="dev" ${scope === 'dev' ? 'selected' : ''}>One person</option></select></label>
          <label class="field" id="nr-person-wrap"><span>Person</span><select id="nr-person">${people.map(([id, p]) => `<option value="${esc(id)}" ${id === preset.assignee ? 'selected' : ''}>${esc(p.name)}</option>`).join('')}</select></label>
          <label class="field"><span>Period</span><select id="nr-kind"><option value="month">Month</option><option value="quarter">Quarter</option><option value="year">Year</option></select></label>
          <label class="field" id="nr-month-wrap"><span>Month</span><select id="nr-month">${MONTHS.map((x, i) => `<option value="${i + 1}" ${i === pm.getUTCMonth() ? 'selected' : ''}>${x}</option>`).join('')}</select></label>
          <label class="field" id="nr-q-wrap" hidden><span>Quarter</span><select id="nr-q"><option value="1">Q1</option><option value="2">Q2</option><option value="3">Q3</option><option value="4">Q4</option></select></label>
          <label class="field"><span>Year</span><select id="nr-year">${years.map((y) => `<option ${y === pm.getUTCFullYear() ? 'selected' : ''}>${y}</option>`).join('')}</select></label>
        </div>
        <fieldset><legend>Masking</legend>
          <label class="check"><input type="checkbox" id="nr-mask"> Mask people’s names</label>
          <label class="check"><input type="checkbox" id="nr-mask-tasks"> Also mask ticket keys and titles</label>
        </fieldset>
        <fieldset><legend>Sections</legend><div class="form-grid">${SECTIONS.map(([k, l]) => `<label class="check"><input type="checkbox" name="nr-sec" value="${k}" checked> ${esc(l)}</label>`).join('')}</div></fieldset>
        <div class="banner info" role="note" id="nr-share">${icon('info')}<span></span></div>
        <label class="check"><input type="checkbox" id="nr-ack"> I understand who will be able to see this report</label>`,
      actions: [{ label: 'Cancel', value: null }, { label: 'Generate', value: 'go', primary: true }],
    });
    const el = m.el;
    const $ = (s) => el.querySelector(s);
    const gen = el.querySelector('footer .primary');
    const sync = () => {
      const sc = $('#nr-scope').value;
      const k = $('#nr-kind').value;
      $('#nr-person-wrap').hidden = sc !== 'dev';
      $('#nr-month-wrap').hidden = k !== 'month';
      $('#nr-q-wrap').hidden = k !== 'quarter';
      const masked = $('#nr-mask').checked;
      const who = sc === 'dev' ? 'one person’s' : sc === 'combined' ? 'the team’s and every person’s' : 'the team’s';
      $('#nr-share span').textContent = `The report contains ${who} delivery metrics${masked ? ' with names masked' : ' with real names'}. It is saved in this plugin for anyone with access to this Otto workspace, and can be downloaded and forwarded.`;
      gen.disabled = !$('#nr-ack').checked || (sc === 'dev' && !$('#nr-person').value);
    };
    el.addEventListener('change', sync);
    sync();
    // Read values at click time, before the modal is torn down.
    let body = null;
    el.addEventListener('click', (e) => {
      if (e.target.closest('footer button') !== gen) return;
      const kind = $('#nr-kind').value;
      body = {
        account: app.account,
        scope: $('#nr-scope').value,
        assignee: $('#nr-scope').value === 'dev' ? $('#nr-person').value : undefined,
        kind,
        year: parseInt($('#nr-year').value, 10),
        month: kind === 'month' ? parseInt($('#nr-month').value, 10) : undefined,
        quarter: kind === 'quarter' ? parseInt($('#nr-q').value, 10) : undefined,
        mask: $('#nr-mask').checked,
        mask_tasks: $('#nr-mask-tasks').checked,
        sections: [...el.querySelectorAll('[name=nr-sec]:checked')].map((c) => c.value),
      };
    }, true);
    m.done.then(async (act) => {
      if (act !== 'go' || !body) return;
      try {
        const start = await post('/report', body);
        toast(start.already ? 'Already generating — following its progress.' : `Generating ${start.label || 'report'}…`);
        if (app.tab === 'reports' && !app.person) app.render();
        pollJob(app, start.job, (rep) => {
          if (app.tab === 'reports' && !app.person) app.render();
          if (rep) openReport(app, rep.id);
        });
      } catch (e) {
        toast(`Couldn’t start the report: ${e.message}`, 'danger');
      }
    });
  }

  TP.views.reports = {
    newReport,
    openReport,
    render(host, { app }) {
      clearInterval(activeT);
      section(host, {
        title: 'Reports',
        headerEnd: '<button type="button" class="compact" id="rp-new">New report</button>',
        sub: 'Agent-written HTML reports: team or person, month / quarter / year, with every metric, phase breakdowns, rework, input checks and Jira links.',
        load: async () => {
          const [list, active] = await Promise.all([
            api(`/reports?account=${encodeURIComponent(app.account)}`),
            api(`/reports/active?account=${encodeURIComponent(app.account)}`).catch(() => ({ active: [] })),
          ]);
          const reps = list.reports || [];
          const act = active.active || [];
          let html = '';
          if (act.length) {
            html += `<div class="banner info" role="status" aria-live="polite">${icon('refresh')}<span><b>Generating now:</b> ${act.map((a) => `${esc(a.label)} <span class="dim">(${esc(a.step)}, ${TP.fmtSecs(Date.now() - a.started_at)})</span>`).join(' · ')}</span></div>`;
            activeT = setTimeout(() => app.tab === 'reports' && !app.person && app.render(), 5000);
          }
          if (!reps.length) {
            return html + TP.emptyState({ title: 'No reports yet', body: 'Generate a team report, or open a person on the People tab for a personal one.', steps: [{ label: 'Choose scope and period', done: false }, { label: 'Pick masking and sections', done: false }, { label: 'Generate and review before sharing', done: false }] });
          }
          return html + TP.table({
            caption: `${reps.length} saved reports, newest first`,
            cols: [{ label: 'Report' }, { label: 'Scope' }, { label: 'Masking' }, { label: 'Created', num: true }, { label: '' }],
            rows: reps.map((r) => [
              esc(`${['team', 'combined'].includes(r.report_scope || 'dev') ? 'Team' : r.assignee_name} — ${r.label}`),
              esc(r.report_scope === 'combined' ? 'Team + people' : r.report_scope === 'team' ? 'Team' : 'Person'),
              r.masked || r.mask ? badge('info', 'masked') : badge('', 'names visible'),
              fmtDate(r.created_at),
              `<button type="button" class="compact" data-rep="${esc(r.id)}">Open</button>`,
            ]),
          });
        },
        after(body) {
          body.querySelectorAll('[data-rep]').forEach((b) => (b.onclick = () => openReport(app, b.dataset.rep)));
        },
      });
      host.querySelector('#rp-new').onclick = () => newReport(app);
    },
  };
})();
