//! Bytecode VM — executes compiled bytecode modules.
#![allow(dead_code)]

use crate::bytecode::{BytecodeModule, OpCode};
use crate::types::{JSObjectClassEnum, JSValue};
use crate::{api::{js_throw_error, js_to_number, js_to_string, js_value_to_atom,
                   js_get_property_str, js_get_property_uint32, js_new_string, js_new_string_len,
                   js_is_string, js_is_function, js_is_number}, JSContextImpl};

pub struct VM;

macro_rules! pop {
    ($stack:expr, $ctx:expr) => {
        match $stack.pop() {
            Some(v) => v,
            None => return js_throw_error($ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow"),
        }
    };
}

impl VM {
    pub fn new() -> Self {
        Self
    }

    pub fn run_module(&mut self, ctx: &mut JSContextImpl, module: &BytecodeModule) -> JSValue {
        self.run_module_with_locals(ctx, module, &[])
    }

    pub fn run_module_with_locals(
        &mut self,
        ctx: &mut JSContextImpl,
        module: &BytecodeModule,
        param_values: &[JSValue],
    ) -> JSValue {
        if module.functions.is_empty() {
            return JSValue::UNDEFINED;
        }
        let func = &module.functions[module.main];
        let num_locals = func.locals.len();
        let mut locals: Vec<JSValue> = vec![JSValue::UNDEFINED; num_locals];
        // Fill in parameter values
        for (i, &v) in param_values.iter().enumerate() {
            if i < num_locals {
                locals[i] = v;
            }
        }
        let mut stack: Vec<JSValue> = Vec::with_capacity(32);
        let global = crate::api::js_get_global_object(ctx);
        let mut pc = 0usize;
        let code = &func.code;
        let constants = &func.constants;
        let code_len = code.len();

        while pc < code_len {
            let ins = code[pc];
            match ins.op {
                OpCode::Nop => {}

                OpCode::Const => {
                    let idx = ins.a as usize;
                    if let Some(val) = constants.get(idx) {
                        stack.push(*val);
                    } else {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range");
                    }
                }

                OpCode::LoadLocal => {
                    let slot = ins.a as usize;
                    let val = if slot < locals.len() { locals[slot] } else { JSValue::UNDEFINED };
                    stack.push(val);
                }

                OpCode::StoreLocal => {
                    let slot = ins.a as usize;
                    let val = pop!(stack, ctx);
                    if slot < locals.len() {
                        locals[slot] = val;
                    }
                    stack.push(val);
                }

                OpCode::IncLocal => {
                    let slot = ins.a as usize;
                    let amount = ins.b as i32;
                    if slot < locals.len() {
                        let old = locals[slot];
                        if old.is_int() {
                            if let Some(n) = old.int32() {
                                locals[slot] = JSValue::from_int32(n.wrapping_add(amount));
                            }
                        } else {
                            let n = match js_to_number(ctx, old) {
                                Ok(n) => n,
                                Err(e) => return e,
                            };
                            locals[slot] = crate::helpers::number_to_value(ctx, n + amount as f64);
                        }
                    }
                }

                OpCode::LoadGlobal => {
                    let idx = ins.a as usize;
                    let name_val = match constants.get(idx) {
                        Some(v) => *v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range"),
                    };
                    let atom = js_value_to_atom(ctx, name_val);
                    if atom <= 0 {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode global name not string");
                    }
                    let val = ctx
                        .get_property_atom_id(global, atom as u32)
                        .unwrap_or(JSValue::UNDEFINED);
                    stack.push(val);
                }

                OpCode::StoreGlobal => {
                    let idx = ins.a as usize;
                    let name_val = match constants.get(idx) {
                        Some(v) => *v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range"),
                    };
                    let atom = js_value_to_atom(ctx, name_val);
                    if atom <= 0 {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode global name not string");
                    }
                    let value = pop!(stack, ctx);
                    if !ctx.set_property_atom_id(global, atom as u32, value) {
                        return js_throw_error(ctx, JSObjectClassEnum::TypeError, "property set failed");
                    }
                    stack.push(value);
                }

                OpCode::Add => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    // String concatenation if either side is a string
                    let a_is_str = ctx.string_bytes(a).is_some();
                    let b_is_str = ctx.string_bytes(b).is_some();
                    if a_is_str || b_is_str {
                        let ls = js_to_string(ctx, a);
                        let rs = js_to_string(ctx, b);
                        let lb = ctx.string_bytes(ls).unwrap_or(b"");
                        let rb = ctx.string_bytes(rs).unwrap_or(b"");
                        let mut out = Vec::with_capacity(lb.len() + rb.len());
                        out.extend_from_slice(lb);
                        out.extend_from_slice(rb);
                        stack.push(js_new_string_len(ctx, &out));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(crate::helpers::number_to_value(ctx, an + bn));
                    }
                }

                OpCode::Sub => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, an - bn));
                }

                OpCode::Mul => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, an * bn));
                }

                OpCode::Div => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, an / bn));
                }

                OpCode::Mod => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, an % bn));
                }

                OpCode::Neg => {
                    let v = pop!(stack, ctx);
                    let n = match js_to_number(ctx, v) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, -n));
                }

                OpCode::Eq => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let result = vm_loose_eq(ctx, a, b);
                    stack.push(JSValue::new_bool(result));
                }

                OpCode::Neq => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let result = vm_loose_eq(ctx, a, b);
                    stack.push(JSValue::new_bool(!result));
                }

                OpCode::StrictEq => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let result = vm_strict_eq(ctx, a, b);
                    stack.push(JSValue::new_bool(result));
                }

                OpCode::StrictNeq => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let result = vm_strict_eq(ctx, a, b);
                    stack.push(JSValue::new_bool(!result));
                }

                OpCode::Lt => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
                        stack.push(JSValue::new_bool(la < lb));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(JSValue::new_bool(an < bn));
                    }
                }

                OpCode::Gt => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
                        stack.push(JSValue::new_bool(la > lb));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(JSValue::new_bool(an > bn));
                    }
                }

                OpCode::Le => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
                        stack.push(JSValue::new_bool(la <= lb));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(JSValue::new_bool(an <= bn));
                    }
                }

                OpCode::Ge => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
                        stack.push(JSValue::new_bool(la >= lb));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(JSValue::new_bool(an >= bn));
                    }
                }

                OpCode::Not => {
                    let v = pop!(stack, ctx);
                    let truthy = crate::evals::is_truthy(ctx, v);
                    stack.push(JSValue::new_bool(!truthy));
                }

                OpCode::And | OpCode::Or => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let a_truthy = crate::evals::is_truthy(ctx, a);
                    let result = if ins.op == OpCode::And {
                        if a_truthy { b } else { a }
                    } else if a_truthy {
                        a
                    } else {
                        b
                    };
                    stack.push(result);
                }

                OpCode::Drop | OpCode::Pop => {
                    let _ = stack.pop();
                }

                OpCode::Dup => {
                    let v = pop!(stack, ctx);
                    stack.push(v);
                    stack.push(v);
                }

                OpCode::Jump => {
                    pc = ins.a as usize;
                    continue;
                }

                OpCode::JumpIfFalse => {
                    let v = pop!(stack, ctx);
                    if !crate::evals::is_truthy(ctx, v) {
                        pc = ins.a as usize;
                        continue;
                    }
                }

                OpCode::JumpIfTrue => {
                    let v = pop!(stack, ctx);
                    if crate::evals::is_truthy(ctx, v) {
                        pc = ins.a as usize;
                        continue;
                    }
                }

                OpCode::Call => {
                    let argc = ins.a as usize;
                    // Stack: [... func, arg0, arg1, ..., argN-1]
                    if stack.len() < argc + 1 {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow in call");
                    }
                    let args_start = stack.len() - argc;
                    let func_idx = args_start - 1;
                    let func_val = stack[func_idx];
                    let args: Vec<JSValue> = stack[args_start..].to_vec();
                    stack.truncate(func_idx);

                    if let Some(result) = crate::api::call_function_value(ctx, func_val, global, &args) {
                        stack.push(result);
                    } else {
                        return js_throw_error(ctx, JSObjectClassEnum::TypeError, "not a function");
                    }
                }

                OpCode::GetProp => {
                    let key = pop!(stack, ctx);
                    let obj = pop!(stack, ctx);
                    if let Some(key_bytes) = ctx.string_bytes(key) {
                        let key_str = core::str::from_utf8(key_bytes).unwrap_or("").to_string();
                        let val = js_get_property_str(ctx, obj, &key_str);
                        stack.push(val);
                    } else if key.is_int() {
                        let idx = key.int32().unwrap_or(0) as u32;
                        let val = js_get_property_uint32(ctx, obj, idx);
                        stack.push(val);
                    } else {
                        let key_s = js_to_string(ctx, key);
                        let key_bytes = ctx.string_bytes(key_s).unwrap_or(b"").to_vec();
                        let key_str = core::str::from_utf8(&key_bytes).unwrap_or("").to_string();
                        let val = js_get_property_str(ctx, obj, &key_str);
                        stack.push(val);
                    }
                }

                OpCode::SetProp => {
                    let value = pop!(stack, ctx);
                    let key = pop!(stack, ctx);
                    let obj = pop!(stack, ctx);
                    if let Some(key_bytes) = ctx.string_bytes(key) {
                        let key_str = core::str::from_utf8(key_bytes).unwrap_or("").to_string();
                        crate::api::js_set_property_str(ctx, obj, &key_str, value);
                    }
                    stack.push(value);
                }

                OpCode::GetElem => {
                    let idx = pop!(stack, ctx);
                    let obj = pop!(stack, ctx);
                    if idx.is_int() {
                        let i = idx.int32().unwrap_or(0) as u32;
                        let val = js_get_property_uint32(ctx, obj, i);
                        stack.push(val);
                    } else if let Some(idx_bytes) = ctx.string_bytes(idx) {
                        let idx_str = core::str::from_utf8(idx_bytes).unwrap_or("").to_string();
                        let val = js_get_property_str(ctx, obj, &idx_str);
                        stack.push(val);
                    } else {
                        let n = match js_to_number(ctx, idx) { Ok(n) => n, Err(e) => return e };
                        let i = n as u32;
                        let val = js_get_property_uint32(ctx, obj, i);
                        stack.push(val);
                    }
                }

                OpCode::Concat => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let ls = js_to_string(ctx, a);
                    let rs = js_to_string(ctx, b);
                    let lb = ctx.string_bytes(ls).unwrap_or(b"");
                    let rb = ctx.string_bytes(rs).unwrap_or(b"");
                    let mut out = Vec::with_capacity(lb.len() + rb.len());
                    out.extend_from_slice(lb);
                    out.extend_from_slice(rb);
                    stack.push(js_new_string_len(ctx, &out));
                }

                OpCode::ToNumber => {
                    let v = pop!(stack, ctx);
                    let n = match js_to_number(ctx, v) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, n));
                }

                OpCode::Typeof => {
                    let v = pop!(stack, ctx);
                    let type_str = if v.is_bool() {
                        "boolean"
                    } else if js_is_number(ctx, v) != 0 {
                        "number"
                    } else if js_is_string(ctx, v) != 0 {
                        "string"
                    } else if v.is_undefined() {
                        "undefined"
                    } else if v.is_null() {
                        "object"
                    } else if js_is_function(ctx, v) != 0 {
                        "function"
                    } else if v.is_ptr() {
                        "object"
                    } else {
                        "undefined"
                    };
                    stack.push(js_new_string(ctx, type_str));
                }

                OpCode::BitAnd => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) & (bn as i32);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::BitOr => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) | (bn as i32);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::BitXor => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) ^ (bn as i32);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::Shl => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) << ((bn as u32) & 31);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::Shr => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) >> ((bn as u32) & 31);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::UShr => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = ((an as u32) >> ((bn as u32) & 31)) as i32;
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::BitNot => {
                    let v = pop!(stack, ctx);
                    let n = match js_to_number(ctx, v) { Ok(n) => n, Err(e) => return e };
                    let result = !(n as i32);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::Return => {
                    return stack.pop().unwrap_or(JSValue::UNDEFINED);
                }
            }
            pc += 1;
        }
        JSValue::UNDEFINED
    }
}

/// Loose equality (==) for the VM.
fn vm_loose_eq(ctx: &mut JSContextImpl, a: JSValue, b: JSValue) -> bool {
    if (a.is_null() && b.is_undefined()) || (a.is_undefined() && b.is_null()) {
        return true;
    }
    if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
        return la == lb;
    }
    if a.0 == b.0 {
        return true;
    }
    let ln = js_to_number(ctx, a).ok();
    let rn = js_to_number(ctx, b).ok();
    if let (Some(l), Some(r)) = (ln, rn) {
        l == r
    } else {
        false
    }
}

/// Strict equality (===) for the VM.
fn vm_strict_eq(ctx: &mut JSContextImpl, a: JSValue, b: JSValue) -> bool {
    if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
        return la == lb;
    }
    if a.0 == b.0 {
        return true;
    }
    if ctx.object_class_id(a).is_some() || ctx.object_class_id(b).is_some() {
        return false;
    }
    let ln = js_to_number(ctx, a).ok();
    let rn = js_to_number(ctx, b).ok();
    if let (Some(l), Some(r)) = (ln, rn) {
        l == r
    } else {
        false
    }
}
