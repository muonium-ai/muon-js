import "./style.css";

import { MiniRedisClient } from "./client";
import { MetricsDashboard } from "./dashboard";
import type { CoreResponse, MiniRedisCommand, MiniRedisMetrics } from "./types";

const client = new MiniRedisClient();

function must<T>(value: T | null, id: string): T {
  if (!value) {
    throw new Error(`Missing expected DOM element: ${id}`);
  }
  return value;
}

const startBtn = must(document.querySelector<HTMLButtonElement>("#startBtn"), "#startBtn");
const stopBtn = must(document.querySelector<HTMLButtonElement>("#stopBtn"), "#stopBtn");
const resetBtn = must(document.querySelector<HTMLButtonElement>("#resetBtn"), "#resetBtn");
const uncapBtn = must(document.querySelector<HTMLButtonElement>("#uncapBtn"), "#uncapBtn");
const statusText = must(document.querySelector<HTMLElement>("#statusText"), "#statusText");
const leaderboardList = must(document.querySelector<HTMLOListElement>("#leaderboardList"), "#leaderboardList");
const gpuCanvas = must(document.querySelector<HTMLCanvasElement>("#gpuCanvas"), "#gpuCanvas");
const gpuWarning = must(document.querySelector<HTMLElement>("#gpuWarning"), "#gpuWarning");

const statEls = {
  opsTotal: must(document.querySelector<HTMLElement>("#opsTotal"), "#opsTotal"),
  opsWindow: must(document.querySelector<HTMLElement>("#opsWindow"), "#opsWindow"),
  batchAvg: must(document.querySelector<HTMLElement>("#batchAvg"), "#batchAvg"),
  latP50: must(document.querySelector<HTMLElement>("#latP50"), "#latP50"),
  latP95: must(document.querySelector<HTMLElement>("#latP95"), "#latP95"),
  latP99: must(document.querySelector<HTMLElement>("#latP99"), "#latP99"),
  queueDepth: must(document.querySelector<HTMLElement>("#queueDepth"), "#queueDepth"),
  errorsTotal: must(document.querySelector<HTMLElement>("#errorsTotal"), "#errorsTotal"),
  renderFps: must(document.querySelector<HTMLElement>("#renderFps"), "#renderFps")
};

const dashboard = new MetricsDashboard(gpuCanvas, gpuWarning);
void dashboard.init();

const PLAYER_COUNT = 1500;
const UPDATES_PER_TICK = 320;
const TICK_MS = 20;

const players = Array.from({ length: PLAYER_COUNT }, (_, i) => `player:${i + 1}`);
const scores = new Float64Array(PLAYER_COUNT);

let runTimer: number | null = null;
let fpsTimer: number | null = null;
let running = false;
let pendingBatches = 0;
let tickCount = 0;

function setStatus(text: string): void {
  statusText.textContent = text;
}

function responseData(response: CoreResponse | undefined): unknown {
  if (!response) {
    return undefined;
  }
  if (!response.ok) {
    return undefined;
  }
  return response.data;
}

function renderLeaderboard(members: string[]): void {
  const top = [...members].reverse().slice(0, 10);
  leaderboardList.innerHTML = "";

  for (const member of top) {
    const index = Number(member.split(":")[1] ?? "0") - 1;
    const score = Number.isInteger(index) && index >= 0 && index < scores.length ? scores[index] : 0;
    const li = document.createElement("li");
    li.textContent = `${member} (${Math.round(score)})`;
    leaderboardList.appendChild(li);
  }
}

function updateStats(metrics: MiniRedisMetrics): void {
  statEls.opsTotal.textContent = metrics.ops_total.toLocaleString();
  statEls.opsWindow.textContent = metrics.ops_window_1s.toLocaleString();
  statEls.batchAvg.textContent = metrics.batch_size_avg.toFixed(1);
  statEls.latP50.textContent = metrics.latency_p50_us.toLocaleString();
  statEls.latP95.textContent = metrics.latency_p95_us.toLocaleString();
  statEls.latP99.textContent = metrics.latency_p99_us.toLocaleString();
  statEls.queueDepth.textContent = metrics.queue_depth.toLocaleString();
  statEls.errorsTotal.textContent = metrics.errors_total.toLocaleString();
}

function buildCommands(includeTopRead: boolean): MiniRedisCommand[] {
  const commands: MiniRedisCommand[] = [];

  for (let i = 0; i < UPDATES_PER_TICK; i += 1) {
    const idx = (Math.random() * PLAYER_COUNT) | 0;
    const delta = 1 + ((Math.random() * 4) | 0);
    const player = players[idx];
    scores[idx] += delta;

    commands.push({
      kind: "hincrby",
      key: `stats:${player}`,
      field: "score",
      delta
    });

    commands.push({
      kind: "zadd",
      key: "leaderboard",
      score: scores[idx],
      member: player
    });
  }

  if (includeTopRead) {
    commands.push({ kind: "zrange", key: "leaderboard", start: -20, stop: -1 });
  }

  return commands;
}

async function runTick(): Promise<void> {
  if (!running) {
    return;
  }
  if (pendingBatches > 3) {
    return;
  }

  const includeTopRead = tickCount % 4 === 0;
  const commands = buildCommands(includeTopRead);
  pendingBatches += 1;

  try {
    const raw = await client.batch(commands);
    const responses = Array.isArray(raw) ? (raw as CoreResponse[]) : [];
    if (includeTopRead) {
      const last = responses[responses.length - 1];
      const data = responseData(last);
      if (Array.isArray(data)) {
        renderLeaderboard(data.filter((item): item is string => typeof item === "string"));
      }
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    setStatus(`error: ${message}`);
  } finally {
    pendingBatches = Math.max(0, pendingBatches - 1);
    tickCount += 1;
  }
}

function startSimulation(): void {
  if (running) {
    return;
  }
  running = true;
  startBtn.disabled = true;
  stopBtn.disabled = false;
  setStatus("running");

  runTimer = window.setInterval(() => {
    void runTick();
  }, TICK_MS);

  fpsTimer = window.setInterval(() => {
    statEls.renderFps.textContent = dashboard.fps().toString();
  }, 500);
}

function stopSimulation(): void {
  running = false;
  startBtn.disabled = false;
  stopBtn.disabled = true;
  setStatus("stopped");
  if (runTimer !== null) {
    clearInterval(runTimer);
    runTimer = null;
  }
  if (fpsTimer !== null) {
    clearInterval(fpsTimer);
    fpsTimer = null;
  }
}

async function resetRuntime(): Promise<void> {
  stopSimulation();
  scores.fill(0);
  tickCount = 0;
  pendingBatches = 0;
  leaderboardList.innerHTML = "";
  await client.reset();
  setStatus("reset");
}

const unsubscribe = client.subscribeMetrics((metrics) => {
  updateStats(metrics);
  dashboard.pushMetrics(metrics);
});

startBtn.addEventListener("click", () => {
  startSimulation();
});

stopBtn.addEventListener("click", () => {
  stopSimulation();
});

resetBtn.addEventListener("click", () => {
  void resetRuntime();
});

let fpsUncapped = false;
uncapBtn.addEventListener("click", () => {
  fpsUncapped = !fpsUncapped;
  dashboard.setUncapped(fpsUncapped);
  uncapBtn.textContent = fpsUncapped ? "Cap FPS" : "Uncap FPS";
});

window.addEventListener("beforeunload", () => {
  unsubscribe();
  stopSimulation();
  dashboard.destroy();
  client.terminate();
});

setStatus("ready");
