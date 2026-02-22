/// <reference lib="webworker" />

import init, { WasmMiniRedis } from "./wasm/muon_js.js";

import type { WorkerRequest, WorkerResponse } from "./types";

const METRICS_PUSH_INTERVAL_MS = 100;

let runtimePromise: Promise<WasmMiniRedis> | null = null;
let pendingQueueDepth = 0;

function getRuntime(): Promise<WasmMiniRedis> {
  if (!runtimePromise) {
    runtimePromise = init().then(() => new WasmMiniRedis(16));
  }
  return runtimePromise;
}

function errorMessage(err: unknown): string {
  if (err instanceof Error) {
    return err.message;
  }
  if (typeof err === "string") {
    return err;
  }
  return "unknown worker error";
}

async function pushMetrics(): Promise<void> {
  if (!runtimePromise) {
    return;
  }
  try {
    const runtime = await runtimePromise;
    runtime.set_queue_depth(pendingQueueDepth);
    const data = await runtime.metrics_snapshot();
    self.postMessage({ kind: "metrics_push", data });
  } catch {
    // keep interval alive; request/response path reports actionable errors
  }
}

setInterval(() => {
  void pushMetrics();
}, METRICS_PUSH_INTERVAL_MS);

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  void (async () => {
    const request = event.data;
    pendingQueueDepth += 1;

    const send = (payload: WorkerResponse): void => {
      self.postMessage(payload);
    };

    try {
      const runtime = await getRuntime();
      runtime.set_queue_depth(pendingQueueDepth);

      switch (request.kind) {
        case "exec": {
          const data = await runtime.exec(request.command as never);
          send({ id: request.id, ok: true, data });
          break;
        }
        case "batch": {
          const data = await runtime.exec_batch(request.commands as never);
          send({ id: request.id, ok: true, data });
          break;
        }
        case "metrics": {
          const data = await runtime.metrics_snapshot();
          send({ id: request.id, ok: true, data });
          break;
        }
        case "reset": {
          runtime.reset();
          send({ id: request.id, ok: true, data: { ok: true } });
          break;
        }
      }
    } catch (err) {
      send({ id: request.id, ok: false, error: errorMessage(err) });
    } finally {
      pendingQueueDepth = Math.max(0, pendingQueueDepth - 1);
      if (runtimePromise) {
        const runtime = await runtimePromise;
        runtime.set_queue_depth(pendingQueueDepth);
      }
    }
  })();
};

export {};
