//! wasm-bindgen adapter for muoncache core command execution.

use std::cell::Cell;

use wasm_bindgen::prelude::*;

use crate::muon_cache::core::{CoreCommand, CoreExecutor};

#[wasm_bindgen]
pub struct WasmMuonCache {
    core: Cell<Option<CoreExecutor>>,
}

#[wasm_bindgen]
impl WasmMuonCache {
    #[wasm_bindgen(constructor)]
    pub fn new(databases: u16) -> Self {
        Self {
            core: Cell::new(Some(CoreExecutor::new(databases as usize))),
        }
    }

    #[wasm_bindgen]
    pub fn exec(&self, command_json: JsValue) -> Result<JsValue, JsValue> {
        let cmd: CoreCommand = serde_wasm_bindgen::from_value(command_json)
            .map_err(|err| JsValue::from_str(&format!("invalid command payload: {err}")))?;
        let response = self.with_core_mut(|core| core.execute(&cmd))?;
        serde_wasm_bindgen::to_value(&response)
            .map_err(|err| JsValue::from_str(&format!("response serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn exec_batch(&self, commands_json: JsValue) -> Result<JsValue, JsValue> {
        let commands: Vec<CoreCommand> = serde_wasm_bindgen::from_value(commands_json)
            .map_err(|err| JsValue::from_str(&format!("invalid batch payload: {err}")))?;
        let response = self.with_core_mut(|core| core.execute_batch(&commands))?;
        serde_wasm_bindgen::to_value(&response)
            .map_err(|err| JsValue::from_str(&format!("batch serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn metrics_snapshot(&self) -> Result<JsValue, JsValue> {
        let snapshot = self.with_core(|core| core.metrics_snapshot())?;
        serde_wasm_bindgen::to_value(&snapshot)
            .map_err(|err| JsValue::from_str(&format!("metrics serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn reset(&self) -> Result<(), JsValue> {
        self.with_core_mut(|core| core.reset())
    }

    #[wasm_bindgen]
    pub fn set_queue_depth(&self, depth: u32) -> Result<(), JsValue> {
        self.with_core_mut(|core| core.set_queue_depth(depth))
    }
}

impl WasmMuonCache {
    /// Take the core out, run `f` with mutable access, then put it back.
    /// If `f` panics (becomes a JS exception under panic=abort), the core is
    /// lost but the runtime won't falsely report "busy" on subsequent calls.
    fn with_core_mut<T>(&self, f: impl FnOnce(&mut CoreExecutor) -> T) -> Result<T, JsValue> {
        let mut core = self
            .core
            .take()
            .ok_or_else(|| JsValue::from_str("muoncache runtime not available"))?;
        let result = f(&mut core);
        self.core.set(Some(core));
        Ok(result)
    }

    fn with_core<T>(&self, f: impl FnOnce(&CoreExecutor) -> T) -> Result<T, JsValue> {
        let core = self
            .core
            .take()
            .ok_or_else(|| JsValue::from_str("muoncache runtime not available"))?;
        let result = f(&core);
        self.core.set(Some(core));
        Ok(result)
    }
}
