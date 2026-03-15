/**
 * TrafficLab — JavaScript simulation driver.
 *
 * Loads the WASM module (built by wasm-pack), drives the animation loop,
 * renders both simulation panes to Canvas, and wires up all UI controls.
 *
 * Rendering architecture:
 *   - Single <canvas> split vertically: left = no-cache, right = cached
 *   - Each pane: intersection grid, signal indicators, vehicle queues,
 *     live metrics overlay
 *   - T-000108 will inject real vehicle positions into step_frame();
 *     for now the renderer visualises queue bars and signal state.
 */

import init, { TrafficLab } from './pkg/trafficlab.js';

// ── Constants ────────────────────────────────────────────────────────────────

const TRAFFIC_MODES = {
  normal:   { vpm: 120, cycleMultiplier: 1.0 },
  rush:     { vpm: 420, cycleMultiplier: 0.8 },
  festival: { vpm: 600, cycleMultiplier: 0.6 },
  rain:     { vpm:  60, cycleMultiplier: 1.2 },
};

const WARP_OPTIONS = [1, 10, 100, 1000];

// Vehicle type visual properties: [color, cross-road-width-px]
// Order matches probability CDF: car 60%, motorcycle 20%, truck 10%, bus 5%, auto-rickshaw 5%
const VT = [
  ['#e05252', 42],   // car
  ['#e8c84a', 50],   // bus
  ['#5270e0', 46],   // truck
  ['#52c87a', 24],   // motorcycle
  ['#a052e0', 34],   // auto-rickshaw
];

const VEH_H   = 11;  // vehicle rect height (along road axis) for N/S queues
const VEH_GAP =  3;  // gap between consecutive vehicles
const MAX_VIS =  6;  // max individual vehicles drawn per queue arm

/** Deterministic vehicle type for a given queue slot (direction × slot index). */
function slotVehicleTypeIdx(dirIdx, slotIdx) {
  const h = ((dirIdx * 31 + slotIdx * 97) >>> 0) % 100;
  if (h < 60) return 0; // car
  if (h < 80) return 3; // motorcycle
  if (h < 90) return 2; // truck
  if (h < 95) return 1; // bus
  return 4;              // auto-rickshaw
}

// ── Renderer helpers ─────────────────────────────────────────────────────────

const W = 1200;
const H = 580;
const HALF = W / 2;
const PAD  = 40;

/**
 * Draw a single intersection pane.
 * @param {CanvasRenderingContext2D} ctx
 * @param {number} ox  - x offset (0 for left pane, HALF for right)
 * @param {Object} state - keys: signalPhase, queues {n,s,e,w}, simSeconds, tick,
 *                          vehicles, tps, cacheHits, cacheMisses, cacheRatio,
 *                          cacheKeys, cacheLatency, isCached
 */
function drawPane(ctx, ox, state) {
  const PW = HALF;    // pane width
  const PH = H;

  // ── Background ────────────────────────────────────────────────────
  ctx.fillStyle = '#111114';
  ctx.fillRect(ox, 0, PW, PH);

  // ── Intersection grid ────────────────────────────────────────────
  const cx  = ox + PW / 2;
  const cy  = PH / 2 - 30;
  const road = 60;        // road width px
  const half  = road / 2;

  // Road surfaces
  ctx.fillStyle = '#1e1e22';
  // N-S road
  ctx.fillRect(cx - half, 0, road, PH - 100);
  // E-W road
  ctx.fillRect(ox + PAD, cy - half, PW - PAD * 2, road);

  // Lane centre lines (dashed)
  ctx.strokeStyle = '#2e2e34';
  ctx.lineWidth = 1;
  ctx.setLineDash([8, 8]);
  ctx.beginPath();
  ctx.moveTo(cx, 0);       ctx.lineTo(cx, cy - half);
  ctx.moveTo(cx, cy + half); ctx.lineTo(cx, PH - 100);
  ctx.moveTo(ox + PAD, cy); ctx.lineTo(cx - half, cy);
  ctx.moveTo(cx + half, cy); ctx.lineTo(ox + PW - PAD, cy);
  ctx.stroke();
  ctx.setLineDash([]);

  // Intersection box
  ctx.fillStyle = '#252528';
  ctx.fillRect(cx - half, cy - half, road, road);

  // Zebra crossing hints
  ctx.strokeStyle = '#303036';
  ctx.lineWidth = 2;
  for (let i = 0; i < 4; i++) {
    const x = cx - half + 6 + i * 12;
    ctx.beginPath();
    ctx.moveTo(x, cy - half - 8);
    ctx.lineTo(x, cy - half);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(x, cy + half);
    ctx.lineTo(x, cy + half + 8);
    ctx.stroke();
  }

  // ── Traffic signals ──────────────────────────────────────────────
  drawSignals(ctx, cx, cy, half, road, state.signalPhase);

  // ── Vehicle queues ───────────────────────────────────────────────
  drawVehicleQueues(ctx, cx, cy, half, state.queues);

  // ── Metrics overlay ──────────────────────────────────────────────
  drawMetrics(ctx, ox, PW, state);
}

