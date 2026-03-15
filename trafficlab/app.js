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

/**
 * Draw a vehicle number badge + turn-direction arrow overlay.
 * Must be called immediately after drawVehicleShape with the same cx/cy/angle.
 * Only intended for the 1×1 pane view (not grid cells).
 *
 * Turn direction is assigned by lane discipline:
 *   - leftmost lane (0)              → left turn  ←
 *   - rightmost lane (laneCount-1)   → right turn →
 *   - middle lanes / single lane     → straight   ↑
 *
 * Both badges are drawn in the vehicle's rotated local space, so they
 * automatically face the correct direction for all four approach arms.
 *
 * @param {CanvasRenderingContext2D} ctx
 * @param {number} cx        - vehicle centre X
 * @param {number} cy        - vehicle centre Y
 * @param {number} vLen      - vehicle length (along travel axis)
 * @param {number} num       - 1-based slot index within the queue arm
 * @param {number} lane      - 0-based lane index assigned to this vehicle
 * @param {number} laneCount - total lanes
 * @param {number} angle     - same rotation angle passed to drawVehicleShape
 */
function drawVehicleOverlay(ctx, cx, cy, vLen, num, lane, laneCount, angle) {
  let arrowChar;
  if (laneCount <= 1) {
    arrowChar = '\u2191';          // ↑ straight
  } else if (lane === 0) {
    arrowChar = '\u2190';          // ← left
  } else if (lane >= laneCount - 1) {
    arrowChar = '\u2192';          // → right
  } else {
    arrowChar = '\u2191';          // ↑ straight (middle lanes)
  }

  ctx.save();
  ctx.translate(cx, cy);
  ctx.rotate(angle);
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';

  // ── Number badge (near vehicle front = top in local space) ──────────
  const numY = -vLen * 0.22;
  ctx.fillStyle = 'rgba(0,0,0,0.62)';
  ctx.fillRect(-7, numY - 5, 14, 10);
  ctx.font = 'bold 7px monospace';
  ctx.fillStyle = '#ffffff';
  ctx.fillText(num, 0, numY);

  // ── Arrow badge (near vehicle rear = bottom in local space) ─────────
  const arrY = vLen * 0.22;
  ctx.fillStyle = 'rgba(0,0,0,0.5)';
  ctx.fillRect(-6, arrY - 5, 12, 10);
  ctx.font = '8px sans-serif';
  ctx.fillStyle = '#ffe840';
  ctx.fillText(arrowChar, 0, arrY);

  ctx.restore();
}

// ── Vehicle animation system ─────────────────────────────────────────────────

/**
 * Visual vehicle for smooth animation.
 * @typedef {{
 *   typeIdx: number,  lane: number,
 *   pos: number,      targetPos: number,
 *   state: 'approach'|'queued'|'discharge',
 *   age: number
 * }} VisVehicle
 */

// Road geometry (derived from drawPane):
// N arm: ~200px from canvas top to intersection edge
// S arm: ~220px from intersection edge to road end
// E/W arms: ~230px from intersection edge to pane edge
// Vehicles spawn at the road edge and drive the full length to the signal.
// After discharge, they cross the intersection and exit the opposite road.
const SPAWN_DIST_NS  = 200;   // px from intersection edge — approx full N/S road length
const SPAWN_DIST_EW  = 230;   // px for E/W roads (pane edge to intersection)
const EXIT_DIST      = 230;   // px past intersection before removal (opposite road length)
const APPROACH_SPEED = 180;   // px/sec — smooth driving speed toward intersection
const DISCHARGE_SPEED = 220;  // px/sec — slightly faster exit through intersection
const QUEUE_LERP = 12;        // lerp factor for settling into queue position

/**
 * Manages visual vehicle lists for one pane (4 directions).
 * Reconciles with WASM queue counts each frame and produces smooth motion.
 */
class PaneAnimator {
  constructor() {
    this.dirs = { n: [], s: [], e: [], w: [] };
    this._nextId = 0;
  }

  /** Clear all vehicles (on sim reset). */
  reset() {
    for (const k of ['n','s','e','w']) this.dirs[k] = [];
  }

