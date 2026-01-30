//! Bytecode VM scaffolding.
#![allow(dead_code)]

use crate::bytecode::BytecodeModule;
use crate::types::{JSObjectClassEnum, JSValue};
use crate::{api::js_throw_error, JSContextImpl};

pub struct VM;

impl VM {
    pub fn new() -> Self {
        Self
    }

    pub fn run_module(&mut self, ctx: &mut JSContextImpl, _module: &BytecodeModule) -> JSValue {
        js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode VM not implemented")
    }
}