function drawSignals(ctx, cx, cy, half, road, phase) {
  // phase: 0=NSGreen, 1=AllRed, 2=EWGreen
  const nsColor = phase === 0 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');
  const ewColor = phase === 2 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');

  function drawLight(x, y, color) {
    const r = 7;
    // Dark housing box
    ctx.fillStyle = '#0d0d10';
    ctx.beginPath();
    ctx.roundRect(x - r - 4, y - r - 4, (r + 4) * 2, (r + 4) * 2, 3);
    ctx.fill();
    ctx.strokeStyle = '#333';
    ctx.lineWidth = 1;
    ctx.strokeRect(x - r - 4, y - r - 4, (r + 4) * 2, (r + 4) * 2);
    // Glowing light
    ctx.shadowColor = color;
    ctx.shadowBlur = 12;
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fill();
    ctx.shadowBlur = 0;
  }

  // North signal (above intersection, right of road centreline)
  drawLight(cx + half + 12, cy - half - 16, nsColor);
  // South signal (below intersection, left of road centreline)
  drawLight(cx - half - 12, cy + half + 16, nsColor);
  // East signal (right of intersection, bottom of road centreline)
  drawLight(cx + half + 16, cy + half + 12, ewColor);
  // West signal (left of intersection, top of road centreline)
  drawLight(cx - half - 16, cy - half - 12, ewColor);
}

/**
 * Draw individual vehicle rectangles stacked in each queue arm.
 * Vehicle types are assigned deterministically by slot index using the probability
 * distribution (60% car, 20% motorcycle, 10% truck, 5% bus, 5% auto-rickshaw).
 */
function drawVehicleQueues(ctx, cx, cy, half, queues) {
  _drawArmNS(ctx, cx, cy - half, queues.n, 0, -1);  // N grows upward
  _drawArmNS(ctx, cx, cy + half, queues.s, 1, +1);  // S grows downward
  _drawArmEW(ctx, cx + half, cy, queues.e, 2, +1);  // E grows rightward
  _drawArmEW(ctx, cx - half, cy, queues.w, 3, -1);  // W grows leftward
}

/** Draw a North or South queue arm (vehicles are horizontal rects across road). */
function _drawArmNS(ctx, cx, edgeY, count, dirIdx, sign) {
  const visible = Math.min(count, MAX_VIS);
  for (let i = 0; i < visible; i++) {
    const [color, nsW] = VT[slotVehicleTypeIdx(dirIdx, i)];
    const offset = VEH_GAP + i * (VEH_H + VEH_GAP);
    // sign<0 → draw above edgeY (north); sign>0 → draw below (south)
    const y = sign < 0 ? edgeY - offset - VEH_H : edgeY + offset;
    ctx.fillStyle = color + 'cc';
    ctx.beginPath();
    ctx.roundRect(cx - nsW / 2, y, nsW, VEH_H, 2);
    ctx.fill();
  }
  // Queue count label beyond last vehicle
  const labelOff = VEH_GAP + visible * (VEH_H + VEH_GAP) + 5;
  ctx.fillStyle = count > 0 ? '#888' : '#444';
  ctx.font = '10px monospace';
  ctx.textAlign = 'center';
  ctx.fillText(count, cx, sign < 0 ? edgeY - labelOff : edgeY + labelOff);
}

