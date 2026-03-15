/**
 * idm.js — IDM (Intelligent Driver Model) smooth-traffic simulation
 *
 * Each vehicle follows the IDM acceleration formula:
 *   a = a_max * [1 - (v/v0)^4 - (s*(v,dv)/gap)^2]
 * where s*(v,dv) = s0 + vT + v*dv / (2*sqrt(a_max*b))
 *
 * Signal braking: treated as a virtual stationary vehicle at the stop line.
 * Poisson arrivals: vehicles spawn off-canvas and drive in.
 */

// ── IDM parameters ─────────────────────────────────────────────────

const IDM_V0   = 15;    // desired speed (m/s) ≈ 54 km/h
const IDM_T    = 1.5;   // desired time headway (s)
const IDM_S0   = 2.0;   // minimum gap (m)
const IDM_AMAX = 1.5;   // max acceleration (m/s²)
const IDM_B    = 2.0;   // comfortable braking (m/s²)
const IDM_LEN  = 5.0;   // average vehicle length (m)

// Scale: pixels per meter (canvas units)
const M2PX = 8;

// ── Road geometry (canvas coordinates) ────────────────────────────
// Four arms: N, S, E, W — each carries vehicles toward the centre.
// pos: distance from stop-line (positive = approaching, negative = past stop-line/exited)
// 0 = stop-line. Vehicles spawn at pos = ROAD_M + small random offset.

const ROAD_M    = 35;   // road length in metres (≈ 280 px)
const SPAWN_EXTRA = 5;  // extra metres beyond road edge for spawning

// ── Vehicle type palette ───────────────────────────────────────────
const IDM_VT = [
  { label: 'Car',         color: '#e05252', len: 4.5, wid: 2.0, pct: 60 },
  { label: 'Bus',         color: '#e8c84a', len: 12,  wid: 2.8, pct:  5 },
  { label: 'Truck',       color: '#5270e0', len: 8.0, wid: 2.5, pct: 10 },
  { label: 'Motorcycle',  color: '#52c87a', len: 2.2, wid: 1.0, pct: 20 },
  { label: 'Auto',        color: '#a052e0', len: 3.5, wid: 1.8, pct:  5 },
];

function pickVehicleType() {
  const r = Math.random() * 100;
  let acc = 0;
  for (const vt of IDM_VT) {
    acc += vt.pct;
    if (r < acc) return vt;
  }
  return IDM_VT[0];
}

// ── IDM core ──────────────────────────────────────────────────────

function idmAccel(v, gap, vLead, vt) {
  const dv   = v - vLead;
  const sStar = IDM_S0 + Math.max(0, v * IDM_T + v * dv / (2 * Math.sqrt(IDM_AMAX * IDM_B)));
  const sRatio = sStar / Math.max(gap, 0.1);
  return IDM_AMAX * (1 - Math.pow(v / IDM_V0, 4) - sRatio * sRatio);
}

// ── Signal state helper ───────────────────────────────────────────
// phase 0 = NS green/EW red, 1 = yellow, 2 = NS red/EW green
function isGreen(arm, phase) {
  if (arm === 'n' || arm === 's') return phase === 0;
  return phase === 2;
}

// ── IDM Intersection ──────────────────────────────────────────────

export class IDMIntersection {
  constructor() {
    this.vehicles = { n: [], s: [], e: [], w: [] };
    this.nextId   = 0;

    // Signal state
    this.cycleMs  = 60000;  // total cycle length in ms
    this.elapsed  = 0;      // ms within the current cycle
    this.phase    = 0;      // 0=NS-green, 1=yellow, 2=EW-green, 3=yellow(EW→NS)

    // Arrival model
    this.vpm      = 120;    // vehicles per minute total (divided among 4 arms)
    this.spawnAcc = { n:0, s:0, e:0, w:0 };  // fractional spawn accumulator

    // Stats
    this.throughput = 0;    // vehicles that exited
    this.waitSum    = 0;    // sum of wait times (s) for exited vehicles
  }

