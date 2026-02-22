//! wasm-bindgen adapter for mini-redis core command execution.

use wasm_bindgen::prelude::*;

use crate::mini_redis::core::{CoreCommand, CoreExecutor};

#[wasm_bindgen]
pub struct WasmMiniRedis {
    core: CoreExecutor,
}

#[wasm_bindgen]
impl WasmMiniRedis {
    #[wasm_bindgen(constructor)]
    pub fn new(databases: u16) -> Self {
        Self {
            core: CoreExecutor::new(databases as usize),
        }
    }

    #[wasm_bindgen]
    pub fn exec(&mut self, command_json: JsValue) -> Result<JsValue, JsValue> {
        let cmd: CoreCommand = serde_wasm_bindgen::from_value(command_json)
            .map_err(|err| JsValue::from_str(&format!("invalid command payload: {err}")))?;
        let response = self.core.execute(&cmd);
        serde_wasm_bindgen::to_value(&response)
            .map_err(|err| JsValue::from_str(&format!("response serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn exec_batch(&mut self, commands_json: JsValue) -> Result<JsValue, JsValue> {
        let commands: Vec<CoreCommand> = serde_wasm_bindgen::from_value(commands_json)
            .map_err(|err| JsValue::from_str(&format!("invalid batch payload: {err}")))?;
        let response = self.core.execute_batch(&commands);
        serde_wasm_bindgen::to_value(&response)
            .map_err(|err| JsValue::from_str(&format!("batch serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn metrics_snapshot(&self) -> Result<JsValue, JsValue> {
        let snapshot = self.core.metrics_snapshot();
        serde_wasm_bindgen::to_value(&snapshot)
            .map_err(|err| JsValue::from_str(&format!("metrics serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn reset(&mut self) {
        self.core.reset();
    }

    #[wasm_bindgen]
    pub fn set_queue_depth(&mut self, depth: u32) {
        self.core.set_queue_depth(depth);
    }
}