/** Draw an East or West queue arm (vehicles are vertical rects across road). */
function _drawArmEW(ctx, edgeX, cy, count, dirIdx, sign) {
  const visible = Math.min(count, MAX_VIS);
  for (let i = 0; i < visible; i++) {
    const [color, nsW] = VT[slotVehicleTypeIdx(dirIdx, i)];
    const offset = VEH_GAP + i * (VEH_H + VEH_GAP);
    // sign>0 → draw rightward (east); sign<0 → draw leftward (west)
    const x = sign > 0 ? edgeX + offset : edgeX - offset - VEH_H;
    ctx.fillStyle = color + 'cc';
    ctx.beginPath();
    ctx.roundRect(x, cy - nsW / 2, VEH_H, nsW, 2);
    ctx.fill();
  }
  // Queue count label beyond last vehicle
  const labelOff = VEH_GAP + visible * (VEH_H + VEH_GAP) + 5;
  ctx.fillStyle = count > 0 ? '#888' : '#444';
  ctx.font = '10px monospace';
  ctx.textAlign = sign > 0 ? 'left' : 'right';
  ctx.fillText(count, sign > 0 ? edgeX + labelOff : edgeX - labelOff, cy + 4);
}

function drawMetrics(ctx, ox, PW, s) {
  const x = ox + 10;
  const lineH = 16;
  let y = 14;

  const fmt = (n, dec = 0) => Number(n).toLocaleString('en-US', { maximumFractionDigits: dec });
  const simTime = s.simSeconds >= 3600
    ? `${(s.simSeconds / 3600).toFixed(1)} h`
    : s.simSeconds >= 60
    ? `${(s.simSeconds / 60).toFixed(1)} m`
    : `${s.simSeconds.toFixed(0)} s`;

  const lines = [
    { label: 'Tick',       val: fmt(s.tick),                   color: '#b0b0c0' },
    { label: 'Sim time',   val: simTime,                        color: '#b0b0c0' },
    { label: 'Vehicles',   val: fmt(s.vehicles),                color: '#b0b0c0' },
    { label: 'Ticks/s',    val: fmt(s.tps, 0),                  color: '#7ec8ff' },
  ];

  if (s.isCached) {
    const speedup = (s.nocacheTps > 0 && s.tps > 0)
      ? (s.tps / s.nocacheTps).toFixed(1) + '×'
      : '—';
    lines.push(
      { label: '─── Cache ───', val: '', color: '#555' },
      { label: 'Speedup',     val: speedup,                       color: '#ffe840' },
      { label: 'Hits',        val: fmt(s.cacheHits),             color: '#6bffb8' },
      { label: 'Misses',      val: fmt(s.cacheMisses),           color: '#ff9f6b' },
      { label: 'Hit ratio',   val: (s.cacheRatio * 100).toFixed(1) + '%', color: '#6bffb8' },
      { label: 'Avg latency', val: s.cacheLatency.toFixed(2) + ' µs',     color: '#6bffb8' },
      { label: 'Keys stored', val: fmt(s.cacheKeys),             color: '#b0d0ff' },
    );
  }

  ctx.font = '11px monospace';
  for (const { label, val, color } of lines) {
    ctx.fillStyle = '#444';
    ctx.fillText(label, x, y);
    if (val) {
      ctx.fillStyle = color;
      ctx.fillText(val, x + 90, y);
    }
    y += lineH;
  }
}

// ── Main ─────────────────────────────────────────────────────────────────────

