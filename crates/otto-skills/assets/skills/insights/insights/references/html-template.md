# HTML template (extended)

The base report is the original `/insights` layout (sections, CSS, bar-chart pattern, Time-of-
Day selector). This file keeps that verbatim and adds the four Otto extensions:
**(a) provider switcher**, **(b) Trend section**, **(c) Action Plan section**, **(d) every
narrative section rewritten action-first** per `actionability-contract.md`.

Render order: title → stats row → **Trend** → **Action Plan** → **Combined** view (all the
original sections) → **per-provider** views behind the switcher. Self-contained: inline
CSS/JS, `file://`-openable. Fill `assets/report-skeleton.html`.

## Section order (combined view = the original)

1. Title + subtitle (period label + provider mix + stat summary)
2. **At a Glance** (golden box) — What's working / What's hindering / Quick wins / Ambitious workflows — every bullet action-first
3. Nav TOC
4. Stats row — messages, lines (+added/-removed), files, active days, msgs/day
5. **Trend** (new — see below)
6. **Action Plan** (new — see below)
7. **What You Work On** — project-area cards
8. Charts: What You Wanted (goal_categories) + Top Tools (tool_counts)
9. Charts: Languages + Session Types
10. **How You Use Claude Code** — narrative + key-insight box
11. Response Time Distribution + median/average
12. Multi-Clauding (if `overlap_events > 0`)
13. Charts: Time of Day + Tool Errors
14. **Impressive Things You Did** — big-win cards (each with a next-level action)
15. Charts: What Helped Most + Outcomes
16. **Where Things Go Wrong** — friction cards (each with an action + target)
17. Charts: Primary Friction Types + Inferred Satisfaction
18. Fun Ending

Per-provider views repeat the **applicable** sections only: Claude gets all of them; Codex/agy
get the behavioral subset (stats, tools, languages-if-any, hours, response time) plus a banner
stating no facet/narrative signal is available.

## CSS (copy verbatim into `<style>`, then append the extension CSS)

