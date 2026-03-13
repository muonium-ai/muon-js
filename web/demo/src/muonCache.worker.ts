/// <reference lib="webworker" />

import init, { WasmMuonCache } from "./wasm/muon_js.js";

import type { WorkerRequest, WorkerResponse } from "./types";

const METRICS_PUSH_INTERVAL_MS = 100;

let runtimePromise: Promise<WasmMuonCache> | null = null;
let runtimeQueue: Promise<void> = Promise.resolve();
let pendingQueueDepth = 0;

function getRuntime(): Promise<WasmMuonCache> {
  if (!runtimePromise) {
    runtimePromise = init().then(() => new WasmMuonCache(16));
  }
  return runtimePromise;
}

function enqueueRuntime<T>(fn: (runtime: WasmMuonCache) => T | Promise<T>): Promise<T> {
  const task = runtimeQueue.then(async () => {
    const runtime = await getRuntime();
    return fn(runtime);
  });

  runtimeQueue = task.then(
    () => undefined,
    () => undefined
  );

  return task;
}

function errorMessage(err: unknown): string {
  if (err instanceof Error) {
    if (err.stack) {
      return `${err.message}\n${err.stack}`;
    }
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
    const data = (await enqueueRuntime((runtime) => runtime.metrics_snapshot())) as {
      queue_depth?: number;
    };
    if (data && typeof data === "object") {
      data.queue_depth = pendingQueueDepth;
    }
    self.postMessage({ kind: "metrics_push", data });
  } catch {
    // keep interval alive; request/response path reports actionable errors
  }
}

setInterval(() => {
  void pushMetrics();
}, METRICS_PUSH_INTERVAL_MS);

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  const request = event.data;
  pendingQueueDepth += 1;

  const send = (payload: WorkerResponse): void => {
    self.postMessage(payload);
  };

  void enqueueRuntime((runtime) => {
    switch (request.kind) {
      case "exec":
        return runtime.exec(request.command as never);
      case "batch":
        return runtime.exec_batch(request.commands as never);
      case "metrics":
        return runtime.metrics_snapshot();
      case "reset":
        runtime.reset();
        return { ok: true };
    }
  })
    .then((data) => {
      if (request.kind === "metrics" && data && typeof data === "object") {
        (data as { queue_depth?: number }).queue_depth = pendingQueueDepth;
      }
      send({ id: request.id, ok: true, data });
    })
    .catch((err) => {
      send({ id: request.id, ok: false, error: errorMessage(err) });
    })
    .finally(() => {
      pendingQueueDepth = Math.max(0, pendingQueueDepth - 1);
    });
};

export {};