async function main() {
  const statusBar = document.getElementById('status-bar');

  // Load WASM
  await init();
  statusBar.textContent = 'WASM loaded — simulation running';

  const canvas = document.getElementById('sim-canvas');
  const ctx    = canvas.getContext('2d');

  let lab = new TrafficLab();
  lab.set_speed_multiplier(1000);

  // ── Wire controls ──────────────────────────────────────────────────

  function applyMode(mode) {
    const m = TRAFFIC_MODES[mode] || TRAFFIC_MODES.normal;
    lab.set_vehicles_per_min(m.vpm);
    document.getElementById('vpm').value = m.vpm;
    document.getElementById('vpm-val').textContent = m.vpm;
    const cycle = Math.round(60 * m.cycleMultiplier);
    lab.set_signal_cycle_secs(cycle);
    document.getElementById('cycle').value = cycle;
    document.getElementById('cycle-val').textContent = cycle;
  }

  function applyCountry(country) {
    lab.set_free_left_turn(country === 'india');
  }

  document.getElementById('vpm').addEventListener('input', e => {
    const v = parseInt(e.target.value);
    document.getElementById('vpm-val').textContent = v;
    lab.set_vehicles_per_min(v);
  });

  document.getElementById('cycle').addEventListener('input', e => {
    const v = parseInt(e.target.value);
    document.getElementById('cycle-val').textContent = v;
    lab.set_signal_cycle_secs(v);
  });

  document.getElementById('lanes').addEventListener('input', e => {
    const v = parseInt(e.target.value);
    document.getElementById('lanes-val').textContent = v;
    lab.set_lane_count(v);
  });

  document.getElementById('warp').addEventListener('change', e => {
    lab.set_speed_multiplier(parseInt(e.target.value));
  });

  document.getElementById('mode').addEventListener('change', e => {
    applyMode(e.target.value);
    lab.reset();
  });

  document.getElementById('country').addEventListener('change', e => {
    applyCountry(e.target.value);
    lab.reset();
  });

  document.getElementById('btn-reset').addEventListener('click', () => {
    lab.reset();
    applyMode(document.getElementById('mode').value);
    applyCountry(document.getElementById('country').value);
    lab.set_speed_multiplier(parseInt(document.getElementById('warp').value));
  });

  // Apply defaults
  applyMode('normal');
  lab.set_speed_multiplier(1000);

  // ── Render loop ────────────────────────────────────────────────────

  let lastTime = performance.now();

  function frame(now) {
    const dt = Math.min(now - lastTime, 100); // clamp to 100ms to avoid spiral on tab switch
    lastTime = now;

    lab.step_frame(dt);

    // Collect state
    const noCache = {
      signalPhase: lab.nocache_signal_phase(),
      queues: {
        n: lab.nocache_queue_north(),
        s: lab.nocache_queue_south(),
        e: lab.nocache_queue_east(),
        w: lab.nocache_queue_west(),
      },
      tick:       lab.nocache_tick(),
      simSeconds: lab.nocache_sim_seconds(),
      vehicles:   lab.nocache_vehicles_processed(),
      tps:        lab.tps_nocache(),
      isCached:   false,
    };

    const cached = {
      signalPhase: lab.cached_signal_phase(),
      queues: {
        n: lab.cached_queue_north(),
        s: lab.cached_queue_south(),
        e: lab.cached_queue_east(),
        w: lab.cached_queue_west(),
      },
      tick:          lab.cached_tick(),
      simSeconds:    lab.cached_sim_seconds(),
      vehicles:      lab.cached_vehicles_processed(),
      tps:           lab.tps_cached(),
      nocacheTps:    lab.tps_nocache(),
      cacheHits:     lab.cache_hits(),
      cacheMisses:   lab.cache_misses(),
      cacheRatio:    lab.cache_hit_ratio(),
      cacheKeys:     lab.cache_keys(),
      cacheLatency:  lab.cache_avg_latency_us(),
      isCached:      true,
    };

    // Clear and draw
    ctx.clearRect(0, 0, W, H);
    drawPane(ctx, 0,    noCache);
    drawPane(ctx, HALF, cached);

    // Centre divider
    ctx.strokeStyle = '#2a2a2e';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(HALF, 0);
    ctx.lineTo(HALF, H);
    ctx.stroke();

    requestAnimationFrame(frame);
  }

  requestAnimationFrame(frame);
}

main().catch(err => {
  document.getElementById('status-bar').textContent = 'Error: ' + err.message;
  console.error(err);
});