```css
* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif; background: #f8fafc; color: #334155; line-height: 1.65; padding: 48px 24px; }
.container { max-width: 800px; margin: 0 auto; }
h1 { font-size: 32px; font-weight: 700; color: #0f172a; margin-bottom: 8px; }
h2 { font-size: 20px; font-weight: 600; color: #0f172a; margin-top: 48px; margin-bottom: 16px; }
.subtitle { color: #64748b; font-size: 15px; margin-bottom: 32px; }
.nav-toc { display: flex; flex-wrap: wrap; gap: 8px; margin: 24px 0 32px 0; padding: 16px; background: white; border-radius: 8px; border: 1px solid #e2e8f0; }
.nav-toc a { font-size: 12px; color: #64748b; text-decoration: none; padding: 6px 12px; border-radius: 6px; background: #f1f5f9; transition: all 0.15s; }
.nav-toc a:hover { background: #e2e8f0; color: #334155; }
.stats-row { display: flex; gap: 24px; margin-bottom: 40px; padding: 20px 0; border-top: 1px solid #e2e8f0; border-bottom: 1px solid #e2e8f0; flex-wrap: wrap; }
.stat { text-align: center; }
.stat-value { font-size: 24px; font-weight: 700; color: #0f172a; }
.stat-label { font-size: 11px; color: #64748b; text-transform: uppercase; }
.at-a-glance { background: linear-gradient(135deg, #fef3c7 0%, #fde68a 100%); border: 1px solid #f59e0b; border-radius: 12px; padding: 20px 24px; margin-bottom: 32px; }
.glance-title { font-size: 16px; font-weight: 700; color: #92400e; margin-bottom: 16px; }
.glance-sections { display: flex; flex-direction: column; gap: 12px; }
.glance-section { font-size: 14px; color: #78350f; line-height: 1.6; }
.glance-section strong { color: #92400e; }
.project-areas { display: flex; flex-direction: column; gap: 12px; margin-bottom: 32px; }
.project-area { background: white; border: 1px solid #e2e8f0; border-radius: 8px; padding: 16px; }
.area-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
.area-name { font-weight: 600; font-size: 15px; color: #0f172a; }
.area-count { font-size: 12px; color: #64748b; background: #f1f5f9; padding: 2px 8px; border-radius: 4px; }
.area-desc { font-size: 14px; color: #475569; line-height: 1.5; }
.narrative { background: white; border: 1px solid #e2e8f0; border-radius: 8px; padding: 20px; margin-bottom: 24px; }
.narrative p { margin-bottom: 12px; font-size: 14px; color: #475569; line-height: 1.7; }
.key-insight { background: #f0fdf4; border: 1px solid #bbf7d0; border-radius: 8px; padding: 12px 16px; margin-top: 12px; font-size: 14px; color: #166534; }
.section-intro { font-size: 14px; color: #64748b; margin-bottom: 16px; }
.big-wins { display: flex; flex-direction: column; gap: 12px; margin-bottom: 24px; }
.big-win { background: #f0fdf4; border: 1px solid #bbf7d0; border-radius: 8px; padding: 16px; }
.big-win-title { font-weight: 600; font-size: 15px; color: #166534; margin-bottom: 8px; }
.big-win-desc { font-size: 14px; color: #15803d; line-height: 1.5; }
.friction-categories { display: flex; flex-direction: column; gap: 16px; margin-bottom: 24px; }
.friction-category { background: #fef2f2; border: 1px solid #fca5a5; border-radius: 8px; padding: 16px; }
.friction-title { font-weight: 600; font-size: 15px; color: #991b1b; margin-bottom: 6px; }
.friction-desc { font-size: 13px; color: #7f1d1d; margin-bottom: 10px; }
.friction-examples { margin: 0 0 0 20px; font-size: 13px; color: #334155; }
.friction-examples li { margin-bottom: 4px; }
.charts-row { display: grid; grid-template-columns: 1fr 1fr; gap: 24px; margin: 24px 0; }
.chart-card { background: white; border: 1px solid #e2e8f0; border-radius: 8px; padding: 16px; }
.chart-title { font-size: 12px; font-weight: 600; color: #64748b; text-transform: uppercase; margin-bottom: 12px; }
.bar-row { display: flex; align-items: center; margin-bottom: 6px; }
.bar-label { width: 120px; font-size: 11px; color: #475569; flex-shrink: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.bar-track { flex: 1; height: 6px; background: #f1f5f9; border-radius: 3px; margin: 0 8px; }
.bar-fill { height: 100%; border-radius: 3px; }
.bar-value { width: 36px; font-size: 11px; font-weight: 500; color: #64748b; text-align: right; }
.empty { color: #94a3b8; font-size: 13px; }
.fun-ending { background: linear-gradient(135deg, #fef3c7 0%, #fde68a 100%); border: 1px solid #fbbf24; border-radius: 12px; padding: 24px; margin-top: 40px; text-align: center; }
.fun-headline { font-size: 18px; font-weight: 600; color: #78350f; margin-bottom: 8px; }
.fun-detail { font-size: 14px; color: #92400e; }
@media (max-width: 640px) { .charts-row { grid-template-columns: 1fr; } .stats-row { justify-content: center; } }

/* ---- Otto extensions ---- */
/* provider switcher */
.provider-tabs { display: flex; gap: 8px; margin: 28px 0 8px; flex-wrap: wrap; }
.provider-tab { font-size: 13px; font-weight: 600; color: #475569; background: #f1f5f9; border: 1px solid #e2e8f0; border-radius: 8px; padding: 8px 16px; cursor: pointer; }
.provider-tab.active { background: #0f172a; color: #fff; border-color: #0f172a; }
.provider-tab .depth { font-size: 10px; font-weight: 500; opacity: 0.7; margin-left: 6px; text-transform: uppercase; }
.provider-view { display: none; }
.provider-view.active { display: block; }
.provider-banner { background: #eff6ff; border: 1px solid #bfdbfe; border-radius: 8px; padding: 10px 14px; font-size: 13px; color: #1e40af; margin: 12px 0 8px; }
.provider-banner.basic { background: #fffbeb; border-color: #fde68a; color: #92400e; }
/* finding card (the four-part shape) */
.finding { background: white; border: 1px solid #e2e8f0; border-left: 4px solid #64748b; border-radius: 8px; padding: 14px 16px; margin-bottom: 12px; }
.finding.problem { border-left-color: #dc2626; }
.finding.good { border-left-color: #16a34a; }
.finding-title { font-weight: 600; font-size: 14px; color: #0f172a; margin-bottom: 6px; }
.finding dl { display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; font-size: 13px; }
.finding dt { font-weight: 600; color: #64748b; }
.finding dd { color: #334155; margin: 0; }
/* trend */
.trend-table { width: 100%; border-collapse: collapse; font-size: 13px; background: white; border: 1px solid #e2e8f0; border-radius: 8px; overflow: hidden; }
.trend-table th, .trend-table td { padding: 8px 12px; text-align: left; border-bottom: 1px solid #f1f5f9; }
.trend-table th { font-size: 11px; text-transform: uppercase; color: #64748b; background: #f8fafc; }
.delta-up { color: #16a34a; font-weight: 600; }     /* improved */
.delta-down { color: #dc2626; font-weight: 600; }   /* regressed */
.delta-flat { color: #94a3b8; font-weight: 600; }   /* flat */
.trend-empty { background: #f8fafc; border: 1px dashed #cbd5e1; border-radius: 8px; padding: 16px; color: #64748b; font-size: 14px; }
/* action plan */
.action-plan { display: flex; flex-direction: column; gap: 10px; margin-bottom: 24px; }
.action-item { background: white; border: 1px solid #e2e8f0; border-radius: 8px; padding: 14px 16px; }
.action-rank { display: inline-block; width: 22px; height: 22px; line-height: 22px; text-align: center; background: #0f172a; color: #fff; border-radius: 50%; font-size: 12px; font-weight: 700; margin-right: 8px; }
.action-meta { font-size: 12px; color: #64748b; margin-top: 6px; }
.action-meta .target { color: #16a34a; font-weight: 600; }
.action-status { font-size: 11px; text-transform: uppercase; padding: 2px 8px; border-radius: 999px; margin-left: 8px; }
.status-open { background: #fef3c7; color: #92400e; }
.status-improved { background: #dcfce7; color: #166534; }
.status-closed { background: #e2e8f0; color: #475569; }
```

