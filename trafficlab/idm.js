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

const SPAWN_EXTRA     = 6;    // extra metres beyond road edge
const TURN_DIST       = 18;   // metres past stop-line to complete a turn
const SAFETY_BUFFER_M = 1.5;  // extra gap headroom beyond vehicle length (all phases)

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

// ── VehiclePositionCache ─────────────────────────────────────────
// Lightweight in-JS position registry shaped after the MuonCache HSET/HGET/HDEL
// API.  One snapshot per physics frame lets every collision query read consistent,
// pre-computed screen coordinates instead of re-running turnBezier per pair.
class VehiclePositionCache {
  constructor() { this._store = new Map(); }

  /** Mirrors MuonCache HSET key field value */
  hset(id, field, value) {
    let rec = this._store.get(id);
    if (!rec) { rec = Object.create(null); this._store.set(id, rec); }
    rec[field] = value;
  }

  /** Mirrors MuonCache HGET key field */
  hget(id, field) { return this._store.get(id)?.[field]; }

  /** Mirrors MuonCache HDEL key */
  hdel(id) { this._store.delete(id); }

  /** Snapshot all registered ids (array copy — safe to mutate cache during loop). */
  ids() { return [...this._store.keys()]; }

  clear() { this._store.clear(); }
}

// ── IDM core ──────────────────────────────────────────────────────

function idmAccel(v, gap, vLead) {
  const dv    = v - vLead;
  const sStar = IDM_S0 + Math.max(0, v * IDM_T + v * dv / (2 * Math.sqrt(IDM_AMAX * IDM_B)));
  const sRat  = sStar / Math.max(gap, 0.1);
  return IDM_AMAX * (1 - Math.pow(v / IDM_V0, 4) - sRat * sRat);
}

// ── Signal helpers ────────────────────────────────────────────────
// 8-phase sequential cycle: N-green, N-yellow, E-green, E-yellow,
//                            S-green, S-yellow, W-green, W-yellow.
// Only one arm flows at a time — no crossing traffic, no central congestion.
const PHASE_ARM   = ['n', 'n', 'e', 'e', 's', 's', 'w', 'w'];
const PHASE_STATE = ['green', 'yellow', 'green', 'yellow', 'green', 'yellow', 'green', 'yellow'];

