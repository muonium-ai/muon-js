/* tslint:disable */
/* eslint-disable */

export class WasmMuonCache {
    free(): void;
    [Symbol.dispose](): void;
    exec(command_json: any): any;
    exec_batch(commands_json: any): any;
    js_eval(source: string): any;
    metrics_snapshot(): any;
    constructor(databases: number);
    reset(): void;
    set_queue_depth(depth: number): void;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_wasmmuoncache_free: (a: number, b: number) => void;
    readonly wasmmuoncache_exec: (a: number, b: any) => [number, number, number];
    readonly wasmmuoncache_exec_batch: (a: number, b: any) => [number, number, number];
    readonly wasmmuoncache_js_eval: (a: number, b: number, c: number) => [number, number, number];
    readonly wasmmuoncache_metrics_snapshot: (a: number) => [number, number, number];
    readonly wasmmuoncache_new: (a: number) => number;
    readonly wasmmuoncache_reset: (a: number) => [number, number];
    readonly wasmmuoncache_set_queue_depth: (a: number, b: number) => [number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