  /**
   * Update visual vehicles to match the simulation queue counts.
   * @param {{n:number,s:number,e:number,w:number}} queues - current WASM counts
   * @param {number} dt - wall-clock seconds since last frame
   * @param {number} laneCount
   * @param {number} signalPhase - 0=NSGreen, 1=AllRed, 2=EWGreen
   */
  update(queues, dt, laneCount) {
    for (const dir of ['n','s','e','w']) {
      this._updateDir(dir, queues[dir], dt, laneCount);
    }
  }

  _updateDir(dir, targetCount, dt, laneCount) {
    const list = this.dirs[dir];
    const dirIdx = { n:0, s:1, e:2, w:3 }[dir];
    const isNS = (dir === 'n' || dir === 's');
    const spawnDist = isNS ? SPAWN_DIST_NS : SPAWN_DIST_EW;

    // Separate queued/approaching from discharging
    const active = list.filter(v => v.state !== 'discharge');
    const discharging = list.filter(v => v.state === 'discharge');

    // ── Reconcile count ──────────────────────────────────────────
    while (active.length < targetCount) {
      // Spawn at the far edge of the road
      const idx = active.length;
      const vtIdx = slotVehicleTypeIdx(dirIdx, idx);
      const lane = slotLane(idx, laneCount);
      active.push({
        typeIdx: vtIdx, lane,
        pos: spawnDist + Math.random() * 20,
        targetPos: 0,
        state: 'approach',
        age: 0,
      });
    }
    while (active.length > targetCount) {
      // Discharge front vehicle (smallest pos = closest to intersection)
      active.sort((a, b) => a.pos - b.pos);
      const v = active.shift();
      if (v) {
        v.state = 'discharge';
        v.pos = 0;
        discharging.push(v);
      }
    }

    // ── Compute target positions (queue stacking) ────────────────
    const laneSlots = new Array(laneCount).fill(0);
    for (let i = 0; i < active.length; i++) {
      active[i].lane = slotLane(i, laneCount);
    }
    for (let i = 0; i < active.length; i++) {
      const [, vLen] = VT[active[i].typeIdx];
      const lane = active[i].lane;
      const depth = laneSlots[lane];
      laneSlots[lane]++;
      active[i].targetPos = VEH_GAP + depth * (vLen + VEH_GAP);
    }

    // ── Animate positions ────────────────────────────────────────
    for (const v of active) {
      v.age += dt;
      const diff = v.targetPos - v.pos;
      if (Math.abs(diff) < 0.5) {
        v.pos = v.targetPos;
        v.state = 'queued';
      } else if (v.state === 'approach') {
        // Drive toward intersection at steady speed, but don't overshoot target
        const step = APPROACH_SPEED * dt;
        if (v.pos - step <= v.targetPos) {
          v.pos = v.targetPos;
          v.state = 'queued';
        } else {
          v.pos -= step;
        }
      } else {
        // Queued vehicle settling (e.g. queue shifted forward)
        v.pos += diff * Math.min(QUEUE_LERP * dt, 1);
      }
    }

    // Animate discharging vehicles — drive through intersection and out opposite road
    for (const v of discharging) {
      v.pos -= DISCHARGE_SPEED * dt; // negative = past intersection, into opposite road
      v.age += dt;
    }

    // Remove discharged vehicles that have exited the opposite road
    const kept = discharging.filter(v => v.pos > -EXIT_DIST);

    this.dirs[dir] = [...active, ...kept];
  }
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

