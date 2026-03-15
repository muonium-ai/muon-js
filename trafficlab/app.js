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

// Vehicle type visual properties: [color, length-px, width-px]
// length = along travel direction, width = across road
// Order matches probability CDF: car 60%, motorcycle 20%, truck 10%, bus 5%, auto-rickshaw 5%
const VT = [
  ['#e05252', 28, 14],   // car
  ['#e8c84a', 40, 14],   // bus
  ['#5270e0', 36, 14],   // truck
  ['#52c87a', 20,  8],   // motorcycle
  ['#a052e0', 22, 12],   // auto-rickshaw
];

const VEH_GAP =  2;  // gap between consecutive vehicles
const MAX_VIS =  6;  // max individual vehicles drawn per queue arm

// ── Vehicle shape drawing ────────────────────────────────────────────────────

/**
 * Draw a top-down vehicle shape on the canvas.
 * Draws at origin facing UP (north), then uses ctx transforms for rotation.
 *
 * @param {CanvasRenderingContext2D} ctx
 * @param {number} cx   - centre X of vehicle
 * @param {number} cy   - centre Y of vehicle
 * @param {number} vLen - vehicle length (along travel axis)
 * @param {number} vWid - vehicle width (across travel axis)
 * @param {string} color - fill color
 * @param {number} typeIdx - vehicle type (0=car,1=bus,2=truck,3=motorcycle,4=auto)
 * @param {number} angle  - rotation in radians (0=north/up, π/2=east, π=south, 3π/2=west)
 */
function drawVehicleShape(ctx, cx, cy, vLen, vWid, color, typeIdx, angle) {
  ctx.save();
  ctx.translate(cx, cy);
  ctx.rotate(angle);
  // Draw shapes centred at origin, facing UP (−Y is front)
  const hw = vWid / 2;
  const hl = vLen / 2;
  ctx.fillStyle = color + 'cc';
  ctx.strokeStyle = color;
  ctx.lineWidth = 0.5;

  switch (typeIdx) {
    case 0: _drawCar(ctx, hw, hl); break;
    case 1: _drawBus(ctx, hw, hl); break;
    case 2: _drawTruck(ctx, hw, hl); break;
    case 3: _drawMotorcycle(ctx, hw, hl); break;
    case 4: _drawAutoRickshaw(ctx, hw, hl); break;
    default: _drawCar(ctx, hw, hl); break;
  }
  ctx.restore();
}

/** Car: rounded body + windshield notch + rear window */
function _drawCar(ctx, hw, hl) {
  // Body
  ctx.beginPath();
  ctx.roundRect(-hw, -hl, hw * 2, hl * 2, 3);
  ctx.fill();
  ctx.stroke();
  // Windshield (front = top = -Y)
  ctx.fillStyle = '#22334488';
  ctx.beginPath();
  ctx.roundRect(-hw + 2, -hl + 2, hw * 2 - 4, hl * 0.35, 2);
  ctx.fill();
  // Rear window
  ctx.beginPath();
  ctx.roundRect(-hw + 3, hl - hl * 0.28, hw * 2 - 6, hl * 0.22, 1);
  ctx.fill();
}

/** Bus: long body with window dots along sides */
function _drawBus(ctx, hw, hl) {
  // Body
  ctx.beginPath();
  ctx.roundRect(-hw, -hl, hw * 2, hl * 2, 2);
  ctx.fill();
  ctx.stroke();
  // Front windshield
  ctx.fillStyle = '#22334466';
  ctx.beginPath();
  ctx.roundRect(-hw + 2, -hl + 2, hw * 2 - 4, hl * 0.2, 2);
  ctx.fill();
  // Side windows (small squares along each side)
  ctx.fillStyle = '#4466aa55';
  const winCount = Math.floor(hl * 2 / 8);
  for (let i = 1; i < winCount; i++) {
    const wy = -hl + hl * 0.3 + i * ((hl * 1.5) / winCount);
    ctx.fillRect(-hw + 1, wy, 2, 3);
    ctx.fillRect(hw - 3, wy, 2, 3);
  }
}

/** Truck: cab (front) + cargo bed (rear), slightly narrower cab */
function _drawTruck(ctx, hw, hl) {
  // Cargo bed (rear, full width)
  ctx.beginPath();
  ctx.roundRect(-hw, -hl * 0.1, hw * 2, hl * 1.1, 1);
  ctx.fill();
  ctx.stroke();
  // Cab (front, slightly narrower)
  const cabW = hw * 0.85;
  ctx.beginPath();
  ctx.roundRect(-cabW, -hl, cabW * 2, hl * 0.45, 3);
  ctx.fill();
  ctx.stroke();
  // Cab windshield
  ctx.fillStyle = '#22334488';
  ctx.beginPath();
  ctx.roundRect(-cabW + 2, -hl + 2, cabW * 2 - 4, hl * 0.2, 2);
  ctx.fill();
}

