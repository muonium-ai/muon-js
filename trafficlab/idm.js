/**
 * idm.js — IDM (Intelligent Driver Model) smooth-traffic simulation
 *
 * Each vehicle follows the IDM acceleration formula:
 *   a = a_max * [1 - (v/v0)^4 - (s*(v,dv)/gap)^2]
 *
 * Lane model (left-hand traffic / India style, 3 inbound + 3 outbound lanes per arm):
 *   Each arm has LANES inbound lanes on the LEFT side of the road centre.
 *   Opposite arm outbound traffic uses the right side — no overlap.
 *
 *   Road half-width = LANES * LANE_W_M metres.
 *   Inbound lanes:  offset from centre = +laneW*(0.5 .. LANES-0.5)  (right of centre from driver POV)
 *   Outbound lanes: offset from centre = -laneW*(0.5 .. LANES-0.5)  (left of centre from driver POV)
 *   In screen coords this means: vehicles approaching from N travel in the +x half of the N road.
 *
 * Turn model:
 *   On spawn each vehicle is assigned: 'straight' | 'left' | 'right'
 *   After passing pos=0 (stop-line) the vehicle follows a smooth quadratic Bezier
 *   into the correct exit arm/lane, animated by t ∈ [0,1] over TURN_DIST metres.
 *
 * Overlay: vehicle number and directional arrow drawn via ctx.save/restore.
 */

// ── IDM parameters ─────────────────────────────────────────────────

const IDM_V0   = 15;    // desired speed (m/s) ≈ 54 km/h
const IDM_T    = 1.5;   // desired time headway (s)
const IDM_S0   = 2.0;   // minimum gap (m)
const IDM_AMAX = 1.5;   // max acceleration (m/s²)
const IDM_B    = 2.0;   // comfortable braking (m/s²)

// Scale: pixels per meter
const M2PX = 8;

// Road layout
const LANES     = 3;     // inbound lanes per arm (same count outbound)
const LANE_W_M  = 3.5;  // lane width in metres
const ROAD_M    = 36;    // road length in metres from stop-line to edge

const SPAWN_EXTRA  = 6;   // extra metres beyond road edge
const TURN_DIST    = 18;  // metres past stop-line to complete a turn

// Unique lane names per arm:
//   Inbound  lanes : arm-letter + 1..LANES        e.g. N1, N2, N3
//   Outbound lanes : arm-letter + LANES+1..2*LANES e.g. N4, N5, N6
// This means every lane in the whole intersection has a distinct name.
function inboundLabel(arm, lane)  { return arm.toUpperCase() + (lane + 1); }
function outboundLabel(arm, lane) { return arm.toUpperCase() + (LANES + lane + 1); }

