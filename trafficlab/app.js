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

const VEHICLE_COLORS = {
  car:          '#e05252',
  bus:          '#e8c84a',
  truck:        '#5270e0',
  motorcycle:   '#52c87a',
  autorickshaw: '#a052e0',
};

const SIGNAL_COLORS = {
  0: '#52c87a',   // NSGreen  → draw NS lights green, EW red
  1: '#e8c84a',   // AllRed   → yellow
  2: '#e05252',   // EWGreen  → draw EW lights green, NS red
};

const TRAFFIC_MODES = {
  normal:   { vpm: 120, cycleMultiplier: 1.0 },
  rush:     { vpm: 420, cycleMultiplier: 0.8 },
  festival: { vpm: 600, cycleMultiplier: 0.6 },
  rain:     { vpm:  60, cycleMultiplier: 1.2 },
};

const WARP_OPTIONS = [1, 10, 100, 1000];

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

  // ── Queue bars ───────────────────────────────────────────────────
  drawQueueBars(ctx, ox, cx, cy, half, PW, PH, state.queues);

  // ── Metrics overlay ──────────────────────────────────────────────
  drawMetrics(ctx, ox, PW, state);
}

function drawSignals(ctx, cx, cy, half, road, phase) {
  // phase: 0=NSGreen, 1=AllRed, 2=EWGreen
  const nsColor = phase === 0 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');
  const ewColor = phase === 2 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');

  const r = 6;
  // North signal  (above intersection, on road right edge)
  ctx.fillStyle = nsColor;
  ctx.beginPath(); ctx.arc(cx + half + 10, cy - half - 14, r, 0, Math.PI * 2); ctx.fill();
  // South signal
  ctx.beginPath(); ctx.arc(cx - half - 10, cy + half + 14, r, 0, Math.PI * 2); ctx.fill();
  // East signal
  ctx.fillStyle = ewColor;
  ctx.beginPath(); ctx.arc(cx + half + 14, cy + half + 10, r, 0, Math.PI * 2); ctx.fill();
  // West signal
  ctx.beginPath(); ctx.arc(cx - half - 14, cy - half - 10, r, 0, Math.PI * 2); ctx.fill();
}

function drawQueueBars(ctx, ox, cx, cy, half, PW, PH, queues) {
  const maxQ  = 24;
  const barW  = 18;
  const barMaxH = 100;

  // North queue — bar grows upward from intersection
  const nH = Math.min(queues.n / maxQ, 1) * barMaxH;
  ctx.fillStyle = 'rgba(224, 82, 82, 0.55)';
  ctx.fillRect(cx - barW / 2, cy - half - nH, barW, nH);

  // South queue — bar grows downward
  const sH = Math.min(queues.s / maxQ, 1) * barMaxH;
  ctx.fillRect(cx - barW / 2, cy + half, barW, sH);

  // East queue — bar grows rightward
  const eH = Math.min(queues.e / maxQ, 1) * barMaxH;
  ctx.fillRect(cx + half, cy - barW / 2, eH, barW);

  // West queue — bar grows leftward
  const wH = Math.min(queues.w / maxQ, 1) * barMaxH;
  ctx.fillRect(cx - half - wH, cy - barW / 2, wH, barW);

  // Vehicle count labels
  ctx.fillStyle = '#666';
  ctx.font = '10px monospace';
  ctx.textAlign = 'center';
  ctx.fillText(queues.n, cx, cy - half - nH - 4);
  ctx.fillText(queues.s, cx, cy + half + sH + 12);
  ctx.textAlign = 'left';
  ctx.fillText(queues.e, cx + half + eH + 4, cy + 4);
  ctx.fillText(queues.w, cx - half - wH - 22, cy + 4);
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
    lines.push(
      { label: '─── Cache ───', val: '', color: '#555' },
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