## Provider switcher markup

```html
<div class="provider-tabs">
  <button class="provider-tab active" onclick="showProvider('combined')">Combined</button>
  <button class="provider-tab" onclick="showProvider('claude')">claude<span class="depth">full</span></button>
  <button class="provider-tab" onclick="showProvider('codex')">codex<span class="depth">basic</span></button>
  <!-- include agy only if present, or render it disabled with the empty-note -->
  <button class="provider-tab" onclick="showProvider('agy')">agy<span class="depth">basic</span></button>
</div>

<section id="view-combined" class="provider-view active"> ...combined sections... </section>
<section id="view-claude"   class="provider-view"> ...claude sections (all)... </section>
<section id="view-codex"    class="provider-view">
  <div class="provider-banner basic">Codex has no facets — quantitative/behavioral metrics only (no goal/friction/outcome narrative).</div>
  ...behavioral sections...
</section>
<section id="view-agy" class="provider-view">
  <div class="provider-banner basic">No agy/gemini sessions found this period (transcript path unconfirmed).</div>
</section>
```

```javascript
function showProvider(id) {
  document.querySelectorAll('.provider-view').forEach(v => v.classList.remove('active'));
  document.querySelectorAll('.provider-tab').forEach(t => t.classList.remove('active'));
  document.getElementById('view-' + id).classList.add('active');
  event.target.closest('.provider-tab').classList.add('active');
}
```

## Trend section markup

If `previous_metrics_path` is null:
```html
<h2 id="trend">Trend</h2>
<div class="trend-empty">No prior period to compare — this is the first stored <em>KIND</em> report. Next period will show ▲/▼/▬ deltas here.</div>
```

