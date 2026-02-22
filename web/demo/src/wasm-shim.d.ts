declare module "./wasm/muon_js.js" {
  export default function init(moduleOrPath?: unknown): Promise<unknown>;
  export class WasmMiniRedis {
    constructor(databases: number);
    exec(commandJson: unknown): unknown;
    exec_batch(commandsJson: unknown): unknown;
    metrics_snapshot(): unknown;
    reset(): void;
    set_queue_depth(depth: number): void;
  }
}
