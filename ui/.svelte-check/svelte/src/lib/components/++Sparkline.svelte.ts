///<reference types="svelte" />
;;
  // A tiny trend line for stat tiles (dataviz "stat tile" contract): the run
  // in the de-emphasis ink (--text-dim), the current point in --accent. One
  // series only, so no legend — the tile's label names it. Every point has a
  // hover target bigger than the mark with a native tooltip ("label: value"),
  // and the whole thing carries an aria-label summary, so the trend is never
  // colour-only or pointer-only.
  //
  // `values` may contain nulls (a period with no reading): the line breaks
  // there instead of inventing a value.

  interface Props {
    values: (number | null)[];
    /** One label per value, used in the tooltips ("Wed 23 Sep"). */
    labels?: string[];
    /** Formats a value for tooltips + the aria summary. */
    format?: (v: number) => string;
    /** What the line shows, for the aria summary ("Sessions"). */
    name: string;
    width?: number;
    height?: number;
    /** Index to mark as "current" (default: the last one). */
    current?: number;
  };function $$render() {


  let { values, labels = [], format = (v) => String(v), name, width = 96, height = 28, current }: Props = $props();

  const PAD = 3;
  const cur = $derived(current ?? values.length - 1);

  const geom = $derived.by(() => {
    const nums = values.filter((v): v is number => v != null && Number.isFinite(v));
    if (nums.length === 0) return null;
    let lo = Math.min(...nums);
    let hi = Math.max(...nums);
    if (lo === hi) {
      lo -= 1;
      hi += 1;
    }
    const n = values.length;
    const x = (i: number) => (n <= 1 ? width / 2 : PAD + (i * (width - PAD * 2)) / (n - 1));
    const y = (v: number) => PAD + (1 - (v - lo) / (hi - lo)) * (height - PAD * 2);
    // Split into runs at nulls so gaps stay gaps.
    const runs: string[] = [];
    let run: string[] = [];
    values.forEach((v, i) => {
      if (v == null || !Number.isFinite(v)) {
        if (run.length) runs.push(run.join(' '));
        run = [];
      } else run.push(`${x(i).toFixed(1)},${y(v).toFixed(1)}`);
    });
    if (run.length) runs.push(run.join(' '));
    const ok = (i: number) => i >= 0 && i < n && values[i] != null && Number.isFinite(values[i] as number);
    const pts = values.map((v, i) =>
      v == null || !ok(i) ? null : { x: x(i), y: y(v), v, i, lone: !ok(i - 1) && !ok(i + 1) },
    );
    return { runs, pts };
  });

  const summary = $derived.by(() => {
    const nums = values.filter((v): v is number => v != null && Number.isFinite(v));
    if (nums.length === 0) return `${name}: no data`;
    const first = nums[0];
    const last = nums[nums.length - 1];
    return `${name} over ${values.length} reports: from ${format(first)} to ${format(last)} (low ${format(Math.min(...nums))}, high ${format(Math.max(...nums))})`;
  });
;
async () => {

 { svelteHTML.createElement("svg", {         "class":`spark`,width,height,"viewBox":`0 0 ${width} ${height}`,"role":`img`,"aria-label":summary,});
  if(geom){
        for(let r of __sveltets_2_ensureArray(geom.runs)){let i = 1;i;
      if(r.includes(' ')){
         { svelteHTML.createElement("polyline", {    "points":r,"class":`line`,});}
      }
    }
        for(let p of __sveltets_2_ensureArray(geom.pts)){let i = 1;i;
      if(p){
        if(p.i === cur){
           { svelteHTML.createElement("circle", {        "cx":p.x,"cy":p.y,"r":`3`,"class":`now`,});}
        } else if (p.lone){
          
           { svelteHTML.createElement("circle", {        "cx":p.x,"cy":p.y,"r":`1.5`,"class":`lone`,});}
        }
         { svelteHTML.createElement("circle", {       "cx":p.x,"cy":p.y,"r":`6`,"class":`hit`,}); { svelteHTML.createElement("title", {});labels[p.i] ? `${labels[p.i]}: ` : '';format(p.v); } }
      }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Sparkline__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Sparkline__SvelteComponent_ = ReturnType<typeof Sparkline__SvelteComponent_>;
/*Ωignore_endΩ*/export default Sparkline__SvelteComponent_;