  // ── Configuration ───────────────────────────────────────────────

  setVpm(v)       { this.vpm = v; }
  setCycleMs(ms)  { this.cycleMs = ms; this.elapsed = 0; this.phase = 0; }

  // ── Signal phase ────────────────────────────────────────────────

  _updateSignal(dtMs) {
    this.elapsed += dtMs;
    const c = this.cycleMs;
    // Phase durations: 45% NS-green, 5% yellow, 45% EW-green, 5% yellow
    const durations = [c * 0.45, c * 0.05, c * 0.45, c * 0.05];
    let boundary = 0;
    for (let i = 0; i < 4; i++) {
      boundary += durations[i];
      if (this.elapsed < boundary) { this.phase = i; return; }
    }
    this.elapsed -= c;
    this.phase = 0;
  }

  // Returns effective phase for drawing: 0=NS-green,1=yellow,2=EW-green
  get drawPhase() {
    if (this.phase === 0) return 0;
    if (this.phase === 1) return 1;
    if (this.phase === 2) return 2;
    return 1; // phase 3 = yellow before NS
  }

  // ── Spawning ────────────────────────────────────────────────────

  _spawn(dtSec) {
    const armRate = (this.vpm / 60) / 4;  // vehicles/sec per arm
    for (const arm of ['n', 's', 'e', 'w']) {
      this.spawnAcc[arm] += armRate * dtSec;
      while (this.spawnAcc[arm] >= 1) {
        this.spawnAcc[arm] -= 1;
        const vt = pickVehicleType();
        // Compute spawn pos: ROAD_M + random jitter, measured from stop-line
        const spawnPos = ROAD_M + SPAWN_EXTRA * Math.random();
        this.vehicles[arm].push({
          id:      this.nextId++,
          pos:     spawnPos,   // m from stop-line (positive = incoming)
          vel:     IDM_V0 * (0.6 + 0.4 * Math.random()),
          vt,
          spawnTime: this._simTimeSec,
          waiting: 0,
        });
      }
    }
  }

  // ── Physics step ────────────────────────────────────────────────

  step(dtSec) {
    this._simTimeSec = (this._simTimeSec || 0) + dtSec;
    this._updateSignal(dtSec * 1000);
    this._spawn(dtSec);

    for (const arm of ['n', 's', 'e', 'w']) {
      const list = this.vehicles[arm];
      // Sort by pos descending: closest to stop-line first (lowest pos)
      list.sort((a, b) => a.pos - b.pos);

      const green = isGreen(arm, this.phase);

      for (let i = 0; i < list.length; i++) {
        const v = list[i];
        let gap, vLead;

        if (i === 0) {
          // Leader — only the signal / intersection matters
          if (green || v.pos < 0) {
            // No obstacle — free-flow or already through
            gap   = 999;
            vLead = IDM_V0;
          } else {
            // Red/yellow — virtual stopped vehicle at stop-line
            gap   = Math.max(v.pos - v.vt.len / 2, 0.1);
            vLead = 0;
          }
        } else {
          const leader = list[i - 1];
          gap   = Math.max(v.pos - leader.pos - leader.vt.len, IDM_S0 * 0.5);
          vLead = leader.vel;
        }

        const a = idmAccel(v.vel, gap, vLead, v.vt);
        v.vel = Math.max(0, v.vel + a * dtSec);
        v.pos -= v.vel * dtSec;

        // Track wait time (when velocity < 0.5 m/s near stop-line)
        if (v.vel < 0.5 && v.pos > 0 && v.pos < ROAD_M) {
          v.waiting = (v.waiting || 0) + dtSec;
        }
      }

      // Remove vehicles that have cleared the intersection far side
      const [exited, remaining] = partition(list, v => v.pos < -(ROAD_M * 0.5));
      this.vehicles[arm] = remaining;
      for (const v of exited) {
        this.throughput++;
        this.waitSum += v.waiting || 0;
      }
    }
  }

