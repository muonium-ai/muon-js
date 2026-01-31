//! Bytecode VM scaffolding.
#![allow(dead_code)]

use crate::bytecode::{BytecodeModule, OpCode};
use crate::types::{JSObjectClassEnum, JSValue};
use crate::{api::{js_throw_error, js_to_number, js_get_property_str, js_set_property_str}, JSContextImpl};

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
        let global = crate::api::js_get_global_object(ctx);
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
                OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div
                | OpCode::Eq | OpCode::Neq | OpCode::Lt | OpCode::Gt | OpCode::Le | OpCode::Ge => {
                    let b = match stack.pop() {
                        Some(v) => v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow"),
                    };
                    let a = match stack.pop() {
                        Some(v) => v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow"),
                    };
                    match ins.op {
                        OpCode::Eq | OpCode::Neq | OpCode::Lt | OpCode::Gt | OpCode::Le | OpCode::Ge => {
                            let an = match js_to_number(ctx, a) {
                                Ok(n) => n,
                                Err(e) => return e,
                            };
                            let bn = match js_to_number(ctx, b) {
                                Ok(n) => n,
                                Err(e) => return e,
                            };
                            let result = match ins.op {
                                OpCode::Eq => an == bn,
                                OpCode::Neq => an != bn,
                                OpCode::Lt => an < bn,
                                OpCode::Gt => an > bn,
                                OpCode::Le => an <= bn,
                                OpCode::Ge => an >= bn,
                                _ => false,
                            };
                            stack.push(JSValue::new_bool(result));
                        }
                        _ => {
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
                    }
                }
                OpCode::LoadGlobal => {
                    let idx = ins.a as usize;
                    let name_val = match func.constants.get(idx) {
                        Some(v) => *v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range"),
                    };
                    let name = match ctx.string_bytes(name_val) {
                        Some(b) => b,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode global name not string"),
                    };
                    let name = match core::str::from_utf8(name) {
                        Ok(s) => s.to_string(),
                        Err(_) => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode global name invalid"),
                    };
                    let val = js_get_property_str(ctx, global, &name);
                    stack.push(val);
                }
                OpCode::StoreGlobal => {
                    let idx = ins.a as usize;
                    let name_val = match func.constants.get(idx) {
                        Some(v) => *v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range"),
                    };
                    let name = match ctx.string_bytes(name_val) {
                        Some(b) => b,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode global name not string"),
                    };
                    let name = match core::str::from_utf8(name) {
                        Ok(s) => s.to_string(),
                        Err(_) => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode global name invalid"),
                    };
                    let value = match stack.pop() {
                        Some(v) => v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow"),
                    };
                    let _ = js_set_property_str(ctx, global, &name, value);
                    stack.push(value);
                }
                OpCode::Return => {
                    return stack.pop().unwrap_or(JSValue::UNDEFINED);
                }
            }
        }
        JSValue::UNDEFINED
    }
}
