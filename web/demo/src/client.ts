import type {
  MiniRedisCommand,
  MiniRedisMetrics,
  WorkerPush,
  WorkerRequest,
  WorkerResponse
} from "./types";

type Pending = {
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
};

export class MiniRedisClient {
  private readonly worker: Worker;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly metricHandlers = new Set<(metrics: MiniRedisMetrics) => void>();

  constructor() {
    this.worker = new Worker(new URL("./miniRedis.worker.ts", import.meta.url), {
      type: "module"
    });

    this.worker.addEventListener("message", (event: MessageEvent<WorkerResponse | WorkerPush>) => {
      const payload = event.data;
      if ((payload as WorkerPush).kind === "metrics_push") {
        const push = payload as WorkerPush;
        for (const handler of this.metricHandlers) {
          handler(push.data);
        }
        return;
      }

      const response = payload as WorkerResponse;
      const pending = this.pending.get(response.id);
      if (!pending) {
        return;
      }
      this.pending.delete(response.id);
      if (response.ok) {
        pending.resolve(response.data);
      } else {
        pending.reject(new Error(response.error));
      }
    });

    this.worker.addEventListener("error", (event) => {
      const err = new Error(event.message || "worker error");
      for (const pending of this.pending.values()) {
        pending.reject(err);
      }
      this.pending.clear();
    });
  }

  exec(command: MiniRedisCommand): Promise<unknown> {
    return this.rpc({
      id: this.next(),
      kind: "exec",
      command
    });
  }

  batch(commands: MiniRedisCommand[]): Promise<unknown> {
    return this.rpc({
      id: this.next(),
      kind: "batch",
      commands
    });
  }

  metrics(): Promise<MiniRedisMetrics> {
    return this.rpc({
      id: this.next(),
      kind: "metrics"
    }) as Promise<MiniRedisMetrics>;
  }

  reset(): Promise<void> {
    return this.rpc({
      id: this.next(),
      kind: "reset"
    }).then(() => undefined);
  }

  subscribeMetrics(handler: (metrics: MiniRedisMetrics) => void): () => void {
    this.metricHandlers.add(handler);
    return () => {
      this.metricHandlers.delete(handler);
    };
  }

  terminate(): void {
    this.worker.terminate();
    for (const pending of this.pending.values()) {
      pending.reject(new Error("client terminated"));
    }
    this.pending.clear();
    this.metricHandlers.clear();
  }

  private rpc(request: WorkerRequest): Promise<unknown> {
    return new Promise((resolve, reject) => {
      this.pending.set(request.id, { resolve, reject });
      this.worker.postMessage(request);
    });
  }

  private next(): number {
    const id = this.nextId;
    this.nextId += 1;
    return id;
  }
}
