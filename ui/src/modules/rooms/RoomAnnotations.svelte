<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { RoomAction, RoomSnapshot, RoomPresentation, RoomAnnotationTool, RoomPoint } from '../../lib/api/room-types';
  import { normalizedPoint } from './room-state';
  let { room, source, send, children }: {room: RoomSnapshot; source: RoomPresentation; send: (action: RoomAction) => boolean; children: Snippet} = $props();
  let tool = $state<RoomAnnotationTool>('pointer'), drawing = $state(false), points = $state<RoomPoint[]>([]);
  let strokeEpoch = 0, grantEpoch = 0, now = $state(Date.now());
  const grant = $derived(room.annotation_grants?.find(g => g.source_id === source.id && g.member_id === room.member_id));
  const allowed = $derived(room.annotations_enabled !== false && grant?.allowed && !grant.blocked);
  const owner = $derived(source.member_id === room.member_id);
  const host = $derived(room.host_member_id === room.member_id);
  const marks = $derived((room.annotations ?? []).filter(a => a.source_id === source.id && a.source_generation === source.generation && a.clear_epoch === source.clear_epoch && Date.parse(a.expires_at) > now));
  $effect(() => { const timer = setInterval(() => now = Date.now(), 500); return () => clearInterval(timer); });
  // Keyboard placement (Point / Highlight): a focusable surface with a visible cursor.
  let kb = $state<{x: number, y: number} | null>(null), anchor = $state<RoomPoint | null>(null), kbNote = $state('');
  const keyboardTool = $derived(tool !== 'pen');
  $effect(() => { if (!allowed) { drawing = false; points = []; kb = null; anchor = null; kbNote = ''; } });
  $effect(() => { void tool; anchor = null; kbNote = ''; });
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  function kbFocus() { if (allowed && !kb) kb = {x: 0.5, y: 0.5}; }
  function kbBlur() { kb = null; anchor = null; kbNote = ''; }
  function kbSend(pts: RoomPoint[]) {
    if (!allowed) return;
    send({type: 'annotation', source_id: source.id, source_generation: source.generation, clear_epoch: source.clear_epoch, grant_epoch: grant?.epoch ?? 0, tool, points: pts});
  }
  function kbKey(event: KeyboardEvent) {
    if (!allowed || !kb || event.altKey || event.ctrlKey || event.metaKey) return;
    const step = event.shiftKey ? 0.1 : 0.02;
    const d = event.key === 'ArrowLeft' ? [-step, 0] : event.key === 'ArrowRight' ? [step, 0] : event.key === 'ArrowUp' ? [0, -step] : event.key === 'ArrowDown' ? [0, step] : null;
    if (d) { event.preventDefault(); kb = {x: clamp(kb.x + d[0]), y: clamp(kb.y + d[1])}; return; }
    if (event.key === 'Enter') {
      event.preventDefault();
      if (tool === 'pointer') { kbSend([{...kb}]); kbNote = 'Pointer placed.'; }
      else if (!anchor) { anchor = {...kb}; kbNote = 'First corner set. Move to the opposite corner and press Enter.'; }
      else { kbSend([anchor, {...kb}]); anchor = null; kbNote = 'Highlight placed.'; }
    } else if (event.key === 'Escape' && anchor) { event.preventDefault(); event.stopPropagation(); anchor = null; kbNote = 'Placement canceled.'; }
  }
  function position(event: PointerEvent) { return normalizedPoint(event.clientX, event.clientY, event.currentTarget instanceof SVGElement ? event.currentTarget.getBoundingClientRect() : {left: 0, top: 0, width: 0, height: 0}, source.width, source.height); }
  function begin(event: PointerEvent) {
    if (!allowed || event.button !== 0) return;
    const p = position(event); if (!p) return;
    event.preventDefault(); (event.currentTarget as SVGElement).setPointerCapture(event.pointerId);
    strokeEpoch = source.clear_epoch; grantEpoch = grant?.epoch ?? 0; points = [p]; drawing = true;
  }
  function move(event: PointerEvent) {
    if (!drawing || !allowed || points.length >= 64 || tool === 'pointer') return;
    const p = position(event); if (p) points = tool === 'highlight' ? [points[0], p] : [...points, p];
  }
  function finish() {
    if (drawing && allowed && points.length && (tool !== 'highlight' || points.length === 2)) send({type: 'annotation', source_id: source.id, source_generation: source.generation, clear_epoch: strokeEpoch, grant_epoch: grantEpoch, tool, points});
    drawing = false; points = [];
  }
  function polyline(value: RoomPoint[]) { return value.map(p => `${p.x * source.width},${p.y * source.height}`).join(' '); }
</script>
<div class="media-stage">
  {@render children()}