Otherwise, one row per headline metric. `▲` = improved (good direction), `▼` = regressed,
`▬` = flat. Remember direction: fewer tool errors / faster response = ▲.
```html
<h2 id="trend">Trend <span style="font-size:13px;color:#64748b">vs PREV_LABEL</span></h2>
<table class="trend-table">
  <tr><th>Metric</th><th>Last</th><th>This</th><th>Δ</th></tr>
  <tr><td>Messages</td><td>532</td><td>955</td><td class="delta-up">▲ +423</td></tr>
  <tr><td>Achievement rate</td><td>78%</td><td>71%</td><td class="delta-down">▼ −7pp</td></tr>
  <tr><td>Median response</td><td>44s</td><td>44s</td><td class="delta-flat">▬ 0</td></tr>
  <tr><td>Tool errors</td><td>31</td><td>12</td><td class="delta-up">▲ −19</td></tr>
</table>
```

## Action Plan markup

Top ~5 by impact × effort. Status comes from the `index.json` action-ledger (carried-forward
items show improved/closed; new items are open).
```html
<h2 id="action-plan">Action Plan</h2>
<div class="action-plan">
  <div class="action-item">
    <span class="action-rank">1</span><strong>Split multi-goal sessions at the goal boundary</strong>
    <span class="action-status status-open">open</span>
    <div class="action-meta">Targets <b>median session length</b> · 132 msgs → <span class="target">&lt; 90 msgs</span> · effort S</div>
  </div>
  <!-- ...up to 5... carried-forward items render with status-improved / status-closed -->
</div>
```

## Finding card markup (the four-part shape — use everywhere narrative appears)

```html
<div class="finding problem">
  <div class="finding-title">Sessions run long</div>
  <dl>
    <dt>Evidence</dt><dd>median 132 msgs / 48 min vs the 90-msg / 35-min threshold; 8 of 21 over both.</dd>
    <dt>Action</dt><dd>At ~80 msgs write a one-paragraph handoff, <code>/clear</code>, restart from it; split multi-goal sessions.</dd>
    <dt>Effect</dt><dd>Median back under 90 msgs / 35 min; fewer "lost the thread" restarts.</dd>
  </dl>
</div>
<div class="finding good">
  <div class="finding-title">High landing rate (working — level it up)</div>
  <dl>
    <dt>Evidence</dt><dd>82% fully/mostly achieved (threshold 75%), 14 commits.</dd>
    <dt>Action</dt><dd>The 18% that stalled were all otto-server refactors — pre-write a 3-line acceptance check before those.</dd>
    <dt>Effect</dt><dd>Push 82% → 90%; cut stall-and-restart on refactors.</dd>
  </dl>
</div>
```

## Bar-chart pattern (unchanged from the original)

Width = `(value / maxValue) * 100%`.
```html
<div class="bar-row">
  <div class="bar-label">LABEL</div>
  <div class="bar-track"><div class="bar-fill" style="width:WIDTH%;background:COLOR"></div></div>
  <div class="bar-value">VALUE</div>
</div>
```
Colors: goals `#2563eb` · tools `#0891b2` · languages `#10b981` · session types `#8b5cf6` ·
helped-most `#16a34a` · outcomes `#8b5cf6` · response time `#6366f1` · time-of-day `#8b5cf6` ·
friction `#dc2626` · satisfaction `#eab308` · tool errors `#dc2626` · helpfulness `#8b5cf6`.

## Time-of-Day chart (timezone-aware, unchanged)

Embed `charts.hour_counts` as a JS `rawHourCounts` object and a timezone `<select>` that
re-renders the histogram. Default to the detected `ISRAEL_UTC_OFFSET` (set `selected` on the
matching `<option>` and pass it to the initial `renderHourChart(ISRAEL_UTC_OFFSET)`).

## copyText helper (unchanged)

```javascript
function copyText(btn){const c=btn.previousElementSibling;navigator.clipboard.writeText(c.textContent).then(()=>{btn.textContent='Copied!';setTimeout(()=>btn.textContent='Copy',2000);});}
```
