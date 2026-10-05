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

  /** One masking badge everywhere (list + viewer): names visible is a warning, never neutral. */
  const maskBadge = (r) =>
    r && (r.masked || r.mask)
      ? badge('info', 'names masked', 'People’s names are replaced in this report')
      : badge('warning', 'names visible', 'People’s names appear in this report — check who you share it with');
  const reportTitle = (r) => `${['team', 'combined'].includes(r.report_scope || 'dev') ? 'Team' : r.assignee_name || 'Person'} — ${r.label}`;
  const q = (app, id) => `account=${encodeURIComponent(app.account)}&id=${encodeURIComponent(id)}`;
  const inlineError = (msg, cls = 'retry') =>
    `<div class="inline-error" role="alert">${icon('warn')}<span>${esc(msg)}</span><button type="button" class="compact ${cls}">Retry</button></div>`;

  function commentsHtml(list) {
    if (!list.length) return '<p class="dim small">No comments yet. Notes here are saved with the report for everyone who can open it.</p>';
    return `<ol class="rv-comments">${list
      .map((c) => `<li><div class="small"><b>${esc(c.author || 'lead')}</b> <span class="dim">${esc(fmtDate(c.at))}${c.label ? ` · ${esc(c.label)}` : ''}</span></div><p>${esc(c.text)}</p></li>`)
      .join('')}</ol>`;
  }

  /** Comment thread beside the report: GET/POST /report/comments via api(). */
  function mountComments(app, id, aside) {
    const list = aside.querySelector('.rv-list');
    const load = async () => {
      list.innerHTML = TP.skeleton(2, false);
      try {
        const c = await api(`/report/comments?${q(app, id)}`);
        list.innerHTML = commentsHtml((c && c.comments) || []);
      } catch (e) {
        list.innerHTML = inlineError(`Couldn’t load comments: ${e.message}`, 'rv-c-retry');
        list.querySelector('.rv-c-retry').onclick = load;
      }
    };
    const form = aside.querySelector('form');
    form.onsubmit = async (e) => {
      e.preventDefault();
      const ta = form.querySelector('textarea');
      const text = ta.value.trim();
      if (!text) return ta.focus();
      const btn = form.querySelector('button[type=submit]');
      btn.disabled = true;
      try {
        await post(`/report/comments?${q(app, id)}`, { text, author: 'lead' });
        ta.value = '';
        toast('Comment added.', 'success');
        await load();
      } catch (err) {
        toast(`Couldn’t post the comment: ${err.message}`, 'danger');
      } finally {
        btn.disabled = false;
      }
    };
    load();
    return load;
  }

  async function openReport(app, id) {
    const m = modal({
      wide: true,
      body: `<div style="display:flex;flex-wrap:wrap;gap:var(--sp-3);align-items:center;margin-block-end:var(--sp-4)">
          <h2 id="rv-title" style="margin-inline-end:auto">Report</h2>
          <span id="rv-badge"></span>
          <button type="button" class="compact" id="rv-dl" disabled>${icon('download')}Download</button>
          <button type="button" class="compact icon" id="rv-close" aria-label="Close report" title="Close (Esc)">${icon('x')}</button>
        </div>
        <div id="rv-content" style="min-block-size:0;display:flex;flex-wrap:wrap;gap:var(--sp-4)"></div>`,
      labelledBy: 'rv-title',
    });
    m.el.style.gridTemplateRows = '1fr';
    const body = m.el.querySelector('.modal-body');
    body.style.display = 'grid';
    body.style.gridTemplateRows = 'auto 1fr';
    body.style.minBlockSize = '0';
    const content = m.el.querySelector('#rv-content');
    m.el.querySelector('#rv-close').onclick = () => m.close();
    let r = null;
    const load = async () => {
      content.innerHTML = TP.skeleton(8);
      content.setAttribute('aria-busy', 'true');
      try {
        r = await api(`/report/html?${q(app, id)}`);
      } catch (e) {
        content.removeAttribute('aria-busy');
        content.innerHTML = inlineError(`Couldn’t open the report: ${e.message}`);
        content.querySelector('.retry').onclick = load;
        return;
      }
      content.removeAttribute('aria-busy');
      const title = reportTitle(r);
      m.el.querySelector('#rv-title').textContent = title;
      m.el.querySelector('#rv-badge').innerHTML = maskBadge(r);
      m.el.querySelector('#rv-dl').disabled = false;
      content.innerHTML = `<iframe title="${esc(title)}" sandbox="allow-popups" style="flex:3 1 420px;min-block-size:60vh"></iframe>
        <aside class="rv-aside" aria-label="Comments" style="flex:1 1 240px;min-inline-size:0;display:flex;flex-direction:column;gap:var(--sp-3)">
          <h3>Comments</h3>
          <div class="rv-list"></div>
          <form><label class="field"><span>Add a comment</span><textarea rows="3" maxlength="4000"></textarea></label>
            <button type="submit" class="compact">Post comment</button></form>
        </aside>`;
      content.querySelector('iframe').srcdoc = r.html;
      mountComments(app, id, content.querySelector('aside'));
    };
    m.el.querySelector('#rv-dl').onclick = async () => {
      if (!r) return;
      const title = reportTitle(r);
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
    await load();
    return m;
  }

  /** Same scope/period/masking as the saved entry → a fresh report job. */
  async function regenerate(app, r, onChange) {
    const ok = await confirmer.ask({
      title: 'Regenerate this report?',
      message: `“${reportTitle(r)}” will be rebuilt from today’s data with the same scope, period and masking${r.masked ? ' (names masked)' : ' (names visible)'}. The new report is saved next to the old one for everyone with access to this Otto workspace.`,
      confirmLabel: 'Regenerate',
    });
    if (!ok) return;
    try {
      const start = await post('/report', {
        account: app.account,
        scope: r.report_scope || 'dev',
        assignee: r.report_scope === 'dev' || !r.report_scope ? r.assignee_id || undefined : undefined,
        kind: r.kind,
        year: r.year,
        month: r.month || undefined,
        quarter: r.quarter || undefined,
        mask: Boolean(r.masked || r.mask),
        mask_tasks: Boolean(r.mask_tasks),
        sections: Array.isArray(r.sections) ? r.sections : undefined,
      });
      toast(start.already ? 'Already generating — following its progress.' : `Regenerating ${r.label}…`);
      onChange();
      pollJob(app, start.job, () => onChange());
    } catch (e) {
      toast(`Couldn’t regenerate: ${e.message}`, 'danger');
    }
  }

  async function removeReport(app, r, onChange) {
    const ok = await confirmer.ask({
      title: 'Delete this report?',
      message: `“${reportTitle(r)}” and its comments will be removed for everyone. This can’t be undone; you can generate the period again later.`,
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await api(`/report?${q(app, r.id)}`, { method: 'DELETE' });
      toast('Report deleted.', 'success');
      onChange();
    } catch (e) {
      toast(`Couldn’t delete the report: ${e.message}`, 'danger');
    }
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
    maskBadge,
    commentsHtml,
    render(host, { app }) {
      clearInterval(activeT);
      let lastList = [];
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
          lastList = reps;
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
            cols: [{ label: 'Report' }, { label: 'Scope' }, { label: 'Masking' }, { label: 'Created', num: true }, { label: 'Actions' }],
            rows: reps.map((r, i) => [
              esc(reportTitle(r)),
              esc(r.report_scope === 'combined' ? 'Team + people' : r.report_scope === 'team' ? 'Team' : 'Person'),
              maskBadge(r),
              fmtDate(r.created_at),
              `<span class="chips"><button type="button" class="compact" data-rep="${esc(r.id)}">Open</button>
                <button type="button" class="compact" data-regen="${i}" aria-label="Regenerate ${esc(reportTitle(r))}" title="Regenerate with today’s data">${icon('refresh')}Regenerate</button>
                <button type="button" class="compact danger" data-del="${i}" aria-label="Delete ${esc(reportTitle(r))}" title="Delete report">${icon('x')}Delete</button></span>`,
            ]),
          });
        },
        after(body, rerun) {
          const reps = lastList;
          body.querySelectorAll('[data-rep]').forEach((b) => (b.onclick = () => openReport(app, b.dataset.rep)));
          body.querySelectorAll('[data-regen]').forEach((b) => (b.onclick = () => regenerate(app, reps[+b.dataset.regen], rerun)));
          body.querySelectorAll('[data-del]').forEach((b) => (b.onclick = () => removeReport(app, reps[+b.dataset.del], rerun)));
        },
      });
      host.querySelector('#rp-new').onclick = () => newReport(app);
    },
  };
})();