<svg class="annotation-overlay" class:can-draw={allowed} viewBox={`0 0 ${source.width} ${source.height}`} aria-label={`Annotations on ${source.title}`} role="img" onpointerdown={begin} onpointermove={move} onpointerup={finish} onpointercancel={() => { drawing = false; points = []; }}>
  {#each marks as mark (mark.id)}
    {#if mark.tool === 'pointer' && mark.points[0]}<circle cx={mark.points[0].x * source.width} cy={mark.points[0].y * source.height} r={Math.max(6, source.width / 100)}><title>{room.members?.find(m => m.id === mark.member_id)?.name ?? 'Participant'}’s pointer</title></circle>
    {:else if mark.tool === 'highlight' && mark.points.length === 2}<rect class="highlight" x={Math.min(mark.points[0].x, mark.points[1].x) * source.width} y={Math.min(mark.points[0].y, mark.points[1].y) * source.height} width={Math.abs(mark.points[1].x - mark.points[0].x) * source.width} height={Math.abs(mark.points[1].y - mark.points[0].y) * source.height} />{:else}<polyline points={polyline(mark.points)} />{/if}
  {/each}
  {#if drawing}<polyline points={polyline(points)} />{/if}
  {#if kb && keyboardTool}
    {#if anchor}<rect class="highlight kb-rect" x={Math.min(anchor.x, kb.x) * source.width} y={Math.min(anchor.y, kb.y) * source.height} width={Math.abs(kb.x - anchor.x) * source.width} height={Math.abs(kb.y - anchor.y) * source.height} />{/if}
    <circle class="kb-cursor" cx={kb.x * source.width} cy={kb.y * source.height} r={Math.max(8, source.width / 80)} />
  {/if}
</svg>
{#if allowed && keyboardTool}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div class="place-surface" tabindex="0" role="application" aria-label={tool === 'pointer' ? `Place a pointer on ${source.title}` : `Place a highlight on ${source.title}`} aria-describedby={`place-help-${source.id}`} onfocus={kbFocus} onblur={kbBlur} onkeydown={kbKey}></div>
  <span class="sr-only" id={`place-help-${source.id}`}>Arrow keys move the cursor, Shift for larger steps. {tool === 'pointer' ? 'Enter places the pointer.' : 'Enter sets the first corner, then Enter again completes the highlight.'} Escape cancels. Tab leaves.</span>
  <span class="sr-only" role="status" aria-live="polite">{kbNote}</span>
{/if}
</div>
<div class="annotation-tools">
  {#if allowed}<div class="tools" role="group" aria-label="Annotation tool">
    {#each ['pointer', 'highlight', 'pen'] as value}<button class="btn small" aria-pressed={tool === value} onclick={() => tool = value as RoomAnnotationTool}>{value === 'pointer' ? 'Point' : value === 'highlight' ? 'Highlight' : 'Draw'}</button>{/each}
    <button class="btn small" onclick={() => send({type: 'undo_annotation', source_id: source.id})}>Undo my mark</button>
  </div>
  {:else if !grant?.blocked && room.annotations_enabled !== false}<button class="btn small" disabled={grant?.requested} onclick={() => send({type: 'request_annotation', source_id: source.id})}>{grant?.requested ? 'Annotation requested' : 'Request annotation'}</button>
  {:else}<span>Annotations {grant?.blocked ? 'revoked by host' : 'disabled'}</span>{/if}
  {#if owner || host}<button class="btn small" onclick={() => send({type: 'clear_annotations', source_id: source.id})}>Clear marks</button>{/if}
</div>

{#if owner || host}<div class="requests">{#each room.annotation_grants?.filter(g => g.source_id === source.id && g.member_id !== room.member_id) ?? [] as permission (permission.member_id)}
  {@const member = room.members?.find(m => m.id === permission.member_id)}
  {#if owner && permission.requested && !permission.blocked}<button class="btn small" onclick={() => send({type: 'grant_annotation', source_id: source.id, member_id: permission.member_id, allowed: true})}>Allow {member?.name ?? 'participant'} to draw</button>{/if}
  {#if permission.allowed}<button class="btn small" onclick={() => host ? send({type: 'revoke_annotation', source_id: source.id, member_id: permission.member_id, blocked: true}) : send({type: 'grant_annotation', source_id: source.id, member_id: permission.member_id, allowed: false})}>Revoke {member?.name ?? 'participant'}’s drawing</button>{/if}
{/each}</div>{/if}
<style>
  .place-surface { position: absolute; inset: 0; pointer-events: none; border-radius: var(--radius-s); }
  .place-surface:focus-visible { outline: 2px solid var(--accent-solid); outline-offset: -2px; }
  .kb-cursor { stroke: var(--accent-solid); stroke-width: 2px; fill: var(--accent-soft); vector-effect: non-scaling-stroke; }
  .kb-rect { stroke-dasharray: 6 4; }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  .annotation-overlay { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; fill: none; stroke: var(--accent-solid); stroke-width: 3px; }
  .annotation-overlay.can-draw { pointer-events: auto; touch-action: none; cursor: crosshair; }
  polyline { vector-effect: non-scaling-stroke; stroke-linecap: round; stroke-linejoin: round; } .highlight { stroke: var(--warning); fill: var(--warning-soft); stroke-width: 2px; } circle { fill: var(--accent-soft); }
  .annotation-tools, .tools, .requests { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .media-stage { position: relative; aspect-ratio: 16/9; min-height: 120px; }
  .annotation-tools, .requests { background: var(--surface); padding: 8px; font-size: var(--fs-s); overflow-wrap: anywhere; }
  .requests:empty { display: none; }
  .requests :global(button) { height: auto; min-height: 22px; white-space: normal; text-align: start; max-width: 100%; }
</style>