// ── Vehicle type palette ───────────────────────────────────────────
const IDM_VT = [
  { label: 'Car',        color: '#e05252', len: 4.5, wid: 2.0, pct: 60 },
  { label: 'Bus',        color: '#e8c84a', len: 12,  wid: 2.8, pct:  5 },
  { label: 'Truck',      color: '#5270e0', len: 8.0, wid: 2.5, pct: 10 },
  { label: 'Motorcycle', color: '#52c87a', len: 2.2, wid: 1.0, pct: 20 },
  { label: 'Auto',       color: '#a052e0', len: 3.5, wid: 1.8, pct:  5 },
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

function pickTurn() {
  const r = Math.random();
  if (r < 0.20) return 'left';
  if (r < 0.40) return 'right';
  return 'straight';
}

// ── Turn routing table ────────────────────────────────────────────
// For each arm + turn direction → exit arm.
const TURN_EXIT = {
  n: { left: 'e', straight: 's', right: 'w' },
  s: { left: 'w', straight: 'n', right: 'e' },
  e: { left: 's', straight: 'w', right: 'n' },
  w: { left: 'n', straight: 'e', right: 's' },
};

// ── IDM core ──────────────────────────────────────────────────────

function idmAccel(v, gap, vLead) {
  const dv    = v - vLead;
  const sStar = IDM_S0 + Math.max(0, v * IDM_T + v * dv / (2 * Math.sqrt(IDM_AMAX * IDM_B)));
  const sRat  = sStar / Math.max(gap, 0.1);
  return IDM_AMAX * (1 - Math.pow(v / IDM_V0, 4) - sRat * sRat);
}

// ── Signal helpers ────────────────────────────────────────────────
function isGreen(arm, phase) {
  if (arm === 'n' || arm === 's') return phase === 0;
  return phase === 2;
}

// ── IDM Intersection ──────────────────────────────────────────────

export class IDMIntersection {
  constructor() {
    this.approaching = { n: [], s: [], e: [], w: [] };  // vehicles before stop-line
    this.turning     = [];   // vehicles mid-turn (Bezier through intersection)
    this.exiting     = { n: [], s: [], e: [], w: [] };  // vehicles after turn, leaving on exit arm
    this.nextId  = 0;
    this.vehNum  = 0;   // sequential display number

    // Signal
    this.cycleMs = 60000;
    this.elapsed = 0;
    this.phase   = 0;

    // Arrival
    this.vpm      = 120;
    this.spawnAcc = { n:0, s:0, e:0, w:0 };

    // Stats
    this.throughput = 0;
    this.waitSum    = 0;
    this._simTimeSec = 0;
  }

  setVpm(v)      { this.vpm = v; }
  setCycleMs(ms) { this.cycleMs = ms; this.elapsed = 0; this.phase = 0; }

  // ── Signal ──────────────────────────────────────────────────────

  _updateSignal(dtMs) {
    this.elapsed += dtMs;
    const c = this.cycleMs;
    const durations = [c * 0.45, c * 0.05, c * 0.45, c * 0.05];
    let boundary = 0;
    for (let i = 0; i < 4; i++) {
      boundary += durations[i];
      if (this.elapsed < boundary) { this.phase = i; return; }
    }
    this.elapsed -= c;
    this.phase = 0;
  }

  get drawPhase() {
    // 0=NS-green, 1=yellow, 2=EW-green, (3=yellow→NS treated as yellow)
    return this.phase <= 1 ? this.phase : (this.phase === 2 ? 2 : 1);
  }

  // ── Spawning ────────────────────────────────────────────────────

  _spawn(dtSec) {
    const armRate = (this.vpm / 60) / 4;
    for (const arm of ['n', 's', 'e', 'w']) {
      this.spawnAcc[arm] += armRate * dtSec;
      while (this.spawnAcc[arm] >= 1) {
        this.spawnAcc[arm] -= 1;
        const vt   = pickVehicleType();
        const turn = pickTurn();
        const lane = Math.floor(Math.random() * LANES);
        const spawnPos = ROAD_M + SPAWN_EXTRA * Math.random();
        // Don't spawn if a vehicle in the same lane is too close to the spawn point
        const tooClose = this.approaching[arm].some(
          v => v.lane === lane && Math.abs(v.pos - spawnPos) < v.vt.len + IDM_S0 * 2
        );
        if (tooClose) continue;
        const exitLane = exitLaneFor(turn, lane);
        this.approaching[arm].push({
          id:        this.nextId++,
          num:       ++this.vehNum,
          arm,
          lane,
          turn,
          exitLane,
          exitArm:   TURN_EXIT[arm][turn],
          pos:       spawnPos,
          vel:       IDM_V0 * (0.6 + 0.4 * Math.random()),
          vt,
          waiting:   0,
          spawnTime: this._simTimeSec,
        });
      }
    }
  }

  // ── Physics step ────────────────────────────────────────────────

  step(dtSec) {
    this._simTimeSec += dtSec;
    this._updateSignal(dtSec * 1000);
    this._spawn(dtSec);

    // ── Approaching vehicles: per-lane IDM + stop-line ──────────────────
    for (const arm of ['n', 's', 'e', 'w']) {
      const green = isGreen(arm, this.phase);
      // Process each lane independently — vehicles in different lanes don't block each other
      for (let lane = 0; lane < LANES; lane++) {
        const lv = this.approaching[arm]
          .filter(v => v.lane === lane)
          .sort((a, b) => a.pos - b.pos);  // closest to stop-line first

        for (let i = 0; i < lv.length; i++) {
          const v = lv[i];
          let gap, vLead;
          if (i === 0) {
            if (green || v.pos <= 0) { gap = 999; vLead = IDM_V0; }
            else { gap = Math.max(v.pos - v.vt.len / 2, 0.1); vLead = 0; }
          } else {
            const leader = lv[i - 1];
            gap   = Math.max(v.pos - leader.pos - leader.vt.len, IDM_S0 * 0.5);
            vLead = leader.vel;
          }
          const a = idmAccel(v.vel, gap, vLead);
          v.vel = Math.max(0, v.vel + a * dtSec);
          v.pos -= v.vel * dtSec;
          if (v.vel < 0.5 && v.pos > 0 && v.pos < ROAD_M) v.waiting += dtSec;
        }
      }

      // Move vehicles that crossed the stop-line into the turning list
      const [crossed, remaining] = partition(this.approaching[arm], v => v.pos <= 0);
      this.approaching[arm] = remaining;
      for (const v of crossed) {
        // exitArm and exitLane already set at spawn; preserve them
        this.turning.push({ ...v, turnPos: 0, turnSpeed: Math.max(v.vel, 2) });
      }
    }

    // ── Turning vehicles (Bezier arc through intersection) ───────
    const stillTurning = [];
    for (const v of this.turning) {
      v.turnSpeed = Math.min(IDM_V0 * 0.65, v.turnSpeed + IDM_AMAX * dtSec);
      v.turnPos  += (v.turnSpeed * dtSec) / TURN_DIST;

      if (v.turnPos >= 1) {
        // Graduate onto exit arm as an outbound vehicle
        this.throughput++;
        this.waitSum += v.waiting;
        this.exiting[v.exitArm].push({
          ...v,
          arm:  v.exitArm,
          lane: v.exitLane,
          pos:  0,          // 0 = at exit stop-line, increases as vehicle moves away
          vel:  v.turnSpeed,
        });
      } else {
        stillTurning.push(v);
      }
    }
    this.turning = stillTurning;

    // ── Exiting vehicles: per-lane IDM, pos increases away from intersection ──
    for (const arm of ['n', 's', 'e', 'w']) {
      for (let lane = 0; lane < LANES; lane++) {
        const lv = this.exiting[arm]
          .filter(v => v.lane === lane)
          .sort((a, b) => b.pos - a.pos);  // furthest from intersection first = leader

        for (let i = 0; i < lv.length; i++) {
          const v = lv[i];
          let gap, vLead;
          if (i === 0) {
            gap = 999; vLead = IDM_V0;  // leader: free flow
          } else {
            const leader = lv[i - 1];  // further ahead (higher pos)
            gap   = Math.max(leader.pos - v.pos - leader.vt.len, IDM_S0 * 0.5);
            vLead = leader.vel;
          }
          const a = idmAccel(v.vel, gap, vLead);
          v.vel = Math.max(0, v.vel + a * dtSec);
          v.pos += v.vel * dtSec;  // pos increases (moving away from intersection)
        }
      }
      // Remove vehicles that have cleared the visible road
      this.exiting[arm] = this.exiting[arm].filter(v => v.pos < ROAD_M + SPAWN_EXTRA + 4);
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

// Map turn direction + inbound lane → exit lane number
// right turn = outer lane (kerb side = 0), left = inner (centre side = LANES-1), straight = same
function exitLaneFor(turn, inLane) {
  if (turn === 'right') return 0;
  if (turn === 'left')  return LANES - 1;
  return inLane;
}

// ── Geometry helpers ──────────────────────────────────────────────

/**
 * Return the screen (x,y) and heading angle (radians) for a vehicle on an arm,
 * given the intersection centre (cx,cy), road half-width (roadPx), and pos in metres.
 *
 * Lane model: inbound traffic keeps LEFT of the road centreline (UK-style / Indian roads).
 * From the driver's perspective they are on the left; in canvas coords:
 *   N arm:  inbound (travelling south) → x slightly RIGHT of cx
 *   S arm:  inbound (travelling north) → x slightly LEFT  of cx
 *   E arm:  inbound (travelling west)  → y slightly BELOW  cy
 *   W arm:  inbound (travelling east)  → y slightly ABOVE  cy
 *
 * laneOffset: 0 = nearest centre (inner), LANES-1 = nearest kerb (outer)
 *   inbound side starts at +0.5*laneW from centre and increases outward.
 */
function armPos(arm, lane, posPx, cx, cy, roadPx) {
  const laneWPx  = roadPx / LANES;
  // inbound offset from road centre (positive = away from centre on inbound side)
  const inOffset = (lane + 0.5) * laneWPx;

  // Angle convention: after ctx.rotate(angle), local-y must point in the direction of travel.
  // ctx.rotate(θ) maps local-y → screen direction (-sinθ, cosθ).
  //   N (travel south  = screen +y): need cosθ=1, sinθ=0  → θ = 0
  //   S (travel north  = screen -y): need cosθ=-1,sinθ=0  → θ = π
  //   E (travel west   = screen -x): need sinθ=-1,cosθ=0  → θ = -π/2  (local-y→screen-left)
  //   W (travel east   = screen +x): need sinθ=1, cosθ=0  → θ = π/2   (local-y→screen-right)
  // Wait — for E: local-y→(-sinθ,cosθ)=(-1,0)=screen-left ✓ when θ=π/2.
  //   sinθ=1,cosθ=0 → θ=π/2.  For W: (-sinθ,cosθ)=(1,0)=screen-right ✓ when θ=-π/2.
  switch (arm) {
    case 'n':
      // travelling south (screen +y), inbound lane right of centre (+x)
      return { x: cx + inOffset, y: cy - roadPx - posPx, angle: 0 };
    case 's':
      // travelling north (screen -y), inbound lane left of centre (-x)
      return { x: cx - inOffset, y: cy + roadPx + posPx, angle: Math.PI };
    case 'e':
      // travelling west (screen -x), inbound lane below centre (+y)
      return { x: cx + roadPx + posPx, y: cy + inOffset, angle: Math.PI / 2 };
    case 'w':
      // travelling east (screen +x), inbound lane above centre (-y)
      return { x: cx - roadPx - posPx, y: cy - inOffset, angle: -Math.PI / 2 };
  }
}

/**
 * Smooth turn: quadratic Bezier from approach stop-line point → exit stop-line point.
 * t ∈ [0,1]. Returns {x, y, angle}.
 */
function turnBezier(v, t, cx, cy, roadPx) {
  const laneWPx   = roadPx / LANES;
  const inOffset  = (v.lane + 0.5) * laneWPx;
  const outOffset = (v.exitLane + 0.5) * laneWPx;

  // Start point: stop-line of entry arm
  let p0, p2, c1;
  switch (v.arm) {
    case 'n': p0 = { x: cx + inOffset,  y: cy - roadPx }; break;
    case 's': p0 = { x: cx - inOffset,  y: cy + roadPx }; break;
    case 'e': p0 = { x: cx + roadPx,    y: cy + inOffset }; break;
    case 'w': p0 = { x: cx - roadPx,    y: cy - inOffset }; break;
  }
  // End point: stop-line of exit arm (outbound = on opposite side)
  switch (v.exitArm) {
    case 'n': p2 = { x: cx - outOffset, y: cy - roadPx }; break;
    case 's': p2 = { x: cx + outOffset, y: cy + roadPx }; break;
    case 'e': p2 = { x: cx + roadPx,    y: cy - outOffset }; break;
    case 'w': p2 = { x: cx - roadPx,    y: cy + outOffset }; break;
  }

  // Control point: intersection centre (biased slightly for turn direction)
  const bias = v.turn === 'left' ? 0.6 : (v.turn === 'right' ? -0.6 : 0);
  c1 = { x: cx + (p2.x - p0.x) * bias * 0.25 + cx * 0,
          y: cy + (p2.y - p0.y) * bias * 0.25 + cy * 0 };
  // Use intersection centre as control point (simple but smooth)
  c1 = { x: cx, y: cy };

  // Quadratic Bezier
  const mt = 1 - t;
  const bx = mt * mt * p0.x + 2 * mt * t * c1.x + t * t * p2.x;
  const by = mt * mt * p0.y + 2 * mt * t * c1.y + t * t * p2.y;

  // Tangent direction: atan2(ty, tx) gives screen angle of velocity vector.
  // We need draw angle θ such that local-y aligns with travel: (-sinθ, cosθ) = normalised(tx_,ty_).
  // Solving: θ = atan2(tx_, -ty_)  — equivalent to atan2(ty_,tx_) - π/2
  const tx_ = 2 * (1 - t) * (c1.x - p0.x) + 2 * t * (p2.x - c1.x);
  const ty_ = 2 * (1 - t) * (c1.y - p0.y) + 2 * t * (p2.y - c1.y);
  const angle = Math.atan2(tx_, -ty_);  // rotate 90° so local-y → travel direction

  return { x: bx, y: by, angle };
}

// Arrow characters per turn direction
const TURN_ARROW = { straight: '↑', left: '←', right: '→' };

/**
 * armPosOut — screen position for a vehicle that has exited the intersection
 * and is traveling AWAY on arm `arm`. pos=0 = stop-line, pos increases toward canvas edge.
 * Outbound traffic uses the opposite side of road from inbound.
 */
function armPosOut(arm, lane, posPx, cx, cy, roadPx) {
  const laneWPx   = roadPx / LANES;
  const outOffset = (lane + 0.5) * laneWPx;  // offset on outbound side
  switch (arm) {
    case 'n': return { x: cx - outOffset, y: cy - roadPx - posPx, angle: Math.PI };
    case 's': return { x: cx + outOffset, y: cy + roadPx + posPx, angle: 0 };
    case 'e': return { x: cx + roadPx + posPx, y: cy - outOffset, angle: -Math.PI / 2 };
    case 'w': return { x: cx - roadPx - posPx, y: cy + outOffset, angle:  Math.PI / 2 };
  }
}

// ── IDMRenderer ───────────────────────────────────────────────────

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
    ctx.fillStyle = '#111114';
    ctx.fillRect(0, 0, W, H);

    const roadPx    = LANES * LANE_W_M * M2PX;  // half-road width in px
    const roadLenPx = ROAD_M * M2PX;

    this._drawRoads(cx, cy, roadPx, roadLenPx);
    this._drawIntersectionBox(cx, cy, roadPx);
    this._drawSignals(cx, cy, roadPx, sim.drawPhase);
    this._drawLaneLabels(cx, cy, roadPx);
    this._drawVehicles(cx, cy, sim, roadPx, roadLenPx);
    this._drawStats(sim);
  }

  _drawRoads(cx, cy, roadPx, roadLenPx) {
    const { ctx } = this;
    ctx.fillStyle = '#222228';
    ctx.fillRect(cx - roadPx, cy - roadPx - roadLenPx, roadPx * 2, roadLenPx);  // N
    ctx.fillRect(cx - roadPx, cy + roadPx,             roadPx * 2, roadLenPx);  // S
    ctx.fillRect(cx - roadPx - roadLenPx, cy - roadPx, roadLenPx,  roadPx * 2); // W
    ctx.fillRect(cx + roadPx,             cy - roadPx, roadLenPx,  roadPx * 2); // E

    // Road centre divider (solid yellow)
    ctx.strokeStyle = '#ffff0066';
    ctx.lineWidth   = 1.5;
    ctx.setLineDash([]);
    ctx.beginPath(); ctx.moveTo(cx, cy - roadPx - roadLenPx); ctx.lineTo(cx, cy - roadPx); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(cx, cy + roadPx); ctx.lineTo(cx, cy + roadPx + roadLenPx); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(cx - roadPx - roadLenPx, cy); ctx.lineTo(cx - roadPx, cy); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(cx + roadPx, cy); ctx.lineTo(cx + roadPx + roadLenPx, cy); ctx.stroke();

    // Lane markers (dashed white)
    const laneWPx = (roadPx * 2) / (LANES * 2);  // each lane width
    ctx.strokeStyle = '#ffffff22';
    ctx.lineWidth   = 1;
    ctx.setLineDash([8, 10]);
    for (let l = 1; l < LANES * 2; l++) {
      if (l === LANES) continue; // skip centre line (already drawn)
      const off = -roadPx + l * laneWPx;
      // N/S arms: vertical lanes
      ctx.beginPath(); ctx.moveTo(cx + off, cy - roadPx - roadLenPx); ctx.lineTo(cx + off, cy - roadPx); ctx.stroke();
      ctx.beginPath(); ctx.moveTo(cx + off, cy + roadPx); ctx.lineTo(cx + off, cy + roadPx + roadLenPx); ctx.stroke();
      // E/W arms: horizontal lanes
      ctx.beginPath(); ctx.moveTo(cx - roadPx - roadLenPx, cy + off); ctx.lineTo(cx - roadPx, cy + off); ctx.stroke();
      ctx.beginPath(); ctx.moveTo(cx + roadPx, cy + off); ctx.lineTo(cx + roadPx + roadLenPx, cy + off); ctx.stroke();
    }
    ctx.setLineDash([]);
  }

  // Draw lane name badges at the road edge for all 4 arms.
  // Inbound  badges (warm amber)  : arm-letter + 1-3  e.g. N1 N2 N3  – on the inbound lane side
  // Outbound badges (cool cyan)   : arm-letter + 4-6  e.g. N4 N5 N6  – on the outbound lane side
  _drawLaneLabels(cx, cy, roadPx) {
    const { ctx }  = this;
    const laneWPx  = roadPx / LANES;
    const BADGE_R  = 8;
    const OFFSET   = roadPx + BADGE_R + 6;  // distance from centre to badge centre

    ctx.font         = `bold ${BADGE_R * 1.15}px monospace`;
    ctx.textAlign    = 'center';
    ctx.textBaseline = 'middle';

    // inbound badge colours (warm) and outbound badge colours (cool)
    const IN_COLORS  = ['#ffb347', '#ff7f7f', '#ffec6e'];  // amber/red/yellow
    const OUT_COLORS = ['#4ec9ff', '#7fffb2', '#c47fff'];  // cyan/green/purple

    const drawBadge = (x, y, label, bg) => {
      ctx.beginPath();
      ctx.arc(x, y, BADGE_R, 0, Math.PI * 2);
      ctx.fillStyle   = bg + 'bb';
      ctx.fill();
      ctx.strokeStyle = bg;
      ctx.lineWidth   = 1;
      ctx.stroke();
      ctx.fillStyle   = '#111';
      ctx.fillText(label, x, y);
    };

    for (let lane = 0; lane < LANES; lane++) {
      const off = (lane + 0.5) * laneWPx;
      const inC  = IN_COLORS[lane];
      const outC = OUT_COLORS[lane];

      // N arm  — inbound: +x side,  outbound: −x side
      drawBadge(cx + off, cy - OFFSET, inboundLabel('n', lane),  inC);
      drawBadge(cx - off, cy - OFFSET, outboundLabel('n', lane), outC);

      // S arm  — inbound: −x side,  outbound: +x side
      drawBadge(cx - off, cy + OFFSET, inboundLabel('s', lane),  inC);
      drawBadge(cx + off, cy + OFFSET, outboundLabel('s', lane), outC);

      // E arm  — inbound: +y side,  outbound: −y side
      drawBadge(cx + OFFSET, cy + off, inboundLabel('e', lane),  inC);
      drawBadge(cx + OFFSET, cy - off, outboundLabel('e', lane), outC);

      // W arm  — inbound: −y side,  outbound: +y side
      drawBadge(cx - OFFSET, cy - off, inboundLabel('w', lane),  inC);
      drawBadge(cx - OFFSET, cy + off, outboundLabel('w', lane), outC);
    }
  }

  _drawIntersectionBox(cx, cy, roadPx) {
    this.ctx.fillStyle = '#2a2a30';
    this.ctx.fillRect(cx - roadPx, cy - roadPx, roadPx * 2, roadPx * 2);
  }

  _drawSignals(cx, cy, roadPx, phase) {
    const { ctx } = this;
    const nsColor = phase === 0 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');
    const ewColor = phase === 2 ? '#52c87a' : (phase === 1 ? '#e8c84a' : '#e05252');

    const drawLight = (x, y, color) => {
      const r = 7;
      ctx.fillStyle = color + '30';
      ctx.beginPath(); ctx.arc(x, y, r + 6, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = color + '70';
      ctx.beginPath(); ctx.arc(x, y, r + 2, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = color;
      ctx.beginPath(); ctx.arc(x, y, r,     0, Math.PI * 2); ctx.fill();
    };

    const o = roadPx + 14;
    drawLight(cx - o, cy - o, nsColor);
    drawLight(cx + o, cy - o, nsColor);
    drawLight(cx - o, cy + o, ewColor);
    drawLight(cx + o, cy + o, ewColor);
  }

  _drawVehicles(cx, cy, sim, roadPx, roadLenPx) {
    const { ctx } = this;

    // ── Approaching vehicles ───────────────────────────────────
    for (const arm of ['n', 's', 'e', 'w']) {
      for (const v of sim.approaching[arm]) {
        const posPx = v.pos * M2PX;
        if (posPx > roadLenPx + 20) continue;  // not yet on screen

        const { x, y, angle } = armPos(arm, v.lane, posPx, cx, cy, roadPx);
        this._drawVehicle(v, x, y, angle);
      }
    }

    // ── Turning vehicles (Bezier path) ─────────────────────────
    for (const v of sim.turning) {
      const { x, y, angle } = turnBezier(v, v.turnPos, cx, cy, roadPx);
      this._drawVehicle(v, x, y, angle);
    }

    // ── Exiting vehicles (outbound, leaving the intersection) ───
    for (const arm of ['n', 's', 'e', 'w']) {
      for (const v of sim.exiting[arm]) {
        const posPx = v.pos * M2PX;
        if (posPx > roadLenPx + 20) continue;  // off canvas
        const { x, y, angle } = armPosOut(arm, v.lane, posPx, cx, cy, roadPx);
        this._drawVehicle(v, x, y, angle);
      }
    }
  }

  _drawVehicle(v, cx, cy, angle) {
    const { ctx } = this;
    const lenPx = v.vt.len * M2PX;
    const widPx = v.vt.wid * M2PX;
    const speedRatio = Math.min(v.vel / IDM_V0, 1);
    const alpha = Math.round(140 + speedRatio * 115).toString(16).padStart(2, '0');

    ctx.save();
    ctx.translate(cx, cy);
    ctx.rotate(angle);

    // Body
    ctx.fillStyle   = v.vt.color + alpha;
    ctx.strokeStyle = v.vt.color;
    ctx.lineWidth   = 0.8;
    ctx.fillRect(-widPx / 2, -lenPx / 2, widPx, lenPx);
    ctx.strokeRect(-widPx / 2, -lenPx / 2, widPx, lenPx);

    // Speed bar at front
    const barLen = Math.round(speedRatio * (widPx - 2));
    ctx.fillStyle = '#ffffff44';
    ctx.fillRect(-widPx / 2 + 1, -lenPx / 2, barLen, 2);

    // Number
    const numSize = Math.min(widPx * 0.55, 9);
    ctx.font         = `bold ${numSize}px monospace`;
    ctx.fillStyle    = '#ffffffcc';
    ctx.textAlign    = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(String(v.num % 100), 0, -lenPx * 0.18);

    // Destination badge: turn arrow + unique outbound lane name (e.g. "→N5")
    // exitArm and exitLane are both set at spawn so all vehicles, including approaching,
    // already know their destination lane.
    const destStr = (TURN_ARROW[v.turn] || '↑') + outboundLabel(v.exitArm, v.exitLane ?? 0);
    const arrSize = Math.min(widPx * 0.52, 8);
    ctx.font      = `bold ${arrSize}px monospace`;
    ctx.fillStyle = '#ffe040dd';
    ctx.fillText(destStr, 0, lenPx * 0.25);

    ctx.restore();
  }

  _drawStats(sim) {
    const { ctx, W, H } = this;
    const totalVeh = Object.values(sim.approaching).reduce((s, a) => s + a.length, 0)
                   + sim.turning.length
                   + Object.values(sim.exiting).reduce((s, a) => s + a.length, 0);
    const phase    = ['NS ▶', 'Yellow ●', 'EW ▶', 'Yellow ●'][sim.phase];
    const lines = [
      `Signal: ${phase}`,
      `Active: ${totalVeh}  Throughput: ${sim.throughput}`,
      `Avg wait: ${sim.avgWaitSec().toFixed(1)}s`,
    ];
    ctx.font      = '12px "Segoe UI", system-ui, sans-serif';
    ctx.fillStyle = '#aaa';
    let y = H - 14 - lines.length * 16;
    for (const line of lines) { ctx.fillText(line, 14, y); y += 16; }
  }
}