  avgWaitSec() {
    return this.throughput > 0 ? this.waitSum / this.throughput : 0;
  }
}

function partition(arr, pred) {
  const yes = [], no = [];
  for (const x of arr) (pred(x) ? yes : no).push(x);
  return [yes, no];
}

// ── IDMRenderer ───────────────────────────────────────────────────
// Draws a single IDM intersection centred in the given canvas region.

export class IDMRenderer {
  constructor(canvas) {
    this.canvas = canvas;
    this.ctx    = canvas.getContext('2d');
    this.W      = canvas.width;
    this.H      = canvas.height;
  }

  draw(sim) {
    const { ctx, W, H } = this;
    const cx = W / 2;
    const cy = H / 2;

    ctx.clearRect(0, 0, W, H);

    // Background
    ctx.fillStyle = '#111114';
    ctx.fillRect(0, 0, W, H);

    const lanes   = 3;              // fixed for IDM tab (could be parameterised later)
    const laneWM  = 3.5;           // lane width in metres
    const roadM   = lanes * laneWM; // total road half-width in m
    const roadPx  = roadM * M2PX;  // half-road in px

    this._drawRoads(cx, cy, roadPx, ROAD_M * M2PX);
    this._drawIntersectionBox(cx, cy, roadPx);
    this._drawSignals(cx, cy, roadPx, sim.drawPhase);
    this._drawVehicles(cx, cy, sim, roadPx, lanes, laneWM);
    this._drawStats(sim);
  }

  _drawRoads(cx, cy, roadPx, roadLenPx) {
    const { ctx } = this;
    // Road surface
    ctx.fillStyle = '#222228';
    // North road
    ctx.fillRect(cx - roadPx, cy - roadPx - roadLenPx, roadPx * 2, roadLenPx);
    // South road
    ctx.fillRect(cx - roadPx, cy + roadPx, roadPx * 2, roadLenPx);
    // West road
    ctx.fillRect(cx - roadPx - roadLenPx, cy - roadPx, roadLenPx, roadPx * 2);
    // East road
    ctx.fillRect(cx + roadPx, cy - roadPx, roadLenPx, roadPx * 2);

    // Centre lines (dashed)
    ctx.strokeStyle = '#ffff0044';
    ctx.lineWidth   = 1;
    ctx.setLineDash([8, 10]);
    // N/S centre
    ctx.beginPath(); ctx.moveTo(cx, cy - roadPx - roadLenPx); ctx.lineTo(cx, cy - roadPx); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(cx, cy + roadPx); ctx.lineTo(cx, cy + roadPx + roadLenPx); ctx.stroke();
    // E/W centre
    ctx.beginPath(); ctx.moveTo(cx - roadPx - roadLenPx, cy); ctx.lineTo(cx - roadPx, cy); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(cx + roadPx, cy); ctx.lineTo(cx + roadPx + roadLenPx, cy); ctx.stroke();
    ctx.setLineDash([]);
  }

  _drawIntersectionBox(cx, cy, roadPx) {
    const { ctx } = this;
    ctx.fillStyle = '#2a2a30';
    ctx.fillRect(cx - roadPx, cy - roadPx, roadPx * 2, roadPx * 2);
  }

