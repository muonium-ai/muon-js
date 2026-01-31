//! Bytecode VM scaffolding.
#![allow(dead_code)]

use crate::bytecode::{BytecodeModule, OpCode};
use crate::types::{JSObjectClassEnum, JSValue};
use crate::{api::{js_throw_error, js_to_number}, JSContextImpl};

pub struct VM;

impl VM {
    pub fn new() -> Self {
        Self
    }

    pub fn run_module(&mut self, ctx: &mut JSContextImpl, _module: &BytecodeModule) -> JSValue {
        if _module.functions.is_empty() {
            return JSValue::UNDEFINED;
        }
        let func = &_module.functions[_module.main];
        let mut stack: Vec<JSValue> = Vec::new();
        for ins in &func.code {
            match ins.op {
                OpCode::Nop => {}
                OpCode::Const => {
                    let idx = ins.a as usize;
                    if let Some(val) = func.constants.get(idx) {
                        stack.push(*val);
                    } else {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range");
                    }
                }
                OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div => {
                    let b = match stack.pop() {
                        Some(v) => v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow"),
                    };
                    let a = match stack.pop() {
                        Some(v) => v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow"),
                    };
                    let an = match js_to_number(ctx, a) {
                        Ok(n) => n,
                        Err(e) => return e,
                    };
                    let bn = match js_to_number(ctx, b) {
                        Ok(n) => n,
                        Err(e) => return e,
                    };
                    let out = match ins.op {
                        OpCode::Add => an + bn,
                        OpCode::Sub => an - bn,
                        OpCode::Mul => an * bn,
                        OpCode::Div => an / bn,
                        _ => an,
                    };
                    stack.push(crate::helpers::number_to_value(ctx, out));
                }
                OpCode::Return => {
                    return stack.pop().unwrap_or(JSValue::UNDEFINED);
                }
            }
        }
        JSValue::UNDEFINED
    }
}