/** Motorcycle: narrow oval body */
function _drawMotorcycle(ctx, hw, hl) {
  // Body oval
  ctx.beginPath();
  ctx.ellipse(0, 0, hw, hl, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  // Handlebar (front line)
  ctx.strokeStyle = ctx.fillStyle;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.moveTo(-hw - 1, -hl * 0.5);
  ctx.lineTo(hw + 1, -hl * 0.5);
  ctx.stroke();
}

/** Auto-rickshaw: teardrop-like body, wider at front */
function _drawAutoRickshaw(ctx, hw, hl) {
  ctx.beginPath();
  // Start from front-left, draw a rounded shape wider at front
  ctx.moveTo(-hw, -hl * 0.3);
  ctx.quadraticCurveTo(-hw, -hl, 0, -hl);           // front-left curve
  ctx.quadraticCurveTo(hw, -hl, hw, -hl * 0.3);     // front-right curve
  ctx.lineTo(hw * 0.7, hl);                           // right side tapers
  ctx.quadraticCurveTo(0, hl + 2, -hw * 0.7, hl);   // rounded rear
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
  // Windshield
  ctx.fillStyle = '#22334488';
  ctx.beginPath();
  ctx.roundRect(-hw + 2, -hl + 2, hw * 2 - 4, hl * 0.35, 2);
  ctx.fill();
}

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
  const lanes = state.laneCount || 3;
  // Road width scales with lane count: 16px per lane × 2 halves (one per direction)
  const road = Math.max(lanes * 16 * 2, 48);
  const half  = road / 2;

  // Road surfaces
  ctx.fillStyle = '#1e1e22';
  // N-S road
  ctx.fillRect(cx - half, 0, road, PH - 100);
  // E-W road
  ctx.fillRect(ox + PAD, cy - half, PW - PAD * 2, road);

  // Centre line (solid — divides opposing directions)
  ctx.strokeStyle = '#555520';
  ctx.lineWidth = 2;
  ctx.setLineDash([]);
  ctx.beginPath();
  ctx.moveTo(cx, 0);        ctx.lineTo(cx, cy - half);
  ctx.moveTo(cx, cy + half); ctx.lineTo(cx, PH - 100);
  ctx.moveTo(ox + PAD, cy); ctx.lineTo(cx - half, cy);
  ctx.moveTo(cx + half, cy); ctx.lineTo(ox + PW - PAD, cy);
  ctx.stroke();

  // Lane divider lines (dashed — within each direction half)
  if (lanes > 1) {
    ctx.strokeStyle = '#2e2e34';
    ctx.lineWidth = 1;
    ctx.setLineDash([6, 6]);
    const laneW = half / lanes;
    ctx.beginPath();
    for (let l = 1; l < lanes; l++) {
      // Left half lanes (N-bound traffic)
      const leftX = cx - half + l * laneW;
      ctx.moveTo(leftX, 0);        ctx.lineTo(leftX, cy - half);
      ctx.moveTo(leftX, cy + half); ctx.lineTo(leftX, PH - 100);
      // Right half lanes (S-bound traffic)
      const rightX = cx + l * laneW;
      ctx.moveTo(rightX, 0);        ctx.lineTo(rightX, cy - half);
      ctx.moveTo(rightX, cy + half); ctx.lineTo(rightX, PH - 100);
      // Top half lanes (W-bound traffic on E-W road)
      const topY = cy - half + l * laneW;
      ctx.moveTo(ox + PAD, topY); ctx.lineTo(cx - half, topY);
      ctx.moveTo(cx + half, topY); ctx.lineTo(ox + PW - PAD, topY);
      // Bottom half lanes (E-bound traffic on E-W road)
      const botY = cy + l * laneW;
      ctx.moveTo(ox + PAD, botY); ctx.lineTo(cx - half, botY);
      ctx.moveTo(cx + half, botY); ctx.lineTo(ox + PW - PAD, botY);
    }
    ctx.stroke();
    ctx.setLineDash([]);
  }

  // Intersection box
  ctx.fillStyle = '#252528';
  ctx.fillRect(cx - half, cy - half, road, road);

  // Zebra crossing hints
  ctx.strokeStyle = '#303036';
  ctx.lineWidth = 2;
  const zebraCount = Math.max(Math.round(road / 14), 3);
  for (let i = 0; i < zebraCount; i++) {
    const x = cx - half + 6 + i * ((road - 12) / Math.max(zebraCount - 1, 1));
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
  drawVehicleQueues(ctx, cx, cy, half, state.queues, state.laneCount || 3);

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
 * Vehicles are positioned in lanes across the road width.
 * Vehicle types are assigned deterministically by slot index using the probability
 * distribution (60% car, 20% motorcycle, 10% truck, 5% bus, 5% auto-rickshaw).
 */
function drawVehicleQueues(ctx, cx, cy, half, queues, laneCount) {
  _drawArmNS(ctx, cx, cy - half, queues.n, 0, -1, half, laneCount);  // N grows upward
  _drawArmNS(ctx, cx, cy + half, queues.s, 1, +1, half, laneCount);  // S grows downward
  _drawArmEW(ctx, cx + half, cy, queues.e, 2, +1, half, laneCount);  // E grows rightward
  _drawArmEW(ctx, cx - half, cy, queues.w, 3, -1, half, laneCount);  // W grows leftward
}

/**
 * Assign a vehicle to a lane index (0-based).
 * Distributes vehicles round-robin across available lanes.
 */
function slotLane(slotIdx, laneCount) {
  return slotIdx % laneCount;
}

/**
 * Draw a North or South queue arm.
 * Vehicles are drawn as width × length rects (narrow across road, tall along road).
 * Each vehicle is positioned in its assigned lane within the road half.
 *
 * For N queue: vehicles use the left half of the road (cx - half to cx).
 * For S queue: vehicles use the right half of the road (cx to cx + half).
 * This mimics vehicles on their side of a divided road.
 */
function _drawArmNS(ctx, cx, edgeY, count, dirIdx, sign, roadHalf, laneCount) {
  const visible = Math.min(count, MAX_VIS * laneCount);
  const laneWidth = roadHalf / laneCount;
  const laneBase = (sign < 0) ? cx - roadHalf : cx;
  const angle = (sign < 0) ? 0 : Math.PI; // N=0, S=π

  const laneDepth = new Array(laneCount).fill(0);

  for (let i = 0; i < visible; i++) {
    const vtIdx = slotVehicleTypeIdx(dirIdx, i);
    const [color, vLen, vWid] = VT[vtIdx];
    const lane = slotLane(i, laneCount);
    const depth = laneDepth[lane];
    laneDepth[lane]++;

    const offset = VEH_GAP + depth * (vLen + VEH_GAP);
    const vy = sign < 0 ? edgeY - offset - vLen / 2 : edgeY + offset + vLen / 2;
    const laneCx = laneBase + (lane + 0.5) * laneWidth;

    drawVehicleShape(ctx, laneCx, vy, vLen, vWid, color, vtIdx, angle);
  }
  // Queue count label beyond deepest vehicle
  const maxDepth = Math.max(...laneDepth, 0);
  const avgLen = 26; // approximate average vehicle length for label offset
  const labelOff = VEH_GAP + maxDepth * (avgLen + VEH_GAP) + 5;
  ctx.fillStyle = count > 0 ? '#888' : '#444';
  ctx.font = '10px monospace';
  ctx.textAlign = 'center';
  ctx.fillText(count, cx, sign < 0 ? edgeY - labelOff : edgeY + labelOff);
}

/**
 * Draw an East or West queue arm.
 * Vehicles are drawn as length × width rects (wide along road, narrow across road).
 * Each vehicle is positioned in its assigned lane within the road half.
 *
 * For E queue: vehicles use the bottom half of the road (cy to cy + roadHalf).
 * For W queue: vehicles use the top half of the road (cy - roadHalf to cy).
 */
function _drawArmEW(ctx, edgeX, cy, count, dirIdx, sign, roadHalf, laneCount) {
  const visible = Math.min(count, MAX_VIS * laneCount);
  const laneWidth = roadHalf / laneCount;
  const laneBase = (sign > 0) ? cy : cy - roadHalf;
  const angle = (sign > 0) ? Math.PI / 2 : Math.PI * 1.5; // E=π/2, W=3π/2

  const laneDepth = new Array(laneCount).fill(0);

  for (let i = 0; i < visible; i++) {
    const vtIdx = slotVehicleTypeIdx(dirIdx, i);
    const [color, vLen, vWid] = VT[vtIdx];
    const lane = slotLane(i, laneCount);
    const depth = laneDepth[lane];
    laneDepth[lane]++;

    const offset = VEH_GAP + depth * (vLen + VEH_GAP);
    const vx = sign > 0 ? edgeX + offset + vLen / 2 : edgeX - offset - vLen / 2;
    const laneCy = laneBase + (lane + 0.5) * laneWidth;

    drawVehicleShape(ctx, vx, laneCy, vLen, vWid, color, vtIdx, angle);
  }
  // Queue count label beyond deepest vehicle
  const maxDepth = Math.max(...laneDepth, 0);
  const avgLen = 26;
  const labelOff = VEH_GAP + maxDepth * (avgLen + VEH_GAP) + 5;
  ctx.fillStyle = count > 0 ? '#888' : '#444';
  ctx.font = '10px monospace';
  ctx.textAlign = sign > 0 ? 'left' : 'right';
  ctx.fillText(count, sign > 0 ? edgeX + labelOff : edgeX - labelOff, cy + 4);
}

function drawMetrics(ctx, ox, PW, s, yStart = 14) {
  const x = ox + 10;
  const lineH = 16;
  let y = yStart;

  const fmt = (n, dec = 0) => Number(n).toLocaleString('en-US', { maximumFractionDigits: dec });
  const simTime = s.simSeconds >= 3600
    ? `${(s.simSeconds / 3600).toFixed(1)} h`
    : s.simSeconds >= 60
    ? `${(s.simSeconds / 60).toFixed(1)} m`
    : `${s.simSeconds.toFixed(0)} s`;

  const waitFmt = s.avgWait >= 60
    ? `${(s.avgWait / 60).toFixed(1)} m`
    : `${s.avgWait.toFixed(1)} s`;

  const lines = [
    { label: 'Tick',       val: fmt(s.tick),                   color: '#b0b0c0' },
    { label: 'Sim time',   val: simTime,                        color: '#b0b0c0' },
    { label: 'Vehicles',   val: fmt(s.vehicles),                color: '#b0b0c0' },
    { label: 'Discharged', val: fmt(s.discharged),              color: '#b0b0c0' },
    { label: 'Avg wait',   val: waitFmt,                        color: '#e8c84a' },
    { label: 'Ticks/s',    val: fmt(s.tps, 0),                  color: '#7ec8ff' },
    { label: 'Queues',     val: `N${s.queues.n} S${s.queues.s} E${s.queues.e} W${s.queues.w}`, color: '#888' },
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
// ── Grid renderer ─────────────────────────────────────────────────────────────────────

/**
 * Draw one mini-intersection cell inside a grid pane.
 * All sizing is derived from cellW/cellH, so renders correctly at any grid scale.
 */
function drawMiniIntersection(ctx, ox, oy, cellW, cellH, phase, queues) {
  const cx = ox + cellW / 2;
  const cy = oy + cellH / 2;
  const road = Math.round(Math.min(cellW, cellH) * 0.22); // road width
  const half = road / 2;
  const barMaxH = Math.min(cellH * 0.28, 44);
  const barW = Math.max(Math.round(road * 0.28), 6);

  // Road surfaces
  ctx.fillStyle = '#1e1e22';
  ctx.fillRect(cx - half, oy, road, cellH);
  ctx.fillRect(ox, cy - half, cellW, road);

  // Intersection box
  ctx.fillStyle = '#252528';
  ctx.fillRect(cx - half, cy - half, road, road);

  // Signal dots with glow
  const nsColor = phase === 0 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');
  const ewColor = phase === 2 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');
  const sr = 4;
  function dot(x, y, color) {
    ctx.shadowColor = color; ctx.shadowBlur = 8;
    ctx.fillStyle = color;
    ctx.beginPath(); ctx.arc(x, y, sr, 0, Math.PI * 2); ctx.fill();
    ctx.shadowBlur = 0;
  }
  dot(cx + half + 7,  cy - half - 9,  nsColor);
  dot(cx - half - 7,  cy + half + 9,  nsColor);
  dot(cx + half + 9,  cy + half + 7,  ewColor);
  dot(cx - half - 9,  cy - half - 7,  ewColor);

  // Queue bars (simple proportional bars per arm)
  const maxQ = 24;
  ctx.fillStyle = 'rgba(224, 82, 82, 0.55)';
  const nH = Math.min(queues.n / maxQ, 1) * barMaxH;
  if (nH > 0) ctx.fillRect(cx - barW / 2, cy - half - nH, barW, nH);
  const sH = Math.min(queues.s / maxQ, 1) * barMaxH;
  if (sH > 0) ctx.fillRect(cx - barW / 2, cy + half, barW, sH);
  const eH = Math.min(queues.e / maxQ, 1) * barMaxH;
  if (eH > 0) ctx.fillRect(cx + half, cy - barW / 2, eH, barW);
  const wH = Math.min(queues.w / maxQ, 1) * barMaxH;
  if (wH > 0) ctx.fillRect(cx - half - wH, cy - barW / 2, wH, barW);
}

/**
 * Draw a full NxN grid pane (left or right half of the canvas).
 * Metrics are shown in a strip at the bottom of the pane.
 */
function drawGridPane(ctx, ox, gridSize, isNocache, lab, aggregateState) {
  const PW = HALF;
  const METRICS_H = 180;
  const gridH = H - METRICS_H;
  const cellW = Math.floor(PW / gridSize);
  const cellH = Math.floor(gridH / gridSize);

  // Background
  ctx.fillStyle = '#111114';
  ctx.fillRect(ox, 0, PW, H);

  // Grid cells
  for (let r = 0; r < gridSize; r++) {
    for (let c = 0; c < gridSize; c++) {
      const phase = isNocache
        ? lab.grid_nocache_signal_phase(r, c)
        : lab.grid_cached_signal_phase(r, c);
      const queues = {
        n: isNocache ? lab.grid_nocache_queue_north(r, c) : lab.grid_cached_queue_north(r, c),
        s: isNocache ? lab.grid_nocache_queue_south(r, c) : lab.grid_cached_queue_south(r, c),
        e: isNocache ? lab.grid_nocache_queue_east(r, c)  : lab.grid_cached_queue_east(r, c),
        w: isNocache ? lab.grid_nocache_queue_west(r, c)  : lab.grid_cached_queue_west(r, c),
      };
      drawMiniIntersection(ctx, ox + c * cellW, r * cellH, cellW, cellH, phase, queues);
    }
  }

  // Cell dividers
  ctx.strokeStyle = '#22222a';
  ctx.lineWidth = 1;
  ctx.setLineDash([]);
  for (let r = 1; r < gridSize; r++) {
    ctx.beginPath();
    ctx.moveTo(ox, r * cellH); ctx.lineTo(ox + PW, r * cellH);
    ctx.stroke();
  }
  for (let c = 1; c < gridSize; c++) {
    ctx.beginPath();
    ctx.moveTo(ox + c * cellW, 0); ctx.lineTo(ox + c * cellW, gridH);
    ctx.stroke();
  }

  // Grid-area bottom border
  ctx.strokeStyle = '#2a2a2e';
  ctx.beginPath();
  ctx.moveTo(ox, gridH); ctx.lineTo(ox + PW, gridH);
  ctx.stroke();

  // Aggregate metrics below grid
  drawMetrics(ctx, ox, PW, aggregateState, gridH + 10);
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
    const isIndia = country === 'india';
    lab.set_free_left_turn(isIndia);
    lab.set_motorcycle_splitting(isIndia);
  }

  document.getElementById('vpm').addEventListener('input', e => {
    const v = parseInt(e.target.value);
    document.getElementById('vpm-val').textContent = v;
    lab.set_vehicles_per_min(v);
    lab.reset();
  });

  document.getElementById('cycle').addEventListener('input', e => {
    const v = parseInt(e.target.value);
    document.getElementById('cycle-val').textContent = v;
    lab.set_signal_cycle_secs(v);
    lab.reset();
  });

  document.getElementById('lanes').addEventListener('input', e => {
    const v = parseInt(e.target.value);
    document.getElementById('lanes-val').textContent = v;
    lab.set_lane_count(v);
    lab.reset();
  });

  document.getElementById('grid').addEventListener('change', e => {
    lab.set_grid_size(parseInt(e.target.value));
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

  // Share button
  document.getElementById('btn-share').addEventListener('click', () => {
    const url = window.location.href;
    navigator.clipboard.writeText(url).then(() => {
      const btn = document.getElementById('btn-share');
      btn.textContent = '\u2713 Copied!';
      setTimeout(() => { btn.innerHTML = '&#128279; Share'; }, 2000);
    }).catch(() => {
      window.prompt('Copy this URL:', window.location.href);
    });
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
      discharged: lab.nocache_vehicles_discharged(),
      avgWait:    lab.nocache_avg_wait_sec(),
      tps:        lab.tps_nocache(),
      laneCount:  lab.lane_count(),
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
      discharged:    lab.cached_vehicles_discharged(),
      avgWait:       lab.cached_avg_wait_sec(),
      tps:           lab.tps_cached(),
      laneCount:     lab.lane_count(),
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
    const gs = lab.grid_size();
    if (gs === 1) {
      drawPane(ctx, 0,    noCache);
      drawPane(ctx, HALF, cached);
    } else {
      drawGridPane(ctx, 0,    gs, true,  lab, noCache);
      drawGridPane(ctx, HALF, gs, false, lab, cached);
    }

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