  _drawSignals(cx, cy, roadPx, phase) {
    const { ctx } = this;
    const nsColor = phase === 0 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');
    const ewColor = phase === 2 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');

    const drawLight = (x, y, color) => {
      const r = 7;
      // Faux glow (no shadowBlur — too expensive)
      ctx.fillStyle = color + '30';
      ctx.beginPath(); ctx.arc(x, y, r + 6, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = color + '70';
      ctx.beginPath(); ctx.arc(x, y, r + 2, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = color;
      ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2); ctx.fill();
    };

    const o = roadPx + 12;
    drawLight(cx - o, cy - o, nsColor);
    drawLight(cx + o, cy - o, nsColor);
    drawLight(cx - o, cy + o, ewColor);
    drawLight(cx + o, cy + o, ewColor);
  }

  _drawVehicles(cx, cy, sim, roadPx, lanes, laneWM) {
    const { ctx } = this;
    const roadLenPx = ROAD_M * M2PX;

    const armConfig = {
      n: { axis: 'v', dir:  1, baseX: cx, baseY: cy - roadPx, sign: -1 },
      s: { axis: 'v', dir: -1, baseX: cx, baseY: cy + roadPx, sign:  1 },
      e: { axis: 'h', dir: -1, baseX: cx + roadPx, baseY: cy, sign:  1 },
      w: { axis: 'h', dir:  1, baseX: cx - roadPx, baseY: cy, sign: -1 },
    };

    for (const [arm, cfg] of Object.entries(armConfig)) {
      const list = sim.vehicles[arm];
      for (let i = 0; i < list.length; i++) {
        const v   = list[i];
        const posPx = v.pos * M2PX;   // distance from stop-line in px
        const laneIdx = i % lanes;
        const laneOffset = (laneIdx - (lanes - 1) / 2) * laneWM * M2PX;

        const lenPx = v.vt.len * M2PX;
        const widPx = v.vt.wid * M2PX;

        let vx, vy, rw, rh;
        if (cfg.axis === 'v') {
          vx = cfg.baseX + laneOffset;
          vy = cfg.baseY + cfg.sign * posPx;
          rw = widPx;
          rh = lenPx;
        } else {
          vx = cfg.baseX + cfg.sign * posPx;
          vy = cfg.baseY + laneOffset;
          rw = lenPx;
          rh = widPx;
        }

        // Clip vehicles to visible road area
        if (posPx > roadLenPx + 20) continue;   // off-screen spawn side
        if (posPx < -(roadLenPx + 20)) continue; // off-screen exit side

        // Speed-based brightness: faster = brighter
        const speedRatio = Math.min(v.vel / IDM_V0, 1);
        const alpha = Math.round(140 + speedRatio * 115).toString(16).padStart(2, '0');

        ctx.fillStyle   = v.vt.color + alpha;
        ctx.strokeStyle = v.vt.color;
        ctx.lineWidth   = 0.5;
        ctx.fillRect(vx - rw / 2, vy - rh / 2, rw, rh);
        ctx.strokeRect(vx - rw / 2, vy - rh / 2, rw, rh);

        // Velocity bar (small indicator at front of vehicle)
        const barLen = Math.round(speedRatio * (rw - 2));
        if (cfg.axis === 'v') {
          ctx.fillStyle = '#ffffff33';
          ctx.fillRect(vx - rw / 2 + 1, vy - rh / 2, barLen, 2);
        } else {
          ctx.fillStyle = '#ffffff33';
          ctx.fillRect(vx - rw / 2, vy - rh / 2 + 1, 2, barLen);
        }
      }
    }
  }

  _drawStats(sim) {
    const { ctx, W, H } = this;
    const throughput = sim.throughput;
    const avgWait    = sim.avgWaitSec().toFixed(1);
    const phase      = ['NS ▶', 'Yellow', 'EW ▶', 'Yellow'][sim.phase];
    const totalVeh   = Object.values(sim.vehicles).reduce((s, a) => s + a.length, 0);

    ctx.font      = '12px "Segoe UI", system-ui, sans-serif';
    ctx.fillStyle = '#aaa';
    const lines = [
      `Signal: ${phase}`,
      `Active vehicles: ${totalVeh}`,
      `Throughput: ${throughput}`,
      `Avg wait: ${avgWait}s`,
    ];
    let y = H - 16 - lines.length * 18;
    for (const line of lines) {
      ctx.fillText(line, 14, y);
      y += 18;
    }
  }
}