  // ── Vehicle queues (animated) ───────────────────────────────────
  if (state._animator) {
    drawAnimatedVehicles(ctx, cx, cy, half, state._animator, state.laneCount || 3, state.queues);
  }

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
 * Assign a vehicle to a lane index (0-based).
 * Distributes vehicles round-robin across available lanes.
 */
function slotLane(slotIdx, laneCount) {
  return slotIdx % laneCount;
}

/**
 * Draw animated vehicles from a PaneAnimator for one pane.
 * Converts each VisVehicle's logical `pos` into canvas coordinates and draws
 * the correct shape at the correct rotation.
 *
 * @param {CanvasRenderingContext2D} ctx
 * @param {number} cx - road centre X
 * @param {number} cy - intersection centre Y
 * @param {number} roadHalf - half of total road width
 * @param {PaneAnimator} animator
 * @param {number} laneCount
 * @param {{n:number,s:number,e:number,w:number}} queues - for count labels
 */
function drawAnimatedVehicles(ctx, cx, cy, roadHalf, animator, laneCount, queues) {
  const laneW = roadHalf / laneCount;

  // ── N arm (left half, grows upward) ────────────────────────────
  {
    const edgeY = cy - roadHalf;
    const laneBase = cx - roadHalf;
    for (let i = 0; i < animator.dirs.n.length; i++) {
      const v = animator.dirs.n[i];
      const [color, vLen, vWid] = VT[v.typeIdx];
      const laneCx = laneBase + (v.lane + 0.5) * laneW;
      const vy = edgeY - v.pos - vLen / 2;
      drawVehicleShape(ctx, laneCx, vy, vLen, vWid, color, v.typeIdx, 0);
      drawVehicleOverlay(ctx, laneCx, vy, vLen, i + 1, v.lane, laneCount, 0);
    }
    _drawQueueLabel(ctx, cx, edgeY, queues.n, -1, animator.dirs.n, laneCount);
  }

  // ── S arm (right half, grows downward) ─────────────────────────
  {
    const edgeY = cy + roadHalf;
    const laneBase = cx;
    for (let i = 0; i < animator.dirs.s.length; i++) {
      const v = animator.dirs.s[i];
      const [color, vLen, vWid] = VT[v.typeIdx];
      const laneCx = laneBase + (v.lane + 0.5) * laneW;
      const vy = edgeY + v.pos + vLen / 2;
      drawVehicleShape(ctx, laneCx, vy, vLen, vWid, color, v.typeIdx, Math.PI);
      drawVehicleOverlay(ctx, laneCx, vy, vLen, i + 1, v.lane, laneCount, Math.PI);
    }
    _drawQueueLabel(ctx, cx, edgeY, queues.s, +1, animator.dirs.s, laneCount);
  }

  // ── E arm (bottom half, grows rightward) ───────────────────────
  {
    const edgeX = cx + roadHalf;
    const laneBase = cy;
    for (let i = 0; i < animator.dirs.e.length; i++) {
      const v = animator.dirs.e[i];
      const [color, vLen, vWid] = VT[v.typeIdx];
      const laneCy = laneBase + (v.lane + 0.5) * laneW;
      const vx = edgeX + v.pos + vLen / 2;
      drawVehicleShape(ctx, vx, laneCy, vLen, vWid, color, v.typeIdx, Math.PI / 2);
      drawVehicleOverlay(ctx, vx, laneCy, vLen, i + 1, v.lane, laneCount, Math.PI / 2);
    }
    _drawQLabelEW(ctx, cy, edgeX, queues.e, +1, animator.dirs.e, laneCount);
  }

  // ── W arm (top half, grows leftward) ───────────────────────────
  {
    const edgeX = cx - roadHalf;
    const laneBase = cy - roadHalf;
    for (let i = 0; i < animator.dirs.w.length; i++) {
      const v = animator.dirs.w[i];
      const [color, vLen, vWid] = VT[v.typeIdx];
      const laneCy = laneBase + (v.lane + 0.5) * laneW;
      const vx = edgeX - v.pos - vLen / 2;
      drawVehicleShape(ctx, vx, laneCy, vLen, vWid, color, v.typeIdx, Math.PI * 1.5);
      drawVehicleOverlay(ctx, vx, laneCy, vLen, i + 1, v.lane, laneCount, Math.PI * 1.5);
    }
    _drawQLabelEW(ctx, cy, edgeX, queues.w, -1, animator.dirs.w, laneCount);
  }
}

/** Queue count label for N/S arms. */
function _drawQueueLabel(ctx, cx, edgeY, count, sign, vehicles, laneCount) {
  // Find max depth among queued vehicles
  const laneDepths = new Array(laneCount).fill(0);
  for (const v of vehicles) {
    if (v.state === 'queued') laneDepths[v.lane]++;
  }
  const maxDepth = Math.max(...laneDepths, 0);
  const avgLen = 26;
  const labelOff = VEH_GAP + maxDepth * (avgLen + VEH_GAP) + 5;
  ctx.fillStyle = count > 0 ? '#888' : '#444';
  ctx.font = '10px monospace';
  ctx.textAlign = 'center';
  ctx.fillText(count, cx, sign < 0 ? edgeY - labelOff : edgeY + labelOff);
}

/** Queue count label for E/W arms. */
function _drawQLabelEW(ctx, cy, edgeX, count, sign, vehicles, laneCount) {
  const laneDepths = new Array(laneCount).fill(0);
  for (const v of vehicles) {
    if (v.state === 'queued') laneDepths[v.lane]++;
  }
  const maxDepth = Math.max(...laneDepths, 0);
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

  // ── Animation state ─────────────────────────────────────────────────

  const ncAnimator = new PaneAnimator();
  const caAnimator = new PaneAnimator();

  /** Reset simulation and animators. Use this instead of lab.reset() directly. */
  function resetAll() {
    lab.reset();
    ncAnimator.reset();
    caAnimator.reset();
  }

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
    resetAll();
  });

  document.getElementById('cycle').addEventListener('input', e => {
    const v = parseInt(e.target.value);
    document.getElementById('cycle-val').textContent = v;
    lab.set_signal_cycle_secs(v);
    resetAll();
  });

  document.getElementById('lanes').addEventListener('input', e => {
    const v = parseInt(e.target.value);
    document.getElementById('lanes-val').textContent = v;
    lab.set_lane_count(v);
    resetAll();
  });

  document.getElementById('grid').addEventListener('change', e => {
    lab.set_grid_size(parseInt(e.target.value));
    ncAnimator.reset();
    caAnimator.reset();
  });

  document.getElementById('warp').addEventListener('change', e => {
    lab.set_speed_multiplier(parseInt(e.target.value));
  });

  document.getElementById('mode').addEventListener('change', e => {
    applyMode(e.target.value);
    resetAll();
  });

  document.getElementById('country').addEventListener('change', e => {
    applyCountry(e.target.value);
    resetAll();
  });

  document.getElementById('btn-reset').addEventListener('click', () => {
    resetAll();
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

    const dtSec = dt / 1000;
    const lc = lab.lane_count();

    // Collect state
    const ncQueues = {
      n: lab.nocache_queue_north(),
      s: lab.nocache_queue_south(),
      e: lab.nocache_queue_east(),
      w: lab.nocache_queue_west(),
    };
    const caQueues = {
      n: lab.cached_queue_north(),
      s: lab.cached_queue_south(),
      e: lab.cached_queue_east(),
      w: lab.cached_queue_west(),
    };

    // Update animators (only for 1×1 view)
    const gs = lab.grid_size();
    if (gs === 1) {
      ncAnimator.update(ncQueues, dtSec, lc);
      caAnimator.update(caQueues, dtSec, lc);
    }

    const noCache = {
      signalPhase: lab.nocache_signal_phase(),
      queues: ncQueues,
      tick:       lab.nocache_tick(),
      simSeconds: lab.nocache_sim_seconds(),
      vehicles:   lab.nocache_vehicles_processed(),
      discharged: lab.nocache_vehicles_discharged(),
      avgWait:    lab.nocache_avg_wait_sec(),
      tps:        lab.tps_nocache(),
      laneCount:  lc,
      _animator:  ncAnimator,
      isCached:   false,
    };

    const cached = {
      signalPhase: lab.cached_signal_phase(),
      queues: caQueues,
      tick:          lab.cached_tick(),
      simSeconds:    lab.cached_sim_seconds(),
      vehicles:      lab.cached_vehicles_processed(),
      discharged:    lab.cached_vehicles_discharged(),
      avgWait:       lab.cached_avg_wait_sec(),
      tps:           lab.tps_cached(),
      laneCount:     lc,
      _animator:     caAnimator,
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
