declare module "./wasm/muon_js.js" {
  export default function init(moduleOrPath?: unknown): Promise<unknown>;
  export class WasmMuonCache {
    constructor(databases: number);
    exec(commandJson: unknown): unknown;
    exec_batch(commandsJson: unknown): unknown;
    metrics_snapshot(): unknown;
    reset(): unknown;
    set_queue_depth(depth: number): unknown;
  }
}