function isGreen(arm, phase) {
  return PHASE_STATE[phase] === 'green' && PHASE_ARM[phase] === arm;
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

    // Per-frame position cache (MuonCache API shape) — rebuilt at the top of every step()
    this._posCache = new VehiclePositionCache();

    // Set to [vA, vB] on first detected overlap; step() becomes a no-op after this.
    this.collisionPair = null;
  }

  setVpm(v)      { this.vpm = v; }
  setCycleMs(ms) { this.cycleMs = ms; this.elapsed = 0; this.phase = 0; }

  // ── Signal ──────────────────────────────────────────────────────

  _updateSignal(dtMs) {
    this.elapsed += dtMs;
    const c = this.cycleMs;
    // Each arm gets equal green time; yellow is 5% of that slice.
    const slice  = c / 4;                   // time per arm
    const yellow = slice * 0.10;            // 10% of slice = yellow
    const green  = slice - yellow;          // 90% = green
    const durations = [green, yellow, green, yellow, green, yellow, green, yellow];
    let boundary = 0;
    for (let i = 0; i < 8; i++) {
      boundary += durations[i];
      if (this.elapsed < boundary) { this.phase = i; return; }
    }
    this.elapsed -= c;
    this.phase = 0;
  }

  get drawPhase() {
    // Returns { arm, state } so the renderer can colour each signal independently.
    return { arm: PHASE_ARM[this.phase], state: PHASE_STATE[this.phase] };
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
    if (this.collisionPair) return;  // frozen after collision
    this._simTimeSec += dtSec;
    this._updateSignal(dtSec * 1000);
    this._spawn(dtSec);

    // ── Pre-snapshot: write every turning vehicle's screen position into the cache ──
    // Computed once per frame (MuonCache write phase); all collision queries below
    // read from this snapshot instead of re-invoking turnBezier.
    const _roadPx = LANES * LANE_W_M * M2PX;
    this._posCache.clear();
    for (const tv of this.turning) {
      const { x, y } = turnBezier(tv, tv.turnPos, 0, 0, _roadPx);
      this._posCache.hset(tv.id, 'x', x);
      this._posCache.hset(tv.id, 'y', y);
      this._posCache.hset(tv.id, 'r', (tv.vt.len * 0.5 + SAFETY_BUFFER_M) * M2PX);
    }

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
            if (green) { gap = 999; vLead = IDM_V0; }
            else {
              // gap = distance from front of vehicle to stop-line
              // pos is front-to-stop-line distance, so gap = pos directly
              gap   = Math.max(v.pos, 0.1);
              vLead = 0;
            }
          } else {
            const leader = lv[i - 1];
            gap   = Math.max(v.pos - leader.pos - leader.vt.len - SAFETY_BUFFER_M, IDM_S0 * 0.5);
            vLead = leader.vel;
          }
          const a = idmAccel(v.vel, gap, vLead);
          v.vel = Math.max(0, v.vel + a * dtSec);
          v.pos -= v.vel * dtSec;
          if (v.vel < 0.5 && v.pos > 0 && v.pos < ROAD_M) v.waiting += dtSec;
        }
      }

      // Move vehicles whose FRONT has reached the stop-line (pos <= 0) into turning
      const [crossed, remaining] = partition(this.approaching[arm], v => v.pos <= 0);
      this.approaching[arm] = remaining;
      for (const v of crossed) {
        // Entry gate: sample 3 points along this vehicle's Bezier path and check
        // each against the position cache (MuonCache HGET reads).
        // If blocked, hold the vehicle at the stop-line for the next frame.
        const vr = (v.vt.len * 0.5 + SAFETY_BUFFER_M) * M2PX;
        const existIds = this._posCache.ids();  // snapshot before we might add below
        let blocked = false;
        for (const t of [0, 0.25, 0.5]) {
          const { x: px, y: py } = turnBezier(v, t, 0, 0, _roadPx);
          for (const eid of existIds) {
            const dist = Math.hypot(px - this._posCache.hget(eid, 'x'),
                                    py - this._posCache.hget(eid, 'y'));
            if (dist < vr + this._posCache.hget(eid, 'r')) { blocked = true; break; }
          }
          if (blocked) break;
        }
        if (blocked) {
          // Hold at stop-line; will retry on next green after intersection clears
          v.pos = 0.1;
          v.vel = 0;
          this.approaching[arm].push(v);
        } else {
          // Approved — register in cache (MuonCache HSET) so subsequent crossed
          // vehicles from other arms see this vehicle's entry position.
          const { x: ex, y: ey } = turnBezier(v, 0, 0, 0, _roadPx);
          this._posCache.hset(v.id, 'x', ex);
          this._posCache.hset(v.id, 'y', ey);
          this._posCache.hset(v.id, 'r', vr);
          this.turning.push({ ...v, turnPos: 0, turnSpeed: Math.max(v.vel, 2) });
        }
      }
    }

    // ── Turning vehicles (Bezier arc through intersection) ───────

    // Step 1: free-flow acceleration for all turning vehicles
    for (const v of this.turning) {
      v.turnSpeed = Math.min(IDM_V0 * 0.65, v.turnSpeed + IDM_AMAX * dtSec);
    }

    // Step 2: same-path IDM — vehicles sharing the same exit arm + lane follow each other
    const pathMap = {};
    for (const v of this.turning) {
      const key = v.exitArm + ':' + v.exitLane;
      (pathMap[key] = pathMap[key] || []).push(v);
    }
    for (const grp of Object.values(pathMap)) {
      grp.sort((a, b) => b.turnPos - a.turnPos);  // highest turnPos = leader
      for (let i = 1; i < grp.length; i++) {
        const leader = grp[i - 1];
        const v      = grp[i];
        const gapM   = (leader.turnPos - v.turnPos) * TURN_DIST
                       - leader.vt.len - SAFETY_BUFFER_M;
        const a      = idmAccel(v.turnSpeed, Math.max(gapM, IDM_S0 * 0.5), leader.turnSpeed);
        v.turnSpeed  = Math.max(0.5, v.turnSpeed + a * dtSec);
      }
    }

    // Step 3: cross-path proximity — read cached positions (MuonCache HGET).
    // Hard-stop the yielding vehicle (no minimum speed floor) with a 1.5× look-ahead
    // zone so braking starts well before bodies actually touch.
    if (this.turning.length > 1) {
      for (let i = 0; i < this.turning.length; i++) {
        for (let j = i + 1; j < this.turning.length; j++) {
          const vi = this.turning[i], vj = this.turning[j];
          if (vi.exitArm === vj.exitArm && vi.exitLane === vj.exitLane) continue;
          const xi = this._posCache.hget(vi.id, 'x'), yi = this._posCache.hget(vi.id, 'y');
          const xj = this._posCache.hget(vj.id, 'x'), yj = this._posCache.hget(vj.id, 'y');
          if (xi == null || xj == null) continue;
          const dist     = Math.hypot(xi - xj, yi - yj);
          const safetyPx = this._posCache.hget(vi.id, 'r') + this._posCache.hget(vj.id, 'r');
          if (dist < safetyPx * 1.5) {
            // Proportional hard-stop: squeeze=1 → speed=0, squeeze=0 → no change
            const squeeze = Math.max(0, 1 - dist / safetyPx);
            if (vi.turnPos <= vj.turnPos) {
              vi.turnSpeed = Math.max(0, vi.turnSpeed * (1 - squeeze));
            } else {
              vj.turnSpeed = Math.max(0, vj.turnSpeed * (1 - squeeze));
            }
          }
        }
      }
    }

    // Step 4: advance position and graduate completed turns
    const stillTurning = [];
    for (const v of this.turning) {
      v.turnPos += (v.turnSpeed * dtSec) / TURN_DIST;

      if (v.turnPos >= 1) {
        // Graduate onto exit arm — evict from position cache (MuonCache HDEL)
        this._posCache.hdel(v.id);
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
            gap   = Math.max(leader.pos - v.pos - leader.vt.len - SAFETY_BUFFER_M, IDM_S0 * 0.5);
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

    // ── End-of-step overlap check ─────────────────────────────────
    // Read screen positions from the position cache (already populated above).
    // Only turning vehicles are in the cache; for approaching/exiting we use their
    // 1-D lane position. Within a lane, gaps are guaranteed by IDM; we only need
    // to check turning vehicles against each other (cross-path case).
    const ids = this._posCache.ids();
    outer:
    for (let i = 0; i < ids.length; i++) {
      for (let j = i + 1; j < ids.length; j++) {
        const ai = ids[i], aj = ids[j];
        const dx   = this._posCache.hget(ai, 'x') - this._posCache.hget(aj, 'x');
        const dy   = this._posCache.hget(ai, 'y') - this._posCache.hget(aj, 'y');
        const dist = Math.hypot(dx, dy);
        const minD = this._posCache.hget(ai, 'r') + this._posCache.hget(aj, 'r');
        if (dist < minD * 0.72) {  // 0.72 ≈ half-overlap before triggering freeze
          const va = this.turning.find(v => v.id === ai);
          const vb = this.turning.find(v => v.id === aj);
          if (va && vb) {
            this.collisionPair = [va, vb];
            console.group('%c⚠ IDM COLLISION DETECTED', 'color:#ff4444;font-weight:bold;font-size:14px');
            console.log('Sim time :', this._simTimeSec.toFixed(2), 's');
            console.log('Overlap  :', ((1 - dist / minD) * 100).toFixed(1), '%  (dist', dist.toFixed(1), 'px, minSafe', minD.toFixed(1), 'px)');
            console.table([
              {
                vehicle : `#${va.num} (id ${va.id})`,
                type    : va.vt.label,
                from    : va.arm.toUpperCase(),
                to      : va.exitArm.toUpperCase(),
                turn    : va.turn,
                inLane  : inboundLabel(va.arm, va.lane),
                outLane : outboundLabel(va.exitArm, va.exitLane),
                progress: (va.turnPos * 100).toFixed(1) + '%',
                speed   : va.turnSpeed.toFixed(2) + ' m/s',
                posX    : this._posCache.hget(ai, 'x').toFixed(1),
                posY    : this._posCache.hget(ai, 'y').toFixed(1),
              },
              {
                vehicle : `#${vb.num} (id ${vb.id})`,
                type    : vb.vt.label,
                from    : vb.arm.toUpperCase(),
                to      : vb.exitArm.toUpperCase(),
                turn    : vb.turn,
                inLane  : inboundLabel(vb.arm, vb.lane),
                outLane : outboundLabel(vb.exitArm, vb.exitLane),
                progress: (vb.turnPos * 100).toFixed(1) + '%',
                speed   : vb.turnSpeed.toFixed(2) + ' m/s',
                posX    : this._posCache.hget(aj, 'x').toFixed(1),
                posY    : this._posCache.hget(aj, 'y').toFixed(1),
              },
            ]);
            console.groupEnd();
            break outer;
          }
        }
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

  // posPx is measured from the stop-line to the vehicle's FRONT face.
  // We render at the vehicle CENTRE, so we add halfLen to push the centre back.
  // halfLen is not known here (it varies per vehicle), so the caller passes
  // posPx already = v.pos * M2PX (distance front-to-stop-line);
  // armPos adds nothing extra — the shift is applied at the call site by
  // passing (v.pos + v.vt.len/2) * M2PX as posPx.
  switch (arm) {
    case 'n':
      return { x: cx + inOffset, y: cy - roadPx - posPx, angle: 0 };
    case 's':
      return { x: cx - inOffset, y: cy + roadPx + posPx, angle: Math.PI };
    case 'e':
      return { x: cx + roadPx + posPx, y: cy + inOffset, angle: Math.PI / 2 };
    case 'w':
      return { x: cx - roadPx - posPx, y: cy - inOffset, angle: -Math.PI / 2 };
  }
}

/**
 * Smooth turn: cubic Bezier from approach stop-line → exit stop-line.
 * Control points are tangent extensions from entry and exit directions,
 * giving a tight arc for right turns, a wide arc for left turns, and a
 * straight line for straight-through — the shortest natural path in each case.
 * t ∈ [0,1]. Returns {x, y, angle}.
 */
function turnBezier(v, t, cx, cy, roadPx) {
  const laneWPx   = roadPx / LANES;
  const inOffset  = (v.lane    + 0.5) * laneWPx;
  const outOffset = (v.exitLane + 0.5) * laneWPx;

  // p0 = entry stop-line point, p2 = exit stop-line point
  let p0, p2;
  switch (v.arm) {
    case 'n': p0 = { x: cx + inOffset,  y: cy - roadPx }; break;
    case 's': p0 = { x: cx - inOffset,  y: cy + roadPx }; break;
    case 'e': p0 = { x: cx + roadPx,    y: cy + inOffset }; break;
    case 'w': p0 = { x: cx - roadPx,    y: cy - inOffset }; break;
  }
  switch (v.exitArm) {
    case 'n': p2 = { x: cx - outOffset, y: cy - roadPx  }; break;
    case 's': p2 = { x: cx + outOffset, y: cy + roadPx  }; break;
    case 'e': p2 = { x: cx + roadPx,    y: cy - outOffset }; break;
    case 'w': p2 = { x: cx - roadPx,    y: cy + outOffset }; break;
  }

  // Travel direction vectors:  ENTRY = into the intersection, EXIT = out of the exit arm.
  // These match the armPos / armPosOut angle conventions exactly.
  const ENTRY = { n: [0,1], s: [0,-1], e: [-1,0], w: [1,0]  };
  const EXIT  = { n: [0,-1], s: [0,1], e: [1,0],  w: [-1,0] };

  // Control distance scales with chord length; 0.45 gives near-circular arcs for 90° turns
  // and reduces to a straight line when p0→p2 are collinear with the tangents.
  const chord = Math.hypot(p2.x - p0.x, p2.y - p0.y);
  const d = chord * 0.45;

  const [ex, ey] = ENTRY[v.arm];
  const [fx, fy] = EXIT[v.exitArm];
  const c1 = { x: p0.x + d * ex, y: p0.y + d * ey };  // extend entry tangent
  const c2 = { x: p2.x - d * fx, y: p2.y - d * fy };  // extend exit tangent back

  // Cubic Bezier position
  const mt = 1 - t;
  const bx = mt*mt*mt*p0.x + 3*mt*mt*t*c1.x + 3*mt*t*t*c2.x + t*t*t*p2.x;
  const by = mt*mt*mt*p0.y + 3*mt*mt*t*c1.y + 3*mt*t*t*c2.y + t*t*t*p2.y;

  // Cubic Bezier tangent
  const tx_ = 3*mt*mt*(c1.x-p0.x) + 6*mt*t*(c2.x-c1.x) + 3*t*t*(p2.x-c2.x);
  const ty_ = 3*mt*mt*(c1.y-p0.y) + 6*mt*t*(c2.y-c1.y) + 3*t*t*(p2.y-c2.y);

  // angle = atan2(-tx_, ty_) matches the armPos/armPosOut rotation convention,
  // so vehicle orientation is continuous at the stop-line boundaries.
  const angle = Math.atan2(-tx_, ty_);

  return { x: bx, y: by, angle };
}

// Arrow characters per turn direction.
// These are rendered in the vehicle's ROTATED local frame:
//   ctx.rotate(θ) maps local +y → travel direction.
//   Text "up" sweeps clockwise by θ, so ↑ appears BACKWARD.
//   ↓ → local +y → forward (travel direction)      ✓ straight
//   → → local +x → driver's LEFT  (for all arms)   ✓ left turn
//   ← → local −x → driver's RIGHT (for all arms)   ✓ right turn
const TURN_ARROW = { straight: '↓', left: '→', right: '←' };

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

    // Stop lines — white bar across each inbound half-road, flush with intersection box
    ctx.strokeStyle = '#ffffffcc';
    ctx.lineWidth   = 2;
    // N arm: horizontal line at cy - roadPx, spanning inbound (right) half
    ctx.beginPath(); ctx.moveTo(cx,       cy - roadPx); ctx.lineTo(cx + roadPx, cy - roadPx); ctx.stroke();
    // S arm: horizontal line at cy + roadPx, spanning inbound (left) half
    ctx.beginPath(); ctx.moveTo(cx - roadPx, cy + roadPx); ctx.lineTo(cx,       cy + roadPx); ctx.stroke();
    // E arm: vertical line at cx + roadPx, spanning inbound (bottom) half
    ctx.beginPath(); ctx.moveTo(cx + roadPx, cy);       ctx.lineTo(cx + roadPx, cy + roadPx); ctx.stroke();
    // W arm: vertical line at cx - roadPx, spanning inbound (top) half
    ctx.beginPath(); ctx.moveTo(cx - roadPx, cy - roadPx); ctx.lineTo(cx - roadPx, cy);       ctx.stroke();
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

  _drawSignals(cx, cy, roadPx, drawPhase) {
    const { ctx } = this;
    const GREEN  = '#52c87a';
    const YELLOW = '#e8c84a';
    const RED    = '#e05252';

    const armColor = (arm) => {
      if (drawPhase.arm === arm) return drawPhase.state === 'green' ? GREEN : YELLOW;
      return RED;
    };

    const drawLight = (x, y, color) => {
      const r = 7;
      ctx.fillStyle = color + '30';
      ctx.beginPath(); ctx.arc(x, y, r + 6, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = color + '70';
      ctx.beginPath(); ctx.arc(x, y, r + 2, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = color;
      ctx.beginPath(); ctx.arc(x, y, r,     0, Math.PI * 2); ctx.fill();
    };

    // One light per arm, at the near corner of the intersection box
    const o = roadPx + 14;
    drawLight(cx,     cy - o, armColor('n'));  // N: top-centre
    drawLight(cx,     cy + o, armColor('s'));  // S: bottom-centre
    drawLight(cx + o, cy,     armColor('e'));  // E: right-centre
    drawLight(cx - o, cy,     armColor('w'));  // W: left-centre
  }

  _drawVehicles(cx, cy, sim, roadPx, roadLenPx) {
    const { ctx } = this;

    // ── Approaching vehicles ───────────────────────────────────
    for (const arm of ['n', 's', 'e', 'w']) {
      for (const v of sim.approaching[arm]) {
        const posPx = v.pos * M2PX;
        if (posPx > roadLenPx + 20) continue;  // not yet on screen

        // pos = front-face distance to stop-line; render centre = front + halfLen
        const centrePx = posPx + (v.vt.len / 2) * M2PX;
        const { x, y, angle } = armPos(arm, v.lane, centrePx, cx, cy, roadPx);
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
        // pos = front-face distance from exit stop-line; render at centre
        const centrePx = posPx + (v.vt.len / 2) * M2PX;
        const { x, y, angle } = armPosOut(arm, v.lane, centrePx, cx, cy, roadPx);
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
    const phase    = ['N ▶', 'N ●', 'E ▶', 'E ●', 'S ▶', 'S ●', 'W ▶', 'W ●'][sim.phase];
    const lines = [
      `Signal: ${phase}`,
      `Active: ${totalVeh}  Throughput: ${sim.throughput}`,
      `Avg wait: ${sim.avgWaitSec().toFixed(1)}s`,
    ];
    ctx.font      = '12px "Segoe UI", system-ui, sans-serif';
    ctx.fillStyle = '#aaa';
    let y = H - 14 - lines.length * 16;
    for (const line of lines) { ctx.fillText(line, 14, y); y += 16; }

    if (sim.collisionPair) {
      const [a, b] = sim.collisionPair;
      ctx.font      = 'bold 15px "Segoe UI", system-ui, sans-serif';
      ctx.fillStyle = '#ff4444';
      ctx.fillText(`⚠ COLLISION  #${a.num} ↔ #${b.num}  — simulation paused`, W / 2, H / 2 + 30);
      ctx.font      = '12px "Segoe UI", system-ui, sans-serif';
      ctx.fillStyle = '#ffaaaa';
      ctx.fillText('Reload to restart', W / 2, H / 2 + 52);
    }
  }
}
