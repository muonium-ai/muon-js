//! wasm-bindgen adapter for mini-redis core command execution.

use std::cell::{Ref, RefCell, RefMut};

use wasm_bindgen::prelude::*;

use crate::mini_redis::core::{CoreCommand, CoreExecutor};

#[wasm_bindgen]
pub struct WasmMiniRedis {
    core: RefCell<CoreExecutor>,
}

#[wasm_bindgen]
impl WasmMiniRedis {
    #[wasm_bindgen(constructor)]
    pub fn new(databases: u16) -> Self {
        Self {
            core: RefCell::new(CoreExecutor::new(databases as usize)),
        }
    }

    #[wasm_bindgen]
    pub fn exec(&self, command_json: JsValue) -> Result<JsValue, JsValue> {
        let cmd: CoreCommand = serde_wasm_bindgen::from_value(command_json)
            .map_err(|err| JsValue::from_str(&format!("invalid command payload: {err}")))?;
        let response = {
            let mut core = self.core_mut()?;
            core.execute(&cmd)
        };
        serde_wasm_bindgen::to_value(&response)
            .map_err(|err| JsValue::from_str(&format!("response serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn exec_batch(&self, commands_json: JsValue) -> Result<JsValue, JsValue> {
        let commands: Vec<CoreCommand> = serde_wasm_bindgen::from_value(commands_json)
            .map_err(|err| JsValue::from_str(&format!("invalid batch payload: {err}")))?;
        let response = {
            let mut core = self.core_mut()?;
            core.execute_batch(&commands)
        };
        serde_wasm_bindgen::to_value(&response)
            .map_err(|err| JsValue::from_str(&format!("batch serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn metrics_snapshot(&self) -> Result<JsValue, JsValue> {
        let snapshot = {
            let core = self.core_ref()?;
            core.metrics_snapshot()
        };
        serde_wasm_bindgen::to_value(&snapshot)
            .map_err(|err| JsValue::from_str(&format!("metrics serialization error: {err}")))
    }

    #[wasm_bindgen]
    pub fn reset(&self) -> Result<(), JsValue> {
        let mut core = self.core_mut()?;
        core.reset();
        Ok(())
    }

    #[wasm_bindgen]
    pub fn set_queue_depth(&self, depth: u32) -> Result<(), JsValue> {
        let mut core = self.core_mut()?;
        core.set_queue_depth(depth);
        Ok(())
    }
}

impl WasmMiniRedis {
    fn core_mut(&self) -> Result<RefMut<'_, CoreExecutor>, JsValue> {
        self.core
            .try_borrow_mut()
            .map_err(|_| JsValue::from_str("mini-redis runtime busy"))
    }

    fn core_ref(&self) -> Result<Ref<'_, CoreExecutor>, JsValue> {
        self.core
            .try_borrow()
            .map_err(|_| JsValue::from_str("mini-redis runtime busy"))
    }
}